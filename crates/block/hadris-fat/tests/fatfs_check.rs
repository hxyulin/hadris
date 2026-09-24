//! `check` on clean volumes, on hand-corrupted ones, on volumes left by
//! interrupted operations, and against the host `fsck.fat -n` verdict.

#[path = "common/fatfs.rs"]
mod common;

use std::path::Path;
use std::process::Command;

use common::{CASES, Found, block_on, check_dev};
use hadris_fat::sync::{FatFs, check, format};
use hadris_fat::{Detail, FatKind, FormatOptions, MountOptions, VolumeLabel};
use hadris_fs::sync::{FileSystem, FsDriver, PathExt, Volume};
use hadris_fs::{
    CheckReport, ErrorKind, HeapTable, Location, Name, NewNode, RemoveKind, RenameFlags,
    SetMetadata, Severity,
};
use hadris_io::Error;
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

type Device = MemDevice<Vec<u8>>;

const KINDS: [(FatKind, u64); 3] = [
    (FatKind::Fat12, 2 << 20),
    (FatKind::Fat16, 16 << 20),
    (FatKind::Fat32, 40 << 20),
];

fn device(image: Vec<u8>) -> Device {
    MemDevice::new(image, BlockSize::new(512).unwrap())
}

fn mount(image: Vec<u8>) -> FatFs<Device, HeapTable> {
    FatFs::open_with(
        device(image),
        MountOptions::new().with_table(HeapTable::new()),
    )
    .unwrap()
}

/// Every finding, with a cluster bitmap of `bitmap` bytes.
fn findings_with(image: &[u8], bitmap: usize) -> (CheckReport, Vec<Found>) {
    check_dev(&mut device(image.to_vec()), 1024 + bitmap)
}

fn findings(image: &[u8]) -> Vec<Found> {
    findings_with(image, 1 << 16).1
}

fn kinds(found: &[Found]) -> Vec<Detail> {
    let mut kinds: Vec<_> = found.iter().map(|found| found.detail).collect();
    kinds.sort_by_key(|kind| format!("{kind:?}"));
    kinds.dedup();
    kinds
}

fn sorted(found: &[Found]) -> Vec<String> {
    let mut all: Vec<_> = found.iter().map(|finding| format!("{finding:?}")).collect();
    all.sort();
    all
}

/// The layout of an image, read from its boot sector.
#[derive(Debug, Clone, Copy)]
struct Geo {
    kind: FatKind,
    sector: u64,
    cluster: u64,
    fat_start: u64,
    fat_size: u64,
    fats: u8,
    root_start: u64,
    root_cluster: u32,
    data_start: u64,
    max: u32,
}

fn u16_at(img: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([img[at], img[at + 1]])
}

fn u32_at(img: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(img[at..at + 4].try_into().unwrap())
}

fn geo(img: &[u8]) -> Geo {
    let sector = u16_at(img, 11) as u64;
    let cluster = img[13] as u64 * sector;
    let reserved = u16_at(img, 14) as u64;
    let fats = img[16];
    let root_entries = u16_at(img, 17) as u64;
    let total = match u16_at(img, 19) {
        0 => u32_at(img, 32) as u64,
        n => n as u64,
    };
    let fat_sectors = match u16_at(img, 22) {
        0 => u32_at(img, 36) as u64,
        n => n as u64,
    };
    let root_sectors = (root_entries * 32).div_ceil(sector);
    let data_sector = reserved + fats as u64 * fat_sectors + root_sectors;
    let clusters = (total - data_sector) / img[13] as u64;
    let kind = if root_entries == 0 {
        FatKind::Fat32
    } else if clusters < 4085 {
        FatKind::Fat12
    } else {
        FatKind::Fat16
    };
    Geo {
        kind,
        sector,
        cluster,
        fat_start: reserved * sector,
        fat_size: fat_sectors * sector,
        fats,
        root_start: (reserved + fats as u64 * fat_sectors) * sector,
        root_cluster: if kind == FatKind::Fat32 {
            u32_at(img, 44)
        } else {
            0
        },
        data_start: data_sector * sector,
        max: clusters as u32 + 1,
    }
}

impl Geo {
    fn offset(&self, cluster: u32) -> usize {
        (self.data_start + (cluster as u64 - 2) * self.cluster) as usize
    }

    fn eoc(&self) -> u32 {
        match self.kind {
            FatKind::Fat12 => 0xFFF,
            FatKind::Fat16 => 0xFFFF,
            _ => 0x0FFF_FFFF,
        }
    }

    fn bad(&self) -> u32 {
        self.eoc() - 8
    }

    fn fat_get(&self, img: &[u8], copy: u8, cluster: u32) -> u32 {
        let base = (self.fat_start + copy as u64 * self.fat_size) as usize;
        match self.kind {
            FatKind::Fat12 => {
                let at = base + cluster as usize * 3 / 2;
                let pair = u16_at(img, at) as u32;
                if cluster % 2 == 0 {
                    pair & 0xFFF
                } else {
                    pair >> 4
                }
            }
            FatKind::Fat16 => u16_at(img, base + cluster as usize * 2) as u32,
            _ => u32_at(img, base + cluster as usize * 4) & 0x0FFF_FFFF,
        }
    }

    fn fat_set(&self, img: &mut [u8], copy: u8, cluster: u32, value: u32) {
        let base = (self.fat_start + copy as u64 * self.fat_size) as usize;
        match self.kind {
            FatKind::Fat12 => {
                let at = base + cluster as usize * 3 / 2;
                let pair = u16_at(img, at);
                let pair = if cluster % 2 == 0 {
                    (pair & 0xF000) | value as u16
                } else {
                    (pair & 0x000F) | ((value as u16) << 4)
                };
                img[at..at + 2].copy_from_slice(&pair.to_le_bytes());
            }
            FatKind::Fat16 => {
                let at = base + cluster as usize * 2;
                img[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
            }
            _ => {
                let at = base + cluster as usize * 4;
                img[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
    }

    fn fat_set_all(&self, img: &mut [u8], cluster: u32, value: u32) {
        for copy in 0..self.fats {
            self.fat_set(img, copy, cluster, value);
        }
    }

    fn chain(&self, img: &[u8], first: u32) -> Vec<u32> {
        let mut chain = vec![first];
        loop {
            let next = self.fat_get(img, 0, *chain.last().unwrap());
            if next >= self.eoc() - 7 {
                return chain;
            }
            chain.push(next);
        }
    }

    fn first(&self, img: &[u8], entry: usize) -> u32 {
        let high = if self.kind == FatKind::Fat32 {
            u16_at(img, entry + 20) as u32
        } else {
            0
        };
        (high << 16) | u16_at(img, entry + 26) as u32
    }

    fn set_first(&self, img: &mut [u8], entry: usize, cluster: u32) {
        img[entry + 26..entry + 28].copy_from_slice(&(cluster as u16).to_le_bytes());
        if self.kind == FatKind::Fat32 {
            img[entry + 20..entry + 22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        }
    }

    /// The first unused slot of the directory at `cluster`, or of the root.
    fn end_slot(&self, img: &[u8], cluster: Option<u32>) -> usize {
        let start = match cluster {
            Some(cluster) => self.offset(cluster),
            None if self.kind == FatKind::Fat32 => self.offset(self.root_cluster),
            None => self.root_start as usize,
        };
        (start..).step_by(32).find(|&at| img[at] == 0).unwrap()
    }

    fn fs_info(&self, img: &[u8]) -> usize {
        u16_at(img, 48) as usize * self.sector as usize
    }
}

/// The offset of the short entry named `name` in stored form.
fn entry(img: &[u8], name: &[u8; 11]) -> usize {
    let found: Vec<_> = (0..img.len())
        .step_by(32)
        .filter(|&at| &img[at..at + 11] == name && img[at + 11] & 0x3F != 0x0F)
        .collect();
    assert_eq!(found.len(), 1, "{}", String::from_utf8_lossy(name));
    found[0]
}

fn short_entry(name: &[u8; 11], attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut raw = [0u8; 32];
    raw[..11].copy_from_slice(name);
    raw[11] = attr;
    raw[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    raw[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    raw[28..32].copy_from_slice(&size.to_le_bytes());
    raw
}

/// A volume formatted and filled through `FatFs`: `A.BIN` of three
/// clusters, `B.BIN` of two, `C.BIN` of one, a long-named file, an empty
/// file, and `SUB` holding `D.BIN` and `DEEP/E.BIN`.
fn fixture(kind: FatKind, size: u64) -> Vec<u8> {
    let options = FormatOptions::new()
        .with_kind(kind)
        .with_label(VolumeLabel::new("CHECK").unwrap());
    let fs = format(
        MemDevice::new(vec![0; size as usize], BlockSize::new(512).unwrap()),
        options,
    )
    .unwrap();
    let vol = Volume::new(fs);
    let cluster = vol.stats().unwrap().block_size() as usize;
    vol.write_file("/A.BIN", &common::payload(3 * cluster - 10, 1))
        .unwrap();
    vol.write_file("/B.BIN", &common::payload(2 * cluster, 2))
        .unwrap();
    vol.write_file("/C.BIN", &common::payload(10, 3)).unwrap();
    vol.write_file("/Long name file.txt", b"long").unwrap();
    vol.write_file("/EMPTY.DAT", b"").unwrap();
    vol.create_dir_all("/SUB/DEEP").unwrap();
    vol.write_file("/SUB/D.BIN", &common::payload(cluster + 1, 4))
        .unwrap();
    vol.write_file("/SUB/DEEP/E.BIN", b"deep").unwrap();
    vol.sync().unwrap();
    vol.into_inner().into_inner().into_inner()
}

const A: &[u8; 11] = b"A       BIN";
const B: &[u8; 11] = b"B       BIN";
const C: &[u8; 11] = b"C       BIN";

const LONG: &[u8; 11] = b"LONGNA~1TXT";
const SUB: &[u8; 11] = b"SUB        ";
const DEEP: &[u8; 11] = b"DEEP       ";

/// A finding that must be among those reported: its detail, location and
/// path.
type Exact = (Detail, Location, Option<&'static str>);

/// One corruption: its name, the image, and the finding details it must
/// produce, nothing else.
struct Corruption {
    name: &'static str,
    image: Vec<u8>,
    expected: Vec<Detail>,
    exact: Option<Exact>,
}

fn corruptions(kind: FatKind, size: u64) -> Vec<Corruption> {
    use Detail as K;
    use Location::{Byte, Cluster};
    let clean = fixture(kind, size);
    let g = geo(&clean);
    let fat32 = kind == FatKind::Fat32;
    let a = entry(&clean, A);
    let b = entry(&clean, B);
    let c = entry(&clean, C);
    let long = entry(&clean, LONG);
    let sub = entry(&clean, SUB);
    let deep = entry(&clean, DEEP);
    let a_chain = g.chain(&clean, g.first(&clean, a));
    let b_chain = g.chain(&clean, g.first(&clean, b));
    let sub_cluster = g.first(&clean, sub);
    let deep_cluster = g.first(&clean, deep);
    assert_eq!((a_chain.len(), b_chain.len()), (3, 2));
    let mut out = Vec::new();
    let mut add = |name, change: &dyn Fn(&mut Vec<u8>), mut expected: Vec<Detail>, exact| {
        let mut image = clean.clone();
        change(&mut image);
        expected.sort_by_key(|kind| format!("{kind:?}"));
        out.push(Corruption {
            name,
            image,
            expected,
            exact,
        });
    };

    let media = |img: &mut Vec<u8>| {
        img[21] = 0x12;
        for copy in 0..g.fats {
            let value = (g.fat_get(img, copy, 0) & !0xFF) | 0x12;
            g.fat_set(img, copy, 0, value);
        }
    };
    let mut expected = vec![K::BootSector];
    if fat32 {
        expected.push(K::BackupBootSector);
    }
    add(
        "media descriptor",
        &media,
        expected,
        Some((K::BootSector, Byte(0), None)),
    );
    add(
        "reserved FAT entries",
        &|img| g.fat_set_all(img, 1, 0x12),
        vec![K::ReservedEntries],
        Some((K::ReservedEntries, Byte(g.fat_start), None)),
    );
    if fat32 {
        add(
            "backup boot sector",
            &|img| img[6 * 512 + 3] ^= 1,
            vec![K::BackupBootSector],
            Some((K::BackupBootSector, Byte(6 * 512), None)),
        );
        add(
            "FSInfo signature",
            &|img| {
                let at = g.fs_info(img);
                img[at] ^= 1;
            },
            vec![K::FsInfo],
            Some((K::FsInfo, Byte(g.fs_info(&clean) as u64), None)),
        );
        let recorded = u32_at(&clean, g.fs_info(&clean) + 488);
        add(
            "FSInfo free count",
            &|img| {
                let at = g.fs_info(img) + 488;
                img[at..at + 4].copy_from_slice(&(recorded + 5).to_le_bytes());
            },
            vec![K::FreeCount],
            Some((K::FreeCount, Byte(g.fs_info(&clean) as u64 + 488), None)),
        );
    }
    add(
        "FAT copy mismatch",
        &|img| g.fat_set(img, 1, a_chain[1], 1),
        vec![K::FatCopiesDiffer],
        Some((K::FatCopiesDiffer, Cluster(a_chain[1] as u64), None)),
    );
    add(
        "cross-linked chains",
        &|img| g.set_first(img, b, a_chain[0]),
        vec![K::CrossLink, K::SizeMismatch, K::LostClusters],
        Some((K::CrossLink, Cluster(a_chain[1] as u64), Some("/B.BIN"))),
    );
    let mut expected = vec![K::LostClusters];
    if fat32 {
        expected.push(K::FreeCount);
    }
    add(
        "lost cluster",
        &|img| g.fat_set_all(img, g.max, g.eoc()),
        expected,
        Some((K::LostClusters, Cluster(g.max as u64), None)),
    );
    add(
        "chain longer than size",
        &|img| img[a + 28..a + 32].copy_from_slice(&1u32.to_le_bytes()),
        vec![K::SizeMismatch],
        Some((K::SizeMismatch, Byte(a as u64), Some("/A.BIN"))),
    );
    let too_big = 5 * g.cluster as u32;
    add(
        "chain shorter than size",
        &|img| img[b + 28..b + 32].copy_from_slice(&too_big.to_le_bytes()),
        vec![K::SizeMismatch],
        Some((K::SizeMismatch, Byte(b as u64), Some("/B.BIN"))),
    );
    add(
        "first cluster out of range",
        &|img| g.set_first(img, c, g.max + 1),
        vec![K::InvalidCluster, K::LostClusters],
        Some((K::InvalidCluster, Cluster(g.max as u64 + 1), Some("/C.BIN"))),
    );
    add(
        "broken link",
        &|img| g.fat_set_all(img, a_chain[1], 1),
        vec![K::BrokenChain, K::LostClusters],
        Some((K::BrokenChain, Cluster(a_chain[1] as u64), Some("/A.BIN"))),
    );
    add(
        "cyclic chain",
        &|img| g.fat_set_all(img, a_chain[2], a_chain[0]),
        vec![K::CyclicChain],
        Some((K::CyclicChain, Cluster(a_chain[2] as u64), Some("/A.BIN"))),
    );
    add(
        "cycle into the middle",
        &|img| g.fat_set_all(img, a_chain[2], a_chain[1]),
        vec![K::CyclicChain],
        Some((K::CyclicChain, Cluster(a_chain[2] as u64), Some("/A.BIN"))),
    );
    add(
        "bad cluster in a chain",
        &|img| g.fat_set_all(img, a_chain[1], g.bad()),
        vec![K::BadCluster, K::LostClusters],
        Some((K::BadCluster, Cluster(a_chain[1] as u64), Some("/A.BIN"))),
    );
    add(
        "wrong dot-dot",
        &|img| {
            let at = g.offset(sub_cluster) + 32;
            g.set_first(img, at, deep_cluster);
        },
        vec![K::DotEntries, K::LostClusters],
        Some((
            K::DotEntries,
            Byte(g.offset(sub_cluster) as u64),
            Some("/SUB"),
        )),
    );
    add(
        "wrong dot",
        &|img| {
            let at = g.offset(sub_cluster);
            g.set_first(img, at, deep_cluster);
        },
        vec![K::DotEntries],
        Some((
            K::DotEntries,
            Byte(g.offset(sub_cluster) as u64),
            Some("/SUB"),
        )),
    );
    add(
        "stray dot entry",
        &|img| {
            let at = g.end_slot(img, None);
            img[at..at + 32].copy_from_slice(&short_entry(b".          ", 0x10, 0, 0));
        },
        vec![K::DotEntries],
        None,
    );
    add(
        "invalid short name",
        &|img| img[a + 1] = b'*',
        vec![K::BadName],
        Some((K::BadName, Byte(a as u64), Some("/A*.BIN"))),
    );
    add(
        "long name checksum",
        &|img| img[long + 10] = b'X',
        vec![K::LfnChecksum],
        Some((
            K::LfnChecksum,
            Byte(long as u64 - 64),
            Some("/LONGNA~1.TXX"),
        )),
    );
    add(
        "long name without its short entry",
        &|img| img[long] = 0xE5,
        vec![K::OrphanLfn, K::LostClusters],
        Some((K::OrphanLfn, Byte(long as u64 - 64), Some("/"))),
    );
    add(
        "stray long-name fragment",
        &|img| {
            let at = g.end_slot(img, Some(sub_cluster));
            img[at] = 0x02;
            img[at + 1] = b'x';
            img[at + 11] = 0x0F;
        },
        vec![K::OrphanLfn],
        None,
    );
    add(
        "directory with a size",
        &|img| img[sub + 28] = 1,
        vec![K::DirectorySize],
        Some((K::DirectorySize, Byte(sub as u64), Some("/SUB"))),
    );
    add(
        "label in a subdirectory",
        &|img| {
            let at = g.end_slot(img, Some(sub_cluster));
            img[at..at + 32].copy_from_slice(&short_entry(b"LABEL      ", 0x08, 0, 0));
        },
        vec![K::Label],
        None,
    );
    add(
        "directory named twice",
        &|img| {
            let at = g.end_slot(img, None);
            img[at..at + 32].copy_from_slice(&short_entry(b"SUB2       ", 0x10, sub_cluster, 0));
        },
        vec![K::CrossLink],
        None,
    );
    add(
        "directory linked into its own subtree",
        &|img| {
            let at = g.end_slot(img, Some(deep_cluster));
            img[at..at + 32].copy_from_slice(&short_entry(b"LOOP       ", 0x10, sub_cluster, 0));
        },
        vec![K::CrossLink, K::DotEntries],
        None,
    );
    if fat32 {
        add(
            "directory at the root cluster",
            &|img| g.set_first(img, deep, g.root_cluster),
            vec![K::InvalidCluster, K::LostClusters],
            Some((
                K::InvalidCluster,
                Cluster(g.root_cluster as u64),
                Some("/SUB/DEEP"),
            )),
        );
    }
    out
}

/// What `fsck.fat -n` says about an image: `Some(true)` when clean, `None`
/// when it is not installed.
fn host_clean(image: &[u8]) -> Option<bool> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.img");
    std::fs::write(&path, image).unwrap();
    host_run(&path).map(|status| status.success())
}

fn host_run(path: &Path) -> Option<std::process::ExitStatus> {
    Command::new("fsck.fat")
        .args(["-n"])
        .arg(path)
        .output()
        .ok()
        .map(|out| out.status)
}

#[test]
fn clean_volumes_report_nothing() {
    for (kind, size) in KINDS {
        let image = fixture(kind, size);
        let (report, found) = findings_with(&image, 1 << 16);
        assert_eq!(found, [], "{kind:?}");
        assert!(report.is_clean());
        assert_eq!(report.passes(), 1);
        assert_ne!(host_clean(&image), Some(false), "{kind:?}");
    }
}

#[test]
fn clean_images_from_other_writers_report_nothing() {
    for case in CASES {
        let image = common::build(case);
        let (_, found) = check_dev(&mut common::device(case, image), 1024 + 512);
        assert_eq!(found, [], "{}", case.name);
        let blank = common::blank(case);
        let (report, _) = check_dev(&mut common::device(case, blank), 4096);
        assert!(report.is_clean(), "{}", case.name);
    }
    for bits in ["12", "16", "32"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mkfs.img");
        let size_kib = if bits == "32" { "40960" } else { "16384" };
        let made = Command::new("mkfs.fat")
            .args(["-F", bits, "-C"])
            .arg(&path)
            .arg(size_kib)
            .output();
        let Ok(made) = made else {
            continue;
        };
        assert!(made.status.success());
        let image = std::fs::read(&path).unwrap();
        assert_eq!(findings(&image), [], "mkfs.fat -F {bits}");
    }
}

#[test]
fn corruptions_report_their_findings() {
    let mut disagreements = Vec::new();
    for (kind, size) in KINDS {
        for case in corruptions(kind, size) {
            let context = format!("{kind:?} {}", case.name);
            let found = findings(&case.image);
            assert_eq!(kinds(&found), case.expected, "{context}: {found:?}");
            if let Some((detail, location, path)) = case.exact {
                assert!(
                    found.iter().any(|finding| finding.detail == detail
                        && finding.location == Some(location)
                        && finding.path.as_deref() == path),
                    "{context}: {:?} not in {found:?}",
                    case.exact
                );
            }
            assert_eq!(
                sorted(&findings_with(&case.image, 512).1),
                sorted(&found),
                "{context}: a one-byte bitmap"
            );
            if host_clean(&case.image) == Some(true) {
                disagreements.push(context);
            }
        }
    }
    eprintln!("fsck.fat -n passes: {disagreements:?}");
    // fsck.fat 4.2 prints the long-name and backup boot sector problems but
    // exits 0 with -n, and does not look at labels outside the root or at the
    // FAT12 reserved entries.
    let expected: Vec<String> = KINDS
        .iter()
        .flat_map(|(kind, _)| {
            let mut names = vec![
                "long name checksum",
                "stray long-name fragment",
                "label in a subdirectory",
            ];
            match kind {
                FatKind::Fat12 => names.push("reserved FAT entries"),
                FatKind::Fat32 => names.push("backup boot sector"),
                _ => {}
            }
            names
                .into_iter()
                .map(move |name| format!("{kind:?} {name}"))
        })
        .collect();
    if host_run(Path::new("/nonexistent")).is_some() {
        let mut expected = expected;
        expected.sort();
        disagreements.sort();
        assert_eq!(disagreements, expected, "images fsck.fat -n passes");
    }
}

#[test]
fn every_mode_reports_the_same() {
    let (kind, size) = KINDS[2];
    for case in corruptions(kind, size).into_iter().take(8) {
        let expected = sorted(&findings(&case.image));
        let from_async = block_on(async {
            let mut dev = device(case.image.clone());
            let mut found = Vec::new();
            let mut scratch = [0u8; 4096];
            hadris_fat::r#async::check(&mut dev, &mut scratch, |f| found.push(Found::new(f)))
                .await
                .unwrap();
            found
        });
        assert_eq!(sorted(&from_async), expected, "{}", case.name);
        let from_send = block_on(async {
            let mut dev = device(case.image.clone());
            let mut scratch = [0u8; 4096];
            let report = hadris_fat::async_send::check(&mut dev, &mut scratch, |_| {})
                .await
                .unwrap();
            fn is_send<T: Send>(_: &T) {}
            is_send(&hadris_fat::async_send::check(
                &mut dev,
                &mut scratch,
                |_| {},
            ));
            report
        });
        assert_eq!(
            from_send.findings() as usize,
            expected.len(),
            "{}",
            case.name
        );
    }
}

#[test]
fn short_scratch_is_refused() {
    let mut dev = device(fixture(FatKind::Fat12, 2 << 20));
    let err = check(&mut dev, &mut [0u8; 1535], |_| {}).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
    assert!(
        check(&mut dev, &mut [0u8; 1536], |_| {})
            .unwrap()
            .is_clean()
    );
}

#[test]
fn deep_trees_are_walked_without_a_stack() {
    let fs = format(
        MemDevice::new(vec![0; 40 << 20], BlockSize::new(512).unwrap()),
        FormatOptions::new().with_kind(FatKind::Fat32),
    )
    .unwrap();
    let vol = Volume::new(fs);
    let mut path = String::new();
    for depth in 0..40 {
        path.push_str(&format!("/level {depth}"));
        vol.create_dir_all(&path).unwrap();
        for file in 0..3 {
            vol.write_file(&format!("{path}/file {file}.txt"), path.as_bytes())
                .unwrap();
        }
    }
    vol.sync().unwrap();
    let image = vol.into_inner().into_inner().into_inner();
    let (report, found) = findings_with(&image, 512);
    assert_eq!(found, []);
    assert!(report.passes() > 10);
}

/// A device that fails writes with a device error after `budget` writes.
struct Faulty {
    inner: Device,
    budget: std::rc::Rc<std::cell::Cell<usize>>,
}

impl hadris_io::ErrorType for Faulty {
    type Error = std::io::Error;
}

impl BlockDevice for Faulty {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn writable(&self) -> bool {
        self.inner.writable()
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.inner
            .read_blocks(first, buf)
            .map_err(|err| err.map_device(|never| match never {}))
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        if self.budget.get() == 0 {
            return Err(fault());
        }
        self.budget.set(self.budget.get() - 1);
        self.inner
            .write_blocks(first, buf)
            .map_err(|err| err.map_device(|never| match never {}))
    }
}

fn fault() -> Error<std::io::Error> {
    Error::device(std::io::Error::other("injected fault"), "write failed")
}

fn name(text: &str) -> &Name {
    Name::new(text).unwrap()
}

/// Interrupting each operation after each of its writes leaves only the
/// leftovers the `FatFs` crash-safety rules allow, and a finished operation
/// followed by `sync` leaves a clean volume.
#[test]
fn interrupted_operations_leave_only_repairable_leftovers() {
    use Detail as K;
    let common_leftovers = [K::FatCopiesDiffer, K::FreeCount];
    for (kind, size) in [KINDS[0], KINDS[2]] {
        let before = fixture(kind, size);
        for op in 0..8 {
            let allowed: &[Detail] = match op {
                0 | 1 => &[K::LostClusters, K::OrphanLfn],
                2 | 3 => &[K::CrossLink, K::OrphanLfn, K::DotEntries, K::LostClusters],
                4 => &[K::CrossLink, K::OrphanLfn, K::LostClusters],
                _ => &[K::LostClusters, K::SizeMismatch],
            };
            let mut seen = Vec::new();
            for budget in 0.. {
                let left = std::rc::Rc::new(std::cell::Cell::new(budget));
                let dev = Faulty {
                    inner: device(before.clone()),
                    budget: left.clone(),
                };
                let mut fs =
                    FatFs::open_with(dev, MountOptions::new().with_table(HeapTable::<()>::new()))
                        .unwrap();
                let root = fs.root();
                let meta = SetMetadata::new();
                let result = match op {
                    0 => fs
                        .create(root, name("a new directory"), NewNode::Dir, &meta)
                        .map(|_| ()),
                    1 => fs.remove(root, name("Long name file.txt"), RemoveKind::Any),
                    2 => fs.rename(
                        root,
                        name("Long name file.txt"),
                        root,
                        name("Renamed file.txt"),
                        RenameFlags::empty(),
                    ),
                    3 => fs.resolve("/SUB").and_then(|sub| {
                        fs.rename(
                            sub,
                            name("DEEP"),
                            root,
                            name("Moved Deep"),
                            RenameFlags::empty(),
                        )
                    }),
                    4 => fs.rename(
                        root,
                        name("C.BIN"),
                        root,
                        name("B.BIN"),
                        RenameFlags::empty(),
                    ),
                    5 => fs
                        .resolve("/A.BIN")
                        .and_then(|node| fs.write_at(node, 50_000, &[3u8; 20_000]).map(|_| ())),
                    6 => fs.resolve("/A.BIN").and_then(|node| fs.set_len(node, 1)),
                    _ => fs
                        .resolve("/C.BIN")
                        .and_then(|node| fs.set_len(node, 30_000)),
                };
                let finished = result.is_ok();
                let context = format!("{kind:?} op {op} budget {budget}");
                if finished {
                    left.set(usize::MAX);
                    fs.sync().unwrap();
                }
                let image = fs.into_inner().inner.into_inner();
                let found = findings(&image);
                if finished {
                    assert_eq!(found, [], "{context}");
                    break;
                }
                for finding in &found {
                    let k = finding.detail;
                    assert!(
                        allowed.contains(&k) || common_leftovers.contains(&k),
                        "{context}: {finding:?}"
                    );
                }
                seen.extend(kinds(&found));
            }
            seen.sort_by_key(|kind| format!("{kind:?}"));
            seen.dedup();
            eprintln!("{kind:?} op {op}: {seen:?}");
        }
    }
}

#[test]
fn the_label_is_read_from_the_root() {
    let image = fixture(FatKind::Fat16, 16 << 20);
    let mut fs = mount(image.clone());
    assert_eq!(fs.label().unwrap().unwrap().as_str(), "CHECK");
    let mut odd = image;
    let at = entry(&odd, b"CHECK      ");
    odd[at + 1] = 0xFF;
    let label = mount(odd).label().unwrap().unwrap();
    assert_eq!(label.as_bytes()[..2], [b'C', 0xFF]);
    assert_eq!(label.as_str(), "");
    let mut blank = format(
        MemDevice::new(vec![0; 2 << 20], BlockSize::new(512).unwrap()),
        FormatOptions::new(),
    )
    .unwrap();
    assert_eq!(blank.label().unwrap(), None);
}

/// Writes an LFN fragment with `units` into the slot at `at`.
fn put_fragment(img: &mut [u8], at: usize, sequence: u8, checksum: u8, units: &[u16; 13]) {
    let mut raw = [0u8; 32];
    raw[0] = sequence;
    raw[11] = 0x0F;
    raw[13] = checksum;
    let fields = [1..11, 14..26, 28..32];
    let slots = fields.into_iter().flat_map(|range| range.step_by(2));
    for (pos, unit) in slots.zip(units) {
        raw[pos..pos + 2].copy_from_slice(&unit.to_le_bytes());
    }
    img[at..at + 32].copy_from_slice(&raw);
}

#[test]
fn overlong_long_name_runs_fall_back_to_the_short_name() {
    let long = "x".repeat(255);
    for (kind, size) in KINDS {
        let fs = format(
            MemDevice::new(vec![0; size as usize], BlockSize::new(512).unwrap()),
            FormatOptions::new().with_kind(kind),
        )
        .unwrap();
        let vol = Volume::new(fs);
        vol.write_file("/P", b"").unwrap();
        vol.write_file(&format!("/{long}"), b"data").unwrap();
        vol.sync().unwrap();
        let clean = vol.into_inner().into_inner().into_inner();
        let short = *b"XXXXXX~1   ";
        let sum = short
            .iter()
            .fold(0u8, |sum, &b| sum.rotate_right(1).wrapping_add(b));
        let fragments: Vec<usize> = (0..clean.len())
            .step_by(32)
            .filter(|&at| clean[at] != 0xE5 && clean[at + 11] == 0x0F && clean[at + 13] == sum)
            .collect();
        assert_eq!(fragments.len(), 20, "{kind:?}");
        let p = entry(&clean, b"P          ");

        let mut units = [0xFFFFu16; 13];
        units[0] = u16::from(b'a');
        units[1] = 0;
        let mut too_many = clean.clone();
        put_fragment(&mut too_many, p, 0x40 | 21, sum, &units);
        for (index, &at) in fragments.iter().enumerate() {
            put_fragment(&mut too_many, at, 20 - index as u8, sum, &units);
        }
        let mut too_long = clean.clone();
        let full = [u16::from(b'y'); 13];
        for (index, &at) in fragments.iter().enumerate() {
            let sequence = (20 - index as u8) | if index == 0 { 0x40 } else { 0 };
            put_fragment(&mut too_long, at, sequence, sum, &full);
        }

        let cases: [(&str, Vec<u8>, &[&str]); 2] = [
            ("21 fragments", too_many, &["XXXXXX~1"]),
            ("260 units", too_long, &["P", "XXXXXX~1"]),
        ];
        for (what, image, expected) in cases {
            let found = findings(&image);
            assert_eq!(kinds(&found), [Detail::OrphanLfn], "{kind:?} {what}");
            let vol = Volume::new(mount(image));
            let names: Vec<String> = vol
                .read_dir("/")
                .unwrap()
                .map(|entry| entry.unwrap().name().to_str().unwrap().to_owned())
                .collect();
            assert_eq!(names, expected, "{kind:?} {what}");
            assert_eq!(vol.read_to_vec("/XXXXXX~1").unwrap(), b"data");
        }
    }
}

#[test]
fn a_damaged_fat32_boot_sector_is_checked_from_the_backup() {
    let (kind, size) = KINDS[2];
    let mut image = fixture(kind, size);
    image[..512].fill(0);
    let found = findings(&image);
    assert_eq!(kinds(&found), [Detail::BootSector], "{found:?}");
    assert_eq!(found[0].location, Some(Location::Byte(0)));

    for (kind, size) in &KINDS[..2] {
        let mut image = fixture(*kind, *size);
        image[..512].fill(0);
        let err = check(&mut device(image), &mut [0u8; 4096], |_| {}).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotRecognized, "{kind:?}");
        assert_eq!(Detail::of(&err), Some(Detail::BootSector), "{kind:?}");
    }
}

#[test]
fn a_clear_clean_shutdown_bit_is_a_notice() {
    for (kind, size) in &KINDS[1..] {
        let clean = fixture(*kind, size.to_owned());
        let g = geo(&clean);
        let mut image = clean.clone();
        let bit = if *kind == FatKind::Fat16 {
            0x8000
        } else {
            0x0800_0000
        };
        let value = g.fat_get(&image, 0, 1) & !bit;
        g.fat_set_all(&mut image, 1, value);
        let found = findings(&image);
        assert_eq!(kinds(&found), [Detail::Dirty], "{kind:?}");
        assert_eq!(found[0].severity, Severity::Notice);
        assert_eq!(found[0].location, Some(Location::Byte(g.fat_start)));
    }
}

#[test]
fn findings_name_the_path_of_their_entry() {
    let fs = format(
        MemDevice::new(vec![0; 40 << 20], BlockSize::new(512).unwrap()),
        FormatOptions::new().with_kind(FatKind::Fat32),
    )
    .unwrap();
    let vol = Volume::new(fs);
    let segment = "d".repeat(100);
    let mut path = String::new();
    let mut dirs = Vec::new();
    for depth in 0..12 {
        path.push_str(&format!("/{depth:02}{segment}"));
        vol.create_dir_all(&path).unwrap();
        vol.write_file(&format!("{path}/\u{e9}t\u{e9}.txt"), b"x")
            .unwrap();
        dirs.push(path.clone());
    }
    vol.sync().unwrap();
    let mut image = vol.into_inner().into_inner().into_inner();
    let short = |at: usize| image[at + 11] & 0x3F != 0x0F && image[at] != 0xE5;
    let files: Vec<usize> = (0..image.len())
        .step_by(32)
        .filter(|&at| &image[at + 8..at + 11] == b"TXT" && short(at))
        .collect();
    assert_eq!(files.len(), 12);
    for &at in &files {
        image[at + 28..at + 32].copy_from_slice(&0u32.to_le_bytes());
    }
    let found = findings(&image);
    let paths: Vec<&str> = found
        .iter()
        .filter_map(|found| found.path.as_deref())
        .collect();
    assert_eq!(found.len(), 12, "{found:?}");
    let fits: Vec<String> = dirs
        .iter()
        .filter(|dir| dir.len() + "/\u{e9}t\u{e9}.txt".len() <= 1024)
        .map(|dir| format!("{dir}/\u{e9}t\u{e9}.txt"))
        .collect();
    assert_eq!(fits.len(), 9);
    for want in &fits {
        assert!(paths.contains(&want.as_str()), "{want} not in {paths:?}");
    }
    for path in &paths[fits.len()..] {
        assert_eq!(
            *path, dirs[8],
            "a path that does not fit ends at a whole name"
        );
    }
}
