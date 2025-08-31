#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{
    fmt,
    ops::{Index, IndexMut},
};

use hadris_common::types::{
    endian::{Endian, LittleEndian},
    number::{U16, U32},
};
use hadris_io::{self as io, Read, Seek, SeekFrom, Write};

/// A Type Representing a FAT Sector
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Sector<T = usize>(pub T);

pub trait SectorLike {
    fn to_bytes(self, bytes_per_sector: usize) -> usize;
}

macro_rules! sector_impl {
    ($ty:ty) => {
        impl SectorLike for Sector<$ty> {
            fn to_bytes(self, bytes_per_sector: usize) -> usize {
                (self.0 as usize) * bytes_per_sector
            }
        }
    };
}
sector_impl!(u8);
sector_impl!(u16);
sector_impl!(u32);
sector_impl!(u64);
sector_impl!(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Cluster<T = usize>(pub T);

pub(crate) trait ClusterLike {
    fn to_bytes(self, data_start: usize, bytes_per_cluster: usize) -> usize;
}

macro_rules! cluster_impl {
    ($ty:ty) => {
        impl ClusterLike for Cluster<$ty> {
            fn to_bytes(self, data_start: usize, bytes_per_cluster: usize) -> usize {
                data_start + (self.0 as usize - 2) * bytes_per_cluster
            }
        }
    };
}
cluster_impl!(u8);
cluster_impl!(u16);
cluster_impl!(u32);
cluster_impl!(u64);
cluster_impl!(usize);

pub struct SectorCursor<DATA: Seek> {
    data: DATA,
    sector_size: usize,
}

impl<DATA: Seek> SectorCursor<DATA> {
    pub const fn new(data: DATA, sector_size: usize) -> Self {
        Self { data, sector_size }
    }

    pub fn seek_sector(&mut self, sector: impl SectorLike) -> io::Result<u64> {
        self.seek(SeekFrom::Start(sector.to_bytes(self.sector_size) as u64))
    }
}

pub trait ReadHelper: Read {
    fn read_struct<R: bytemuck::AnyBitPattern + bytemuck::NoUninit + bytemuck::Zeroable>(
        &mut self,
    ) -> io::Result<R> {
        let mut data = R::zeroed();
        self.read_exact(bytemuck::bytes_of_mut(&mut data))?;
        Ok(data)
    }
}

impl<T> ReadHelper for T where T: Read {}

impl<T> Seek for SectorCursor<T>
where
    T: Seek,
{
    fn seek(&mut self, pos: hadris_io::SeekFrom) -> hadris_io::Result<u64> {
        self.data.seek(pos)
    }

    fn rewind(&mut self) -> hadris_io::Result<()> {
        self.data.rewind()
    }

    fn seek_relative(&mut self, offset: i64) -> hadris_io::Result<()> {
        self.data.seek_relative(offset)
    }

    fn stream_position(&mut self) -> hadris_io::Result<u64> {
        self.data.stream_position()
    }
}

impl<T> Read for SectorCursor<T>
where
    T: Read + Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> hadris_io::Result<usize> {
        self.data.read(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> hadris_io::Result<()> {
        self.data.read_exact(buf)
    }
}

pub struct FatFs<DATA: Seek> {
    data: SectorCursor<DATA>,
    info: FatInfo,
    fat: Fat,
    ext: FatFsExt,
}

#[derive(Debug)]
struct FatInfo {
    cluster_size: usize,
    data_start: usize,
}

impl FatInfo {
    fn cluster_start(&self, cluster: impl ClusterLike) -> usize {
        cluster.to_bytes(self.data_start, self.cluster_size)
    }
}

impl<DATA: Seek> fmt::Debug for FatFs<DATA> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FatFs")
            .field("info", &self.info)
            .field("ext", &self.ext)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
enum FatFsExt {
    Fat32(Fat32FsExt),
}

impl FatFsExt {
    fn root_clus(&self) -> Cluster<usize> {
        Cluster(match self {
            Self::Fat32(ext) => ext.root_clus.0 as usize,
        })
    }
}

#[derive(Debug)]
struct Fat32FsExt {
    fs_info_sec: Sector<u16>,
    root_clus: Cluster<u32>,
    free_count: u32,
    next_free: Cluster<u32>,
}

/// Implementations for Read APIs
impl<DATA> FatFs<DATA>
where
    DATA: Read + Seek,
{
    pub fn parse(mut data: DATA) -> io::Result<Self> {
        let bpb = data.read_struct::<RawBpb>()?;
        let mut data = SectorCursor::new(data, bpb.bytes_per_sector.get() as usize);
        let cluster_size = (bpb.sectors_per_cluster as usize) * data.sector_size;
        let bpb_ext32 = data.read_struct::<RawBpbExt32>()?;
        if bpb_ext32.signature_word.get() != 0xAA55 {
            panic!("invalid boot signature");
        }
        if bpb_ext32.ext_boot_signature != 0x29 {
            // Seek backwards before the BPB Ext
            data.seek_relative(-(size_of::<RawBpbExt32>() as i64))?;
            panic!("invalid BPB Ext32, the image is probably FAT12/16");
        }
        let fs_info_sec = Sector(bpb_ext32.fs_info_sector.get());
        data.seek_sector(fs_info_sec)?;
        let fs_info = data.read_struct::<RawFsInfo>()?;
        let ext = FatFsExt::Fat32(Fat32FsExt {
            fs_info_sec,
            root_clus: Cluster(bpb_ext32.root_cluster.get()),
            free_count: fs_info.free_count.get(),
            next_free: Cluster(fs_info.next_free.get()),
        });
        let fat_start = Sector(bpb.reserved_sector_count.get()).to_bytes(data.sector_size);
        let fat_size = bpb.fat_count as usize
            * Sector(bpb_ext32.sectors_per_fat_32.get()).to_bytes(data.sector_size);
        let fat = Fat::Fat32(Fat32 {
            start: fat_start,
            size: fat_size,
            count: bpb.fat_count as usize,
        });
        let info = FatInfo {
            cluster_size,
            data_start: fat_start + fat_size,
        };

        Ok(Self {
            data,
            info,
            fat,
            ext,
        })
    }

    pub fn root_dir(&mut self) -> FileHandle {
        FileHandle(FileHandleInner::RootDir(self.ext.root_clus()))
    }
}

pub struct FileHandle(FileHandleInner);

enum FileHandleInner {
    /// The Root Cluster
    RootDir(Cluster<usize>),
    /// An Entry, storing the index of the entry
    Entry {
        parent_cluster: Cluster<usize>,
        entry_offset_within_cluster: usize,
    },
}

impl FileHandle {
    pub fn entries<'a, DATA: Seek + Read>(&self, fs: &'a mut FatFs<DATA>) -> io::Result<DirectoryIter<'a, DATA>> {
        let cluster = match self.0 {
            FileHandleInner::RootDir(cluster) => cluster,
            FileHandleInner::Entry {
                parent_cluster,
                entry_offset_within_cluster,
            } => {
                let offset = fs.info.cluster_start(parent_cluster) + entry_offset_within_cluster;
                fs.data.seek(SeekFrom::Start(offset as u64))?;
                let entry = fs.data.read_struct::<RawFileEntry>()?;
                assert!(
                    DirEntryAttrFlags::from_bits_retain(entry.attributes)
                        .contains(DirEntryAttrFlags::DIRECTORY),
                    "FileHandle is not a directory"
                );
                let cluster = (u32::from(entry.first_cluster_high.get()) << 16)
                    | u32::from(entry.first_cluster_low.get());
                Cluster(cluster as usize)
            }
        };

        Ok(DirectoryIter { fs, cluster, offset: 0 })
    }

    pub fn read<'a, DATA: Seek + Read>(
        &self,
        fs: &'a mut FatFs<DATA>,
    ) -> io::Result<FileReader<'a, DATA>> {
        let (cluster, size) = match self.0 {
            FileHandleInner::RootDir(_) => panic!("RootDirectory is not a file!"),
            FileHandleInner::Entry {
                parent_cluster,
                entry_offset_within_cluster,
            } => {
                let offset = fs.info.cluster_start(parent_cluster) + entry_offset_within_cluster;
                fs.data.seek(SeekFrom::Start(offset as u64))?;
                let entry = fs.data.read_struct::<RawFileEntry>()?;
                assert!(
                    !DirEntryAttrFlags::from_bits_retain(entry.attributes)
                        .contains(DirEntryAttrFlags::DIRECTORY),
                    "FileHandle is not a file"
                );
                let cluster = (u32::from(entry.first_cluster_high.get()) << 16)
                    | u32::from(entry.first_cluster_low.get());
                (Cluster(cluster as usize), entry.size.get())
            }
        };
        Ok(FileReader {
            fs,
            cluster,
            offset: 0,
            total_read: 0,
            size: size as usize,
        })
    }
}

#[derive(Debug)]
pub struct FileReader<'a, DATA: Seek + Read> {
    fs: &'a mut FatFs<DATA>,
    cluster: Cluster<usize>,
    offset: usize,
    total_read: usize,
    size: usize,
}

impl<DATA: Seek + Read> Read for FileReader<'_, DATA> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.total_read >= self.size {
            return Ok(0);
        }
        if self.offset >= self.fs.info.cluster_size {
            match self
                .fs
                .fat
                .next_cluster(&mut self.fs.data, self.cluster.0)?
            {
                None => return Ok(0),
                Some(cluster) => self.cluster.0 = cluster as usize,
            }
            self.offset = 0;
            std::dbg!(self.cluster);
        }
        let read_max = buf
            .len()
            .min(self.fs.info.cluster_size - self.offset)
            .min(self.size - self.total_read);
        let seek = self.fs.info.cluster_start(self.cluster) + self.offset;
        self.fs.data.seek(SeekFrom::Start(seek as u64))?;
        let read = if read_max > buf.len() {
            self.fs.data.read(buf)?
        } else {
            self.fs.data.read(&mut buf[..read_max])?
        };
        self.offset += read;
        self.total_read += read;
        return Ok(read);
    }
}

pub struct DirectoryIter<'a, DATA: Seek + Read> {
    pub fs: &'a mut FatFs<DATA>,
    cluster: Cluster<usize>,
    offset: usize,
}

impl<DATA: Seek + Read> DirectoryIter<'_, DATA> {
    // FIXME:We don't have a guarantee here that the directory doesn't change at the type level, so
    // we will need some sort of lock guard here, maybe using spin::RwLock?
    pub fn next(
        &mut self,
    ) -> io::Result<Option<DirectoryEntry>> {
        loop {
            if self.offset > self.fs.info.cluster_size {
                match self.fs.fat.next_cluster(&mut self.fs.data, self.cluster.0)? {
                    None => return Ok(None),
                    Some(clus) => self.cluster = Cluster(clus as usize),
                }
                self.offset = 0;
                continue;
            }
            let data = &mut self.fs.data;
            data.seek(SeekFrom::Start(
                (self.fs.info.cluster_start(self.cluster) + self.offset) as u64,
            ))?;
            let data = data.read_struct::<RawDirectoryEntry>()?;
            self.offset += size_of::<RawDirectoryEntry>();
            match unsafe { data.bytes }[0] {
                0x00 => return Ok(None),
                0xE5 => continue,
                _ => {}
            }

            let mut dir_ent = unsafe { data.file };
            // SPEC: 0xE5 indicates it is free, so if the charset uses 0xE5, it has to be converted
            // back
            if dir_ent.name[0] == 0x05 {
                dir_ent.name[0] = 0xE5;
            }

            return Ok(Some(DirectoryEntry::Entry(FileEntry {
                name: ShortFileName::new(dir_ent.name).expect("invalid file name"),
                attr: DirEntryAttrFlags::from_bits_retain(dir_ent.attributes),
                size: dir_ent.size.get() as usize,
                cluster: Cluster(
                    ((u32::from(dir_ent.first_cluster_high.get()) << 16)
                        | u32::from(dir_ent.first_cluster_low.get())) as usize,
                ),
                parent_clus: self.cluster,
                offset_within_cluster: self.offset - size_of::<RawDirectoryEntry>(),
            })));
        }
    }
}

#[derive(Debug)]
pub enum DirectoryEntry {
    Entry(FileEntry),
    #[cfg(feature = "lfn")]
    Lfn(LfnEntry),
}

impl DirectoryEntry {
    pub fn name(&self) -> &str {
        match self {
            Self::Entry(ent) => unsafe { core::str::from_utf8_unchecked(&ent.name.bytes) },
            #[cfg(feature = "lfn")]
            Self::Lfn(_lfn) => unimplemented!(),
        }
    }

    pub fn handle(&self) -> FileHandle {
        match self {
            Self::Entry(ent) => ent.handle(),
            #[cfg(feature = "lfn")]
            Self::Lfn(_lfn) => unimplemented!(),
        }
    }
}

#[derive(Debug)]
pub struct ParseInfo<T> {
    pub data: T,
    pub warnings: FileSystemWarnings,
    pub errors: FileSystemErrors,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct FileSystemWarnings: u64 {

    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct FileSystemErrors: u64 {

    }
}

#[derive(Debug)]
pub struct FileEntry {
    name: ShortFileName,
    attr: DirEntryAttrFlags,
    size: usize,
    parent_clus: Cluster<usize>,
    offset_within_cluster: usize,
    cluster: Cluster<usize>,
}

impl FileEntry {
    pub fn handle(&self) -> FileHandle {
        FileHandle(FileHandleInner::Entry {
            parent_cluster: self.parent_clus,
            entry_offset_within_cluster: self.offset_within_cluster,
        })
    }
}

#[cfg(feature = "lfn")]
#[derive(Debug)]
pub struct LfnEntry {}

pub enum Fat {
    Fat32(Fat32),
}

impl Fat {
    pub fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> io::Result<Option<u32>> {
        match self {
            Self::Fat32(fat32) => fat32.next_cluster(reader, cluster),
        }
    }
}

pub struct Fat32 {
    start: usize,
    size: usize,
    count: usize,
}

impl Fat32 {
    const ENTRY_MASK: u32 = 0x0FFF_FFFF;
    const END_OF_CLUSTER: u32 = 0xFFFF_FFFF;

    pub fn new(start: usize, size: usize, count: usize) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self { start, size, count }
    }

    fn entry_offset(&self, cluster: usize) -> usize {
        debug_assert!(cluster * size_of::<u32>() < self.size);
        self.start + cluster * size_of::<u32>()
    }

    fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> io::Result<u32> {
        reader.seek(SeekFrom::Start(self.entry_offset(cluster) as u64))?;
        let mut data = 0u32;
        reader.read_exact(bytemuck::bytes_of_mut(&mut data))?;
        Ok(data)
    }

    pub fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> io::Result<Option<u32>> {
        let next = self.read_clus(reader, cluster)?;
        std::dbg!(cluster, next);
        if next == Self::END_OF_CLUSTER {
            return Ok(None);
        }

        Ok(Some(next & Self::ENTRY_MASK))
    }
}

/// The RawBpb struct represents the boot sector of any FAT partition
///
/// This only contains the common fields of the boot sector, and is not meant to be used directly
/// for reading or writing to the boot sector, for that, see [`RawBootSector`], which contains
/// the boot sector and the extended boot sector
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RawBpb {
    /// BS_jmpBoot
    pub jump: [u8; 3],
    /// BS_OEMName
    /// The name of the program that formatted the partition
    pub oem_name: [u8; 8],
    /// BPB_BytsPerSec
    /// The number of bytes per sector
    pub bytes_per_sector: U16<LittleEndian>,
    /// BPB_SecPerClus
    /// The number of sectors per cluster
    pub sectors_per_cluster: u8,
    /// BPB_RsvdSecCnt
    ///
    /// The number of reserved sectors, should be nonzero, ans should be a multiple of the sectors per cluster
    /// This is used to:
    /// 1. align the start of the filesystem to the sectors per cluster
    /// 2. Move the data (cluster 2) to the end of fat tables, so that the data can be read from the start of the filesystem
    pub reserved_sector_count: U16<LittleEndian>,
    /// BPB_NumFATs
    ///
    /// The number of fats, 1 is acceptable, but 2 is recommended
    pub fat_count: u8,
    /// BPB_RootEntCnt
    ///
    /// The number of root directory entries
    /// For FAT32, this should be 0
    /// For FAT12/16, this value multiplied by 32 should be a multiple of the bytes per sector
    /// For FAT16, it is recommended to set this to 512 for maximum compatibility
    pub root_entry_count: [u8; 2],
    /// BPB_TotSec16
    ///
    /// The number of sectors
    /// For FAT32, this should be 0
    /// For FAT16, if the number of sectors is greater than 0x10000, you should use total_sectors_32
    pub total_sectors_16: [u8; 2],
    /// BPB_Media
    ///
    /// See the MediaType enum for more information
    pub media_type: u8,
    /// BPB_FATSz16
    ///
    /// The number of sectors per fat
    /// For FAT32, this should be 0
    pub sectors_per_fat_16: [u8; 2],
    /// BPB_SecPerTrk
    ///
    /// The number of sectors per track
    /// This is only relevant for media with have a geometry and used by BIOS interrupt 0x13
    pub sectors_per_track: [u8; 2],
    /// BPB_NumHeads
    ///
    /// Similar situation as sectors_per_track
    pub num_heads: [u8; 2],
    /// BPB_HiddSec
    ///
    /// The number of hidden sectors predicing the partition that contains the FAT volume.
    /// This must be 0 on media that isn't partitioned
    pub hidden_sector_count: [u8; 4],
    /// BPB_TotSec32
    ///
    /// The total number of sectors for FAT32
    /// For FAT16 use, see total_sectors_16
    pub total_sectors_32: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RawBpbExt16 {
    /// BS_DrvNum
    pub drive_number: u8,
    /// BS_Reserved1
    pub reserved1: u8,
    /// BS_BootSig
    ///
    /// The extended boot signature, should be 0x29
    pub ext_boot_signature: u8,
    /// BS_VolID
    ///
    /// Volumme Serial Number
    /// This ID should be unique for each volume
    pub volume_id: [u8; 4],
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: [u8; 11],
    /// BS_FilSysType
    ///
    /// Must be set to the strings "FAT12   ","FAT16   ", or "FAT     "
    pub fs_type: [u8; 8],
    /// Zeros
    /// To make it compatible with bytemuck, instead of using [u8; 448], we use 256 + 128 + 64
    //pub padding1: [u8; 448],
    pub padding1_1: [u8; 256],
    pub padding1_2: [u8; 128],
    pub padding1_3: [u8; 64],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: [u8; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RawBpbExt32 {
    /// BPB_FatSz32
    ///
    /// The number of sectors per fat
    /// BPB_FATSz16 must be 0
    pub sectors_per_fat_32: U32<LittleEndian>,
    /// BPB_ExtFlags
    ///
    /// See the BpbExt32Flags struct for more information
    pub ext_flags: [u8; 2],
    /// BPB_FSVer
    ///
    /// The version of the file system
    /// This must be set to 0x00
    pub version: [u8; 2],
    /// BPB_RootClus
    ///
    /// The cluster number of the root directory
    /// This should be 2, or the first usable (not bad) cluster usable
    pub root_cluster: U32<LittleEndian>,
    /// BPB_FSInfo
    ///
    /// The sector number of the FSINFO structure
    /// NOTE: There is a copy of the FSINFO structure in the
    /// sequence of backup boot sectors, but only the copy
    /// pointed to by this field is kept up to date (i.e., both the
    /// primary and backup boot record point to the same
    /// FSINFO sector)
    pub fs_info_sector: U16<LittleEndian>,
    /// BPB_BkBootSec
    ///
    /// The sector number of the backup boot sector
    /// If set to 6 (only valid non-zero value), the boot sector
    /// in the reserved area is used to store the backup boot sector
    pub boot_sector: [u8; 2],
    /// BPB_Reserved
    /// Reserved, should be zero
    pub reserved: [u8; 12],
    /// BS_DrvNum
    ///
    /// The BIOS interrupt 0x13 drive number
    /// Should be 0x80 or 0x00
    pub drive_number: u8,
    /// BS_Reserved1
    /// Reserved, should be zero
    pub reserved1: u8,
    /// BS_BootSig
    ///
    /// The extended boot signature, should be 0x29
    pub ext_boot_signature: u8,
    /// BS_VolID
    ///
    /// Volumme Serial Number
    /// This ID should be unique for each volume
    pub volume_id: [u8; 4],
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: [u8; 11],
    /// BS_FilSysType
    ///
    /// Must be set to the string "FAT32   "
    pub fs_type: [u8; 8],
    /// Zeros
    ///
    /// To make it compatible with bytemuck, instead of using [u8; 420], we use 256 + 128 + 32 + 4
    /// pub padding1: [u8; 420],
    pub padding1_1: [u8; 256],
    pub padding1_2: [u8; 128],
    pub padding1_3: [u8; 32],
    pub padding1_4: [u8; 4],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: U16<LittleEndian>,
}

/// BPB_ExtFlags
///
/// This is a union of the flags that are set in the BPB_ExtFlags field
/// The flags are the following:
/// bits 0-3: zero based index of the active FAT, mirroring must be disabled
/// bits 4-6: reserved
/// bit 7: FAT mirroring is enabled
/// bits 8-15: reserved
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpbExt32Flags(u16);

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFileEntry {
    /// DIR_Name
    ///
    /// The name of the file, padded with spaces, and in the 8.3 format
    /// A value of 0xE5 indicates that the directory is free. For kanji, 0x05 is used instead of 0xE5
    /// The special value 0x00 also indicates that the directory is free, but also all the entries
    /// following it are free
    /// The name cannot start with a space
    /// Only upper case letters, digits, and the following characters are allowed:
    /// $ % ' - _ @ ~ ` ! ( ) { } ^ # &
    pub name: [u8; 11],
    /// DIR_Attr
    ///
    /// The file attributes
    pub attributes: u8,
    /// DIR_NTRes
    ///
    /// Reserved for use by Windows NT
    pub reserved: u8,
    /// DIR_CrtTimeTenth
    ///
    /// The creation time, in tenths of a second
    pub creation_time_tenth: u8,
    /// DIR_CrtTime
    ///
    /// The creation time, granularity is 2 seconds
    pub creation_time: [u8; 2],
    /// DIR_CrtDate
    ///
    /// The creation date
    pub creation_date: [u8; 2],
    /// DIR_LstAccDate
    ///
    /// The last access date
    pub last_access_date: [u8; 2],
    /// DIR_FstClusHI
    ///
    /// The high word of the first cluster number
    pub first_cluster_high: U16<LittleEndian>,
    /// DIR_WrtTime
    ///
    /// The last write time, granularity is 2 seconds
    pub last_write_time: [u8; 2],
    /// DIR_WrtDate
    ///
    /// The last write date
    pub last_write_date: [u8; 2],
    /// DIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: U16<LittleEndian>,
    /// DIR_FileSize
    ///
    /// The size of the file, in bytes
    pub size: U32<LittleEndian>,
}

unsafe impl bytemuck::NoUninit for RawFileEntry {}
unsafe impl bytemuck::Zeroable for RawFileEntry {}
unsafe impl bytemuck::AnyBitPattern for RawFileEntry {}

/// A long file name entry
/// The maximum length of a long file name is 255 characters, not including the null terminator
/// The characters allowed extend these characters:
///  . + , ; = [ ]
/// Embedded paces are also allowed
/// The name is stored in UTF-16 encoding (UNICODE)
/// When the unicode character cannot be translated to ANSI, an underscore is used
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawLfnEntry {
    /// LFN_Ord
    ///
    /// The order of the LFN entry, the contents must be masked with 0x40 for the last entry
    pub sequence_number: u8,
    /// LFN_Name1
    ///
    /// The first part of the long file name
    pub name1: [u8; 10],
    /// LDIR_Attr
    ///
    /// Attributes, must be set to: ATTR_LONG_NAME, which is:
    /// ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID
    pub attributes: u8,
    /// LFN_Type
    ///
    /// The type of the LFN entry, must be set to 0
    pub ty: u8,
    /// LFN_Chksum
    ///
    /// Checksum of name in the associated short name directory entry at the end of the LFN sequence
    /// THe algorithm described in the FAT spec is:
    /// unsigned char ChkSum (unsigned char \*pFcbName)
    /// {
    ///     short FcbNameLen;
    ///     unsigned char Sum;
    ///     Sum = 0;
    ///     for (FcbNameLen=11; FcbNameLen!=0; FcbNameLen--) {
    ///         // NOTE: The operation is an unsigned char rotate right
    ///         Sum = ((Sum & 1) ? 0x80 : 0) + (Sum >> 1) + *pFcbName++;
    ///     }
    ///     return (Sum);
    /// }
    pub checksum: u8,
    /// LFN_Name2
    ///
    /// The second part of the long file name
    pub name2: [u8; 12],
    /// LDIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: [u8; 2],
    /// LFN_Name3
    ///
    /// The third part of the long file name
    pub name3: [u8; 4],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub union RawDirectoryEntry {
    pub file: RawFileEntry,
    #[cfg(feature = "lfn")]
    pub lfn: RawLfnEntry,
    pub bytes: [u8; 32],
}

impl RawDirectoryEntry {
    pub fn attributes(&self) -> u8 {
        unsafe { self.file }.attributes
    }
}

#[cfg(feature = "lfn")]
#[doc(hidden)]
/// A helper module for implementation of lfn Bytemuck features
mod _lfn {
    use super::RawDirectoryEntry;
    unsafe impl bytemuck::NoUninit for RawDirectoryEntry {}
    unsafe impl bytemuck::Zeroable for RawDirectoryEntry {}
    unsafe impl bytemuck::AnyBitPattern for RawDirectoryEntry {}
}

#[repr(C, packed)]
#[derive(Clone, Copy, bytemuck::NoUninit, bytemuck::AnyBitPattern)]
pub struct RawFsInfo {
    /// FSI_LeadSig
    ///
    /// The lead signature, this have to be 0x41615252, or 'RRaA'
    pub signature: [u8; 4],
    /// FSI_Reserved1
    pub reserved1_1: [u8; 256],
    pub reserved1_2: [u8; 128],
    pub reserved1_3: [u8; 64],
    pub reserved1_4: [u8; 32],
    /// FSI_StrucSig
    ///
    /// The structure signature, this have to be 0x61417272, or 'rrAa'
    pub structure_signature: [u8; 4],
    /// FSI_Free_Count
    ///
    /// The number of free clusters, this have to be bigger than 0, and less than or equal to the
    /// total number of clusters
    /// This should remove any used clusters for headers, FAT tables, etc...
    pub free_count: U32<LittleEndian>,
    /// FSI_Nxt_Free
    ///
    /// The next free cluster number, this have to be bigger than 2, and less than or equal to the
    pub next_free: U32<LittleEndian>,
    /// FSI_Reserved2
    pub reserved2: [u8; 12],
    /// FSI_TrailSig
    ///
    /// The trail signature, this have to be 0xAA550000
    pub trail_signature: U32<LittleEndian>,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DirEntryAttrFlags: u8 {
        const READ_ONLY = 1 << 0;
        const HIDDEN = 1 << 1;
        const SYSTEM = 1 << 2;
        const VOLUME_ID = 1 << 3;
        const DIRECTORY = 1 << 4;
        const ARCHIVE = 1 << 5;
    }
}

impl DirEntryAttrFlags {
    pub const LONG_NAME: Self = Self::from_bits_truncate(
        Self::READ_ONLY.bits() | Self::HIDDEN.bits() | Self::SYSTEM.bits() | Self::VOLUME_ID.bits(),
    );
}

/// A type representing a short filename
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShortFileName {
    bytes: [u8; 12],
}

impl fmt::Debug for ShortFileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ShortFileName")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("disallowed characters in short file name")]
pub struct CreateShortFileNameError;

impl ShortFileName {
    pub const ALLOWED_SYMBOLS: &'static [u8] = b"$%'-_@~`!(){}^#&";

    pub fn new(bytes: [u8; 11]) -> Result<Self, CreateShortFileNameError> {
        for byte in &bytes {
            if byte.is_ascii_uppercase()
                || Self::ALLOWED_SYMBOLS.contains(byte)
                || byte.is_ascii_digit()
                || *byte == b' '
                || *byte > 127
            {
                continue;
            }
            return Err(CreateShortFileNameError);
        }
        let mut local_bytes = [0u8; 12];
        local_bytes[0..8].copy_from_slice(&bytes[0..8]);
        local_bytes[8] = b'.';
        local_bytes[9..12].copy_from_slice(&bytes[8..11]);
        Ok(Self { bytes: local_bytes })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes).expect("ShortFileName is not UTF-8")
    }
}

impl Index<usize> for ShortFileName {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.bytes[index]
    }
}

impl IndexMut<usize> for ShortFileName {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.bytes[index]
    }
}

impl AsRef<[u8]> for ShortFileName {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
