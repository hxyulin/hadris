use alloc::string::String;
use alloc::vec::Vec;

use hadris_fs::{Clock, NoClock};
use hadris_part::PartitionFlags;

use crate::boot::{Emulation, Platform};
use crate::namespace::JolietLevel;

/// The ISO 9660 interchange level of the primary tree.
///
/// The level decides the primary names and whether a file may have more
/// than one extent. Joliet, Rock Ridge and the enhanced tree carry longer
/// names beside it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IsoLevel {
    /// 8.3 names; one extent per file, so files below 4 GiB.
    #[default]
    L1,
    /// Names of up to 30 characters; one extent per file.
    L2,
    /// Level 2 names; files of 4 GiB and more take several extents.
    L3,
}

/// How primary tree names treat lowercase letters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameCase {
    /// Lowercase becomes uppercase, as ECMA-119 d-characters require.
    #[default]
    Upper,
    /// Lowercase is kept, as many producers do; readers accept it.
    Preserve,
}

/// How the volume descriptor identifiers treat characters outside the
/// ECMA-119 a- and d-character sets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Charset {
    /// Identifiers are stored as given, as xorriso and genisoimage do.
    #[default]
    Relaxed,
    /// Lowercase becomes uppercase and other invalid characters `_`.
    Strict,
}

/// The identifiers of the volume descriptors.
///
/// A field left unset stays blank, except the application, which defaults
/// to `HADRIS-ISO`. Joliet and enhanced descriptors carry the same values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VolumeIdentifiers {
    volume: String,
    system: Option<String>,
    volume_set: Option<String>,
    publisher: Option<String>,
    preparer: Option<String>,
    application: Option<String>,
}

impl VolumeIdentifiers {
    /// Identifiers naming the volume `volume`, up to 32 bytes.
    pub fn new(volume: impl Into<String>) -> Self {
        Self {
            volume: volume.into(),
            system: None,
            volume_set: None,
            publisher: None,
            preparer: None,
            application: None,
        }
    }

    /// Sets the system that may use the system area, up to 32 bytes.
    pub fn with_system(self, system: impl Into<String>) -> Self {
        Self {
            system: Some(system.into()),
            ..self
        }
    }

    /// Sets the volume set, up to 128 bytes.
    pub fn with_volume_set(self, volume_set: impl Into<String>) -> Self {
        Self {
            volume_set: Some(volume_set.into()),
            ..self
        }
    }

    /// Sets the publisher, up to 128 bytes.
    pub fn with_publisher(self, publisher: impl Into<String>) -> Self {
        Self {
            publisher: Some(publisher.into()),
            ..self
        }
    }

    /// Sets the data preparer, up to 128 bytes.
    pub fn with_preparer(self, preparer: impl Into<String>) -> Self {
        Self {
            preparer: Some(preparer.into()),
            ..self
        }
    }

    /// Sets the application, up to 128 bytes.
    pub fn with_application(self, application: impl Into<String>) -> Self {
        Self {
            application: Some(application.into()),
            ..self
        }
    }

    /// The volume name.
    pub fn volume(&self) -> &str {
        &self.volume
    }

    /// The system identifier.
    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    /// The volume set identifier.
    pub fn volume_set(&self) -> Option<&str> {
        self.volume_set.as_deref()
    }

    /// The publisher identifier.
    pub fn publisher(&self) -> Option<&str> {
        self.publisher.as_deref()
    }

    /// The data preparer identifier.
    pub fn preparer(&self) -> Option<&str> {
        self.preparer.as_deref()
    }

    /// The application identifier.
    pub fn application(&self) -> Option<&str> {
        self.application.as_deref()
    }
}

impl Default for VolumeIdentifiers {
    fn default() -> Self {
        Self::new("CDROM")
    }
}

bitflags::bitflags! {
    /// The metadata Rock Ridge copies from the tree. What is not copied
    /// gets a default: mode 0644 for files and 0755 for directories, owner
    /// 0, the clock's time.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Preserve: u8 {
        /// Permission bits.
        const PERMISSIONS = 0b001;
        /// Owner user and group.
        const OWNERS = 0b010;
        /// Creation, modification and access times.
        const TIMES = 0b100;
    }
}

/// Where Rock Ridge puts directories nested deeper than ISO 9660 allows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Relocation {
    /// Move them into this directory of the root, as RRIP `CL`, `PL` and
    /// `RE` entries describe. A directory of the tree with that name is
    /// reused and keeps its own entries; any other entry with that name
    /// fails with [`Detail::Relocation`](crate::Detail::Relocation).
    Directory(String),
    /// Fail with [`ErrorKind::InvalidInput`](hadris_fs::ErrorKind::InvalidInput).
    Reject,
}

impl Default for Relocation {
    fn default() -> Self {
        Self::Directory(String::from("rr_moved"))
    }
}

/// Rock Ridge (RRIP 1.12) over the primary tree: POSIX names, modes,
/// owners, times, symlinks, device nodes and hard links.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RockRidge {
    preserve: Preserve,
    relocation: Relocation,
}

impl Default for RockRidge {
    fn default() -> Self {
        Self {
            preserve: Preserve::all(),
            relocation: Relocation::default(),
        }
    }
}

impl RockRidge {
    /// Rock Ridge that copies all metadata and relocates into `rr_moved`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets which metadata is copied from the tree.
    pub fn with_preserve(self, preserve: Preserve) -> Self {
        Self { preserve, ..self }
    }

    /// Sets where deep directories go.
    pub fn with_relocation(self, relocation: Relocation) -> Self {
        Self { relocation, ..self }
    }

    /// The metadata copied from the tree.
    pub fn preserve(&self) -> Preserve {
        self.preserve
    }

    /// Where deep directories go.
    pub fn relocation(&self) -> &Relocation {
        &self.relocation
    }
}

/// A boot information table written into a no-emulation boot image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BootInfo {
    /// Leave the image as it is.
    #[default]
    None,
    /// The 16-byte table at byte 8, as `mkisofs -boot-info-table` writes.
    Standard,
    /// The same table followed by 40 zero bytes, as GRUB 2 and ISOLINUX
    /// expect.
    Grub2,
}

/// One El Torito boot entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BootEntry {
    image: String,
    platform: Platform,
    emulation: Emulation,
    load_size: Option<u16>,
    load_segment: u16,
    info_table: BootInfo,
}

impl BootEntry {
    /// An x86 no-emulation entry booting the file at tree path `image`.
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            platform: Platform::X86,
            emulation: Emulation::NoEmulation,
            load_size: None,
            load_segment: 0,
            info_table: BootInfo::None,
        }
    }

    /// Sets the platform.
    pub fn with_platform(self, platform: Platform) -> Self {
        Self { platform, ..self }
    }

    /// Sets the emulation. The image is used as given: for diskette and
    /// hard-disk emulation it must already hold the disk. A diskette image
    /// must be exactly the diskette's size, or writing fails with
    /// [`Detail::BootImage`](crate::Detail::BootImage).
    pub fn with_emulation(self, emulation: Emulation) -> Self {
        Self { emulation, ..self }
    }

    /// Sets how many 512-byte sectors firmware loads. The default is 1 for
    /// an emulated disk and the whole image, up to 65535, otherwise. Zero
    /// fails with [`Detail::BootImage`](crate::Detail::BootImage); a load
    /// size past the end of a no-emulation image is written as given and
    /// reported as a warning.
    pub fn with_load_size(self, sectors: u16) -> Self {
        Self {
            load_size: Some(sectors),
            ..self
        }
    }

    /// Sets the real-mode load segment; 0 means 0x7C0.
    pub fn with_load_segment(self, segment: u16) -> Self {
        Self {
            load_segment: segment,
            ..self
        }
    }

    /// Writes a boot information table into the image.
    pub fn with_boot_info_table(self, table: BootInfo) -> Self {
        Self {
            info_table: table,
            ..self
        }
    }

    /// The tree path of the image.
    pub fn image(&self) -> &str {
        &self.image
    }

    /// The platform.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The emulation.
    pub fn emulation(&self) -> Emulation {
        self.emulation
    }

    /// The load size, when set.
    pub fn load_size(&self) -> Option<u16> {
        self.load_size
    }

    /// The load segment.
    pub fn load_segment(&self) -> u16 {
        self.load_segment
    }

    /// The boot information table.
    pub fn boot_info_table(&self) -> BootInfo {
        self.info_table
    }
}

/// El Torito boot: a catalog of boot entries.
///
/// The first entry is the default entry and gives the validation entry its
/// platform; each further entry gets a section of its own.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ElTorito {
    entries: Vec<BootEntry>,
    catalog: Option<String>,
}

impl ElTorito {
    /// A catalog with `entry` as its default entry.
    pub fn new(entry: BootEntry) -> Self {
        Self {
            entries: alloc::vec![entry],
            catalog: None,
        }
    }

    /// Adds an entry in a section of its own.
    pub fn with_entry(mut self, entry: BootEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Makes the catalog visible as a file at tree path `path`, whose
    /// parent directory must exist. Without it the catalog is written after
    /// the other data and no directory lists it.
    pub fn with_catalog_path(self, path: impl Into<String>) -> Self {
        Self {
            catalog: Some(path.into()),
            ..self
        }
    }

    /// The entries, default first.
    pub fn entries(&self) -> &[BootEntry] {
        &self.entries
    }

    /// The tree path of the visible catalog.
    pub fn catalog_path(&self) -> Option<&str> {
        self.catalog.as_deref()
    }
}

/// The partition table of a hybrid image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PartitionScheme {
    /// An MBR with one partition over the image, for BIOS USB boot.
    Mbr,
    /// A GPT, with a protective MBR, for UEFI boot.
    Gpt,
    /// A GPT whose MBR mirrors the ISO and EFI partitions, for both.
    Hybrid,
}

/// A partition table in the system area, so the image also boots from a
/// USB stick or disk.
///
/// MBR partition entries count 512-byte sectors in 32 bits, so `Mbr` and
/// `Hybrid` images are limited to 2 TiB. GPT and hybrid images get a backup
/// GPT after the ISO data, counted in the volume space size.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HybridBoot {
    scheme: PartitionScheme,
    bootstrap: Option<Vec<u8>>,
    flags: PartitionFlags,
    efi_partition: Option<String>,
}

impl HybridBoot {
    fn with_scheme(scheme: PartitionScheme, flags: PartitionFlags) -> Self {
        Self {
            scheme,
            bootstrap: None,
            flags,
            efi_partition: None,
        }
    }

    /// An MBR whose partition is marked bootable.
    pub fn mbr() -> Self {
        Self::with_scheme(PartitionScheme::Mbr, PartitionFlags::BOOTABLE)
    }

    /// A GPT.
    pub fn gpt() -> Self {
        Self::with_scheme(PartitionScheme::Gpt, PartitionFlags::empty())
    }

    /// A hybrid MBR and GPT whose ISO partition is marked bootable.
    pub fn hybrid() -> Self {
        Self::with_scheme(PartitionScheme::Hybrid, PartitionFlags::BOOTABLE)
    }

    /// Sets the boot code in the MBR, at most 446 bytes. Longer code fails
    /// the write with [`ErrorKind::LimitExceeded`](hadris_fs::ErrorKind::LimitExceeded)
    /// and [`Detail::HybridBoot`](crate::Detail::HybridBoot).
    pub fn with_bootstrap(self, code: impl Into<Vec<u8>>) -> Self {
        Self {
            bootstrap: Some(code.into()),
            ..self
        }
    }

    /// Sets the MBR flags of the ISO partition, such as
    /// [`PartitionFlags::BOOTABLE`].
    pub fn with_flags(self, flags: PartitionFlags) -> Self {
        Self { flags, ..self }
    }

    /// Exposes the file at tree path `image` as the EFI system partition of
    /// a GPT or hybrid table. Without it, the image of the only UEFI boot
    /// entry is used, if there is exactly one; with several, the table gets
    /// no EFI system partition and the report warns.
    pub fn with_efi_partition(self, image: impl Into<String>) -> Self {
        Self {
            efi_partition: Some(image.into()),
            ..self
        }
    }

    /// The partition scheme.
    pub fn scheme(&self) -> PartitionScheme {
        self.scheme
    }

    /// The MBR boot code.
    pub fn bootstrap(&self) -> Option<&[u8]> {
        self.bootstrap.as_deref()
    }

    /// The MBR flags of the ISO partition.
    pub fn flags(&self) -> PartitionFlags {
        self.flags
    }

    /// The tree path of the EFI system partition image.
    pub fn efi_partition(&self) -> Option<&str> {
        self.efi_partition.as_deref()
    }
}

/// Options for `write`, `plan` and `Session::write`.
///
/// The default is a Level 1 image named `CDROM` with no extensions, dated
/// by [`NoClock`], so the same tree always gives the same bytes.
///
/// ```rust
/// use hadris_iso::{IsoLevel, IsoOptions, JolietLevel, RockRidge, VolumeIdentifiers};
///
/// let options = IsoOptions::default()
///     .with_volume(VolumeIdentifiers::new("INSTALL"))
///     .with_level(IsoLevel::L3)
///     .with_joliet(JolietLevel::L3)
///     .with_rock_ridge(RockRidge::default());
/// assert!(options.rock_ridge().is_some());
/// ```
#[derive(Debug, Clone)]
pub struct IsoOptions<C = NoClock> {
    volume: VolumeIdentifiers,
    level: IsoLevel,
    name_case: NameCase,
    charset: Charset,
    joliet: Option<JolietLevel>,
    rock_ridge: Option<RockRidge>,
    enhanced: bool,
    el_torito: Option<ElTorito>,
    hybrid: Option<HybridBoot>,
    min_blocks: u64,
    clock: C,
}

impl Default for IsoOptions {
    fn default() -> Self {
        Self {
            volume: VolumeIdentifiers::default(),
            level: IsoLevel::default(),
            name_case: NameCase::default(),
            charset: Charset::default(),
            joliet: None,
            rock_ridge: None,
            enhanced: false,
            el_torito: None,
            hybrid: None,
            min_blocks: 0,
            clock: NoClock,
        }
    }
}

impl IsoOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: Clock> IsoOptions<C> {
    /// Sets the volume descriptor identifiers.
    pub fn with_volume(self, volume: VolumeIdentifiers) -> Self {
        Self { volume, ..self }
    }

    /// Sets the interchange level of the primary tree.
    pub fn with_level(self, level: IsoLevel) -> Self {
        Self { level, ..self }
    }

    /// Sets how primary names treat lowercase.
    pub fn with_name_case(self, name_case: NameCase) -> Self {
        Self { name_case, ..self }
    }

    /// Sets how descriptor identifiers treat invalid characters.
    pub fn with_charset(self, charset: Charset) -> Self {
        Self { charset, ..self }
    }

    /// Adds a Joliet tree with UCS-2 names of up to 64 characters.
    pub fn with_joliet(self, level: JolietLevel) -> Self {
        Self {
            joliet: Some(level),
            ..self
        }
    }

    /// Adds Rock Ridge to the primary tree. Without it, symlinks and device
    /// nodes are left out and reported, and deep trees fail.
    pub fn with_rock_ridge(self, rock_ridge: RockRidge) -> Self {
        Self {
            rock_ridge: Some(rock_ridge),
            ..self
        }
    }

    /// Adds an ISO 9660:1999 enhanced tree with names of up to 207 bytes
    /// that keep their case.
    pub fn with_enhanced_tree(self) -> Self {
        Self {
            enhanced: true,
            ..self
        }
    }

    /// Makes the image bootable from optical media.
    pub fn with_el_torito(self, el_torito: ElTorito) -> Self {
        Self {
            el_torito: Some(el_torito),
            ..self
        }
    }

    /// Adds a partition table, so the image boots from disks too.
    pub fn with_hybrid(self, hybrid: HybridBoot) -> Self {
        Self {
            hybrid: Some(hybrid),
            ..self
        }
    }

    /// Places directories and files at or after logical block `blocks`,
    /// leaving room for another format's structures after the descriptors.
    pub fn with_min_blocks(self, blocks: u64) -> Self {
        Self {
            min_blocks: blocks,
            ..self
        }
    }

    /// Sets the clock that dates the volume and the entries without times.
    pub fn with_clock<C2: Clock>(self, clock: C2) -> IsoOptions<C2> {
        IsoOptions {
            volume: self.volume,
            level: self.level,
            name_case: self.name_case,
            charset: self.charset,
            joliet: self.joliet,
            rock_ridge: self.rock_ridge,
            enhanced: self.enhanced,
            el_torito: self.el_torito,
            hybrid: self.hybrid,
            min_blocks: self.min_blocks,
            clock,
        }
    }

    /// The volume descriptor identifiers.
    pub fn volume(&self) -> &VolumeIdentifiers {
        &self.volume
    }

    /// The interchange level.
    pub fn level(&self) -> IsoLevel {
        self.level
    }

    /// How primary names treat lowercase.
    pub fn name_case(&self) -> NameCase {
        self.name_case
    }

    /// How descriptor identifiers treat invalid characters.
    pub fn charset(&self) -> Charset {
        self.charset
    }

    /// The Joliet level, if a Joliet tree is written.
    pub fn joliet(&self) -> Option<JolietLevel> {
        self.joliet
    }

    /// The Rock Ridge options, if Rock Ridge is written.
    pub fn rock_ridge(&self) -> Option<&RockRidge> {
        self.rock_ridge.as_ref()
    }

    /// Whether an enhanced tree is written.
    pub fn has_enhanced_tree(&self) -> bool {
        self.enhanced
    }

    /// The El Torito options.
    pub fn el_torito(&self) -> Option<&ElTorito> {
        self.el_torito.as_ref()
    }

    /// The hybrid boot options.
    pub fn hybrid(&self) -> Option<&HybridBoot> {
        self.hybrid.as_ref()
    }

    /// The first logical block available to directories and files.
    pub fn min_blocks(&self) -> u64 {
        self.min_blocks
    }

    /// The clock.
    pub fn clock(&self) -> &C {
        &self.clock
    }
}

/// How `Session::write` puts a changed tree back on the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionMode {
    /// A new session after the last one: its own descriptor set, directory
    /// tree and new file data, reusing the extents of unchanged files. The
    /// descriptors at logical sector 16 are replaced by a copy of the new
    /// set, so readers without a table of contents see the new session; the
    /// system area and its partition tables are left as they are, even
    /// when the options have hybrid boot. The session starts after the old
    /// volume and after every partition and backup GPT of the image.
    Append,
    /// The same image updated in place, for rewritable media and image
    /// files: new directories, path tables and file data after the old
    /// volume and every partition, and the descriptors at sector 16
    /// replaced. Hybrid boot data is kept, and GPT and MBR partitions that
    /// ended with the old volume are extended unless that would overlap
    /// another partition, with the backup GPT moved to the new end.
    Rewrite,
}
