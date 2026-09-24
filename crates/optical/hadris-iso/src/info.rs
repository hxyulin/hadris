use crate::error::Detail;
use crate::namespace::{JolietLevel, Namespace, Namespaces};
use crate::raw::{self, VolumeDescriptor};

/// A directory tree's root: the first block of its data and its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Root {
    pub(crate) extent: u32,
    pub(crate) size: u32,
    /// The little-endian path table: its first block and its length.
    pub(crate) path_table: (u32, u32),
}

/// What opening an image learns from its descriptors and root directory.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Info {
    pub(crate) block_size: u32,
    pub(crate) volume_blocks: u32,
    pub(crate) primary: Root,
    pub(crate) joliet: Option<(Root, JolietLevel)>,
    pub(crate) enhanced: Option<Root>,
    /// The `SP` skip count when the primary tree has Rock Ridge.
    pub(crate) rock_ridge: Option<u8>,
    pub(crate) boot_catalog: Option<u32>,
    pub(crate) descriptors: u32,
}

impl Info {
    pub(crate) fn namespaces(&self) -> Namespaces {
        Namespaces::new(
            self.rock_ridge.is_some(),
            self.joliet.map(|(_, level)| level),
            self.enhanced.is_some(),
        )
    }

    /// The tree `namespace` names, with [`Namespace::Preferred`] resolved.
    pub(crate) fn tree(&self, namespace: Namespace) -> Option<(Namespace, Root)> {
        let namespace = match namespace {
            Namespace::Preferred => self.namespaces().preferred(),
            other => other,
        };
        let root = match namespace {
            Namespace::Primary => self.primary,
            Namespace::RockRidge if self.rock_ridge.is_some() => self.primary,
            Namespace::Joliet => self.joliet?.0,
            Namespace::Enhanced => self.enhanced?,
            _ => return None,
        };
        Some((namespace, root))
    }
}

/// Reads the volume descriptor set one sector at a time.
pub(crate) struct DescriptorScan {
    primary: Option<(Root, u32, u32)>,
    joliet: Option<(Root, JolietLevel)>,
    enhanced: Option<Root>,
    boot_catalog: Option<u32>,
    seen: u32,
}

/// Descriptors read before the set counts as unterminated.
pub(crate) const MAX_DESCRIPTORS: u32 = 64;

fn root_of(
    record: &raw::RootDirectoryRecord,
    table: raw::U32Le,
    table_size: raw::U32Both,
) -> Result<Root, Detail> {
    let header = &record.header;
    if !header.extent.is_consistent() || !header.data_len.is_consistent() {
        return Err(Detail::DescriptorFields);
    }
    let extent = header
        .extent
        .get()
        .checked_add(u32::from(header.extended_attr_record))
        .filter(|&extent| extent != 0)
        .ok_or(Detail::DirectoryRecord)?;
    Ok(Root {
        extent,
        size: header.data_len.get(),
        path_table: (table.get(), table_size.get()),
    })
}

impl DescriptorScan {
    pub(crate) fn new() -> Self {
        Self {
            primary: None,
            joliet: None,
            enhanced: None,
            boot_catalog: None,
            seen: 0,
        }
    }

    /// Takes the next descriptor sector. Returns `true` at the terminator.
    pub(crate) fn feed(&mut self, sector: &[u8; raw::SECTOR_SIZE]) -> Result<bool, Detail> {
        self.seen += 1;
        if self.seen > MAX_DESCRIPTORS {
            return Err(Detail::NoPrimaryDescriptor);
        }
        let descriptor = VolumeDescriptor::from_bytes(*sector);
        if !descriptor.header().is_valid() {
            return Err(Detail::DescriptorHeader);
        }
        match descriptor {
            VolumeDescriptor::Terminator(terminator) => {
                if terminator.reserved.iter().any(|&byte| byte != 0) {
                    return Err(Detail::Terminator);
                }
                return Ok(true);
            }
            VolumeDescriptor::Primary(pvd) if self.primary.is_none() => {
                if !pvd.volume_space_size.is_consistent()
                    || !pvd.volume_set_size.is_consistent()
                    || !pvd.volume_sequence_number.is_consistent()
                    || !pvd.logical_block_size.is_consistent()
                    || !pvd.path_table_size.is_consistent()
                    || !pvd.root.header.volume_sequence_number.is_consistent()
                {
                    return Err(Detail::DescriptorFields);
                }
                let block_size = u32::from(pvd.logical_block_size.get());
                if !(512..=2048).contains(&block_size) || !block_size.is_power_of_two() {
                    return Err(Detail::BlockSize);
                }
                self.primary = Some((
                    root_of(&pvd.root, pvd.type_l_path_table, pvd.path_table_size)?,
                    block_size,
                    pvd.volume_space_size.get(),
                ));
            }
            VolumeDescriptor::Supplementary(svd) => {
                if svd.is_enhanced() {
                    if self.enhanced.is_none() {
                        self.enhanced = Some(root_of(
                            &svd.root,
                            svd.type_l_path_table,
                            svd.path_table_size,
                        )?);
                    }
                } else if let Some(level) =
                    JolietLevel::from_escape_sequences(&svd.escape_sequences)
                    && self.joliet.is_none_or(|(_, old)| old < level)
                {
                    self.joliet = Some((
                        root_of(&svd.root, svd.type_l_path_table, svd.path_table_size)?,
                        level,
                    ));
                }
            }
            VolumeDescriptor::BootRecord(boot) if boot.is_el_torito() => {
                self.boot_catalog.get_or_insert(boot.catalog_ptr.get());
            }
            _ => {}
        }
        Ok(false)
    }

    pub(crate) fn finish(self) -> Result<Info, Detail> {
        let (primary, block_size, volume_blocks) =
            self.primary.ok_or(Detail::NoPrimaryDescriptor)?;
        Ok(Info {
            block_size,
            volume_blocks,
            primary,
            joliet: self.joliet,
            enhanced: self.enhanced,
            rock_ridge: None,
            boot_catalog: self.boot_catalog,
            descriptors: self.seen,
        })
    }
}
