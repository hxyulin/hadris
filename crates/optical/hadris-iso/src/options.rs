use alloc::string::String;
use alloc::vec::Vec;

use hadris_fs::{Content, DateTime, NoClock};
use hadris_part::PartitionFlags;

use crate::boot::{Emulation, Platform};

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

/// A text identifier of the volume descriptors.
///
/// Joliet and ISO 9660:1999 descriptors carry the same values. The `File`
/// identifiers name files of the root directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IsoId {
    /// The system that may use the system area, up to 32 bytes.
    System,
    /// The volume name, up to 32 bytes; `CDROM` by default.
    Volume,
    /// The volume set, up to 128 bytes.
    VolumeSet,
    /// The publisher, up to 128 bytes.
    Publisher,
    /// The data preparer, up to 128 bytes.
    Preparer,
    /// The application, up to 128 bytes; `HADRIS-ISO` by default.
    Application,
    /// The copyright file, up to 37 bytes.
    CopyrightFile,
    /// The abstract file, up to 37 bytes.
    AbstractFile,
    /// The bibliographic file, up to 37 bytes.
    BibliographicFile,
}

impl IsoId {
    const fn index(self) -> usize {
        self as usize
    }
}

/// A date of the volume descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IsoDate {
    /// When the volume was created; the options' time by default.
    Created,
    /// When the volume was last modified; the options' time by default.
    Modified,
    /// When the data becomes obsolete; unspecified by default.
    Expires,
    /// When the data may first be used; unspecified by default.
    Effective,
}

impl IsoDate {
    const fn index(self) -> usize {
        self as usize
    }
}

bitflags::bitflags! {
    /// The metadata Rock Ridge copies from the tree. What is not copied
    /// gets a default: mode 0644 for files and 0755 for directories, owner
    /// 0, the options' time.
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
///
/// They move into a directory of the root, as RRIP `CL`, `PL` and `RE`
/// entries describe. libarchive (`bsdtar`) reads relocated directories only
/// from a root directory named `rr_moved` or `.rr_moved`, so those are the
/// two names offered. A directory of the tree with the chosen name is reused
/// and keeps its own entries; any other entry with that name fails with
/// [`Detail::Relocation`](crate::Detail::Relocation). libarchive takes the
/// first root directory with either name, so a tree directory with the other
/// name whose primary identifier sorts first fails too: `.rr_moved` with
/// [`NameCase::Preserve`] and [`RrMoved`](Self::RrMoved), or `rr_moved` with
/// [`DotRrMoved`](Self::DotRrMoved) and [`NameCase::Upper`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Relocation {
    /// Move them into `rr_moved`.
    #[default]
    RrMoved,
    /// Move them into `.rr_moved`.
    DotRrMoved,
    /// Fail with [`ErrorKind::InvalidInput`](hadris_fs::ErrorKind::InvalidInput).
    Refuse,
}

impl Relocation {
    /// The relocation directory's name, or `None` for
    /// [`Refuse`](Self::Refuse).
    pub const fn directory(self) -> Option<&'static str> {
        match self {
            Self::RrMoved => Some("rr_moved"),
            Self::DotRrMoved => Some(".rr_moved"),
            Self::Refuse => None,
        }
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
    Table,
    /// The same table followed by 40 zero bytes, as GRUB 2 and ISOLINUX
    /// expect.
    Grub2,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum BootImage {
    Path(String),
    Appended(usize),
}

/// One El Torito boot entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BootEntry {
    image: BootImage,
    platform: Platform,
    emulation: Emulation,
    load_size: Option<u16>,
    load_segment: u16,
    info: BootInfo,
}

impl BootEntry {
    fn with_image(image: BootImage, platform: Platform) -> Self {
        Self {
            image,
            platform,
            emulation: Emulation::NoEmulation,
            load_size: None,
            load_segment: 0,
            info: BootInfo::None,
        }
    }

    /// A BIOS (x86) no-emulation entry booting the file at tree path
    /// `image`.
    pub fn bios(image: &str) -> Self {
        Self::with_image(BootImage::Path(String::from(image)), Platform::X86)
    }

    /// A UEFI entry booting the file at tree path `image`, usually a FAT
    /// image holding `EFI/BOOT/BOOTX64.EFI`.
    pub fn uefi(image: &str) -> Self {
        Self::with_image(BootImage::Path(String::from(image)), Platform::Efi)
    }

    /// A UEFI entry booting the partition that
    /// [`Hybrid::with_appended`] added as number `index`, counted from 0,
    /// so the image is stored once for El Torito and the partition table.
    pub fn uefi_appended(index: usize) -> Self {
        Self::with_image(BootImage::Appended(index), Platform::Efi)
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
    pub fn with_boot_info(self, info: BootInfo) -> Self {
        Self { info, ..self }
    }

    /// The tree path of the image, unless it is an appended partition.
    pub fn image(&self) -> Option<&str> {
        match &self.image {
            BootImage::Path(path) => Some(path),
            BootImage::Appended(_) => None,
        }
    }

    /// The number of the appended partition the entry boots, if it boots
    /// one.
    pub fn appended(&self) -> Option<usize> {
        match self.image {
            BootImage::Appended(index) => Some(index),
            BootImage::Path(_) => None,
        }
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
    pub fn boot_info(&self) -> BootInfo {
        self.info
    }
}

/// El Torito boot: a catalog of boot entries.
///
/// The first entry is the default entry and gives the validation entry its
/// platform; each further entry gets a section of its own. A catalog
/// without entries fails the plan with
/// [`Detail::BootImage`](crate::Detail::BootImage).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ElTorito {
    entries: Vec<BootEntry>,
    catalog: Option<String>,
}

impl ElTorito {
    /// An empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an entry: the first is the default entry, each further one
    /// gets a section of its own.
    pub fn with_entry(mut self, entry: BootEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Makes the catalog visible as a file at tree path `path`, whose
    /// parent directory must exist. Without it the catalog is written after
    /// the other data and no directory lists it.
    pub fn with_catalog_path(self, path: &str) -> Self {
        Self {
            catalog: Some(String::from(path)),
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
pub(crate) enum PartitionScheme {
    Mbr,
    Gpt,
    GptHybridMbr,
}

/// A partition stored after the files of the image, inside the ISO 9660
/// volume but in no directory, and listed in the partition table.
#[derive(Debug, Clone)]
pub struct AppendedPartition {
    content: Content,
}

impl AppendedPartition {
    /// An EFI system partition holding `content`, usually a FAT image. The
    /// first one is the EFI system partition of a GPT, and
    /// [`BootEntry::uefi_appended`] boots it from optical media.
    pub fn esp(content: Content) -> Self {
        Self { content }
    }

    /// The partition's bytes.
    pub fn content(&self) -> &Content {
        &self.content
    }
}

/// A partition table in the system area, so the image also boots from a
/// USB stick or disk.
///
/// MBR partition entries count 512-byte sectors in 32 bits, so `mbr` and
/// `gpt_hybrid_mbr` images are limited to 2 TiB. GPT images get a backup
/// GPT after the ISO data, counted in the volume space size.
///
/// A GPT gets an EFI system partition over the first appended partition,
/// or else over the image of the only UEFI boot entry after the default
/// entry. With several such entries and no appended partition it gets
/// none, and the report warns.
#[derive(Debug, Clone)]
pub struct Hybrid {
    scheme: PartitionScheme,
    bootstrap: Option<Vec<u8>>,
    flags: PartitionFlags,
    appended: Vec<AppendedPartition>,
}

impl Hybrid {
    fn with_scheme(scheme: PartitionScheme, flags: PartitionFlags) -> Self {
        Self {
            scheme,
            bootstrap: None,
            flags,
            appended: Vec::new(),
        }
    }

    /// An MBR with one partition over the image, marked bootable, for BIOS
    /// USB boot.
    pub fn mbr() -> Self {
        Self::with_scheme(PartitionScheme::Mbr, PartitionFlags::BOOTABLE)
    }

    /// A GPT with a protective MBR, for UEFI boot.
    pub fn gpt() -> Self {
        Self::with_scheme(PartitionScheme::Gpt, PartitionFlags::empty())
    }

    /// A GPT whose MBR mirrors the ISO partition, marked bootable, and the
    /// EFI system partition, for BIOS and UEFI boot.
    pub fn gpt_hybrid_mbr() -> Self {
        Self::with_scheme(PartitionScheme::GptHybridMbr, PartitionFlags::BOOTABLE)
    }

    /// Sets the boot code in the MBR, at most 446 bytes. Longer code fails
    /// the plan with [`ErrorKind::LimitExceeded`](hadris_fs::ErrorKind::LimitExceeded)
    /// and [`Detail::HybridBoot`](crate::Detail::HybridBoot). A plain GPT
    /// has no boot code.
    pub fn with_bootstrap(self, code: &[u8]) -> Self {
        Self {
            bootstrap: Some(code.to_vec()),
            ..self
        }
    }

    /// Sets the MBR flags of the ISO partition, such as
    /// [`PartitionFlags::BOOTABLE`].
    pub fn with_flags(self, flags: PartitionFlags) -> Self {
        Self { flags, ..self }
    }

    /// Adds a partition after the files. Only a GPT lists it; an `mbr`
    /// table fails the plan with
    /// [`Detail::HybridBoot`](crate::Detail::HybridBoot).
    pub fn with_appended(mut self, partition: AppendedPartition) -> Self {
        self.appended.push(partition);
        self
    }

    pub(crate) fn scheme(&self) -> PartitionScheme {
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

    /// The appended partitions, in order.
    pub fn appended(&self) -> &[AppendedPartition] {
        &self.appended
    }
}

/// Options for `write`, `plan` and `Session::write`.
///
/// The default is a Level 1 image named `CDROM` with no extensions, dated
/// 1980-01-01 ([`NoClock::TIME`]), so the same tree always gives the same
/// bytes.
///
/// ```rust
/// use hadris_iso::{IsoId, IsoLevel, IsoOptions};
///
/// let options = IsoOptions::new()
///     .with_id(IsoId::Volume, "INSTALL")
///     .with_level(IsoLevel::L3)
///     .with_joliet()
///     .with_rock_ridge();
/// assert!(options.rock_ridge());
/// assert_eq!(options.id(IsoId::Volume), Some("INSTALL"));
/// ```
#[derive(Debug, Clone)]
pub struct IsoOptions {
    ids: [Option<String>; 9],
    dates: [Option<DateTime>; 4],
    level: IsoLevel,
    name_case: NameCase,
    joliet: bool,
    rock_ridge: bool,
    relocation: Relocation,
    preserve: Preserve,
    iso1999: bool,
    el_torito: Option<ElTorito>,
    hybrid: Option<Hybrid>,
    min_blocks: u64,
    time: DateTime,
    seed: Option<u64>,
}

impl Default for IsoOptions {
    fn default() -> Self {
        let mut ids = [const { None }; 9];
        ids[IsoId::Volume.index()] = Some(String::from("CDROM"));
        ids[IsoId::Application.index()] = Some(String::from("HADRIS-ISO"));
        Self {
            ids,
            dates: [None; 4],
            level: IsoLevel::default(),
            name_case: NameCase::default(),
            joliet: false,
            rock_ridge: false,
            relocation: Relocation::default(),
            preserve: Preserve::all(),
            iso1999: false,
            el_torito: None,
            hybrid: None,
            min_blocks: 0,
            time: NoClock::TIME,
            seed: None,
        }
    }
}

impl IsoOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a descriptor identifier. It is stored as given, as xorriso and
    /// genisoimage do, in UCS-2 for Joliet; one longer than its field fails
    /// the plan with [`Detail::Identifier`](crate::Detail::Identifier).
    /// An empty identifier leaves the field blank.
    pub fn with_id(mut self, id: IsoId, value: &str) -> Self {
        self.ids[id.index()] = Some(String::from(value));
        self
    }

    /// Sets a descriptor date.
    pub fn with_date(mut self, date: IsoDate, time: DateTime) -> Self {
        self.dates[date.index()] = Some(time);
        self
    }

    /// Sets the interchange level of the primary tree.
    pub fn with_level(self, level: IsoLevel) -> Self {
        Self { level, ..self }
    }

    /// Sets how primary names treat lowercase.
    pub fn with_name_case(self, name_case: NameCase) -> Self {
        Self { name_case, ..self }
    }

    /// Adds a Joliet tree with UCS-2 names of up to 64 characters.
    pub fn with_joliet(self) -> Self {
        Self {
            joliet: true,
            ..self
        }
    }

    /// Adds Rock Ridge to the primary tree. Without it, symlinks and device
    /// nodes are left out and reported, and deep trees fail.
    pub fn with_rock_ridge(self) -> Self {
        Self {
            rock_ridge: true,
            ..self
        }
    }

    /// Sets where Rock Ridge puts deep directories; `rr_moved` by default.
    pub fn with_relocation(self, relocation: Relocation) -> Self {
        Self { relocation, ..self }
    }

    /// Sets which metadata Rock Ridge copies from the tree; all of it by
    /// default.
    pub fn with_preserve(self, preserve: Preserve) -> Self {
        Self { preserve, ..self }
    }

    /// Adds an ISO 9660:1999 tree with names of up to 207 bytes that keep
    /// their case.
    pub fn with_iso1999(self) -> Self {
        Self {
            iso1999: true,
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
    pub fn with_hybrid(self, hybrid: Hybrid) -> Self {
        Self {
            hybrid: Some(hybrid),
            ..self
        }
    }

    /// Places directories and files at or after logical block `blocks`,
    /// leaving room for another format's structures after the descriptors,
    /// as the ISO 9660 and UDF bridge writer does.
    pub fn with_min_blocks(self, blocks: u64) -> Self {
        Self {
            min_blocks: blocks,
            ..self
        }
    }

    /// Sets the time that dates the volume and the entries without times,
    /// such as `SOURCE_DATE_EPOCH`. The writer reads no clock.
    pub fn with_time(self, time: DateTime) -> Self {
        Self { time, ..self }
    }

    /// Sets the seed the GPT disk and partition GUIDs derive from, with the
    /// tree's paths, sizes and times. Without one they derive from the time
    /// and the tree, so the same inputs give the same image and different
    /// trees different GUIDs.
    pub fn with_seed(self, seed: u64) -> Self {
        Self {
            seed: Some(seed),
            ..self
        }
    }

    /// A descriptor identifier, or `None` when it is blank.
    pub fn id(&self, id: IsoId) -> Option<&str> {
        self.ids[id.index()].as_deref().filter(|id| !id.is_empty())
    }

    /// A descriptor date, as written: the options' time for
    /// [`IsoDate::Created`] and [`IsoDate::Modified`] unless set, `None`
    /// for an unspecified date.
    pub fn date(&self, date: IsoDate) -> Option<DateTime> {
        match (self.dates[date.index()], date) {
            (Some(time), _) => Some(time),
            (None, IsoDate::Created | IsoDate::Modified) => Some(self.time),
            (None, _) => None,
        }
    }

    /// The interchange level.
    pub fn level(&self) -> IsoLevel {
        self.level
    }

    /// How primary names treat lowercase.
    pub fn name_case(&self) -> NameCase {
        self.name_case
    }

    /// Whether a Joliet tree is written.
    pub fn joliet(&self) -> bool {
        self.joliet
    }

    /// Whether Rock Ridge is written.
    pub fn rock_ridge(&self) -> bool {
        self.rock_ridge
    }

    /// Where Rock Ridge puts deep directories.
    pub fn relocation(&self) -> Relocation {
        self.relocation
    }

    /// The metadata Rock Ridge copies from the tree.
    pub fn preserve(&self) -> Preserve {
        self.preserve
    }

    /// Whether an ISO 9660:1999 tree is written.
    pub fn iso1999(&self) -> bool {
        self.iso1999
    }

    /// The El Torito options.
    pub fn el_torito(&self) -> Option<&ElTorito> {
        self.el_torito.as_ref()
    }

    /// The hybrid boot options.
    pub fn hybrid(&self) -> Option<&Hybrid> {
        self.hybrid.as_ref()
    }

    /// The first logical block available to directories and files.
    pub fn min_blocks(&self) -> u64 {
        self.min_blocks
    }

    /// The time that dates the volume and the entries without times.
    pub fn time(&self) -> DateTime {
        self.time
    }

    /// The seed of the GUIDs, if set.
    pub fn seed(&self) -> Option<u64> {
        self.seed
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
