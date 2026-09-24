//! Multi-extent file conformance and reader coverage.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use hadris_fs::sync::FileSystem;
use hadris_fs::tree::{Content, Tree};
use hadris_fs::{OpenMode, Resolve};
use hadris_iso::{IsoLevel, IsoOptions, Namespace, RockRidge};
use hadris_storage::{BlockIndex, BlockSize};
use hadris_tests::harness::command::{require_or_skip, run_command};
use hadris_tests::harness::tree::EntryData;
use hadris_tests::iso::model::IsoState;
use hadris_tests::iso::{SECTOR_SIZE, VOLUME_ID, hadris, spec, xorriso};

const FILE_SIZE: usize = 4 * SECTOR_SIZE;
const MULTI_EXTENT: u8 = 0x80;

struct Fixture {
    bytes: Vec<u8>,
    expected: IsoState,
    records: [usize; 3],
}

fn write_both_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    bytes[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
}

fn fixture() -> Fixture {
    let contents = (0..FILE_SIZE).map(|index| (index % 251) as u8).collect();
    let expected = IsoState {
        volume_id: VOLUME_ID.to_string(),
        entries: BTreeMap::from([("/LARGE.BIN".to_string(), EntryData::File(contents))]),
    };
    let mut bytes = hadris::write(&expected).unwrap();
    let root_record = 16 * SECTOR_SIZE + 156;
    let root_extent =
        u32::from_le_bytes(bytes[root_record + 2..root_record + 6].try_into().unwrap()) as usize;
    let mut file_record = root_extent * SECTOR_SIZE;
    file_record += bytes[file_record] as usize;
    file_record += bytes[file_record] as usize;
    let record_len = bytes[file_record] as usize;
    let first_extent =
        u32::from_le_bytes(bytes[file_record + 2..file_record + 6].try_into().unwrap());
    let template = bytes[file_record..file_record + record_len].to_vec();
    let records = [
        file_record,
        file_record + record_len,
        file_record + record_len * 2,
    ];
    let sections = [
        (first_extent, SECTOR_SIZE as u32, true),
        (first_extent + 1, (2 * SECTOR_SIZE) as u32, true),
        (first_extent + 3, SECTOR_SIZE as u32, false),
    ];
    for (record, (extent, length, has_more)) in records.iter().zip(sections) {
        bytes[*record..*record + record_len].copy_from_slice(&template);
        write_both_u32(&mut bytes, *record + 2, extent);
        write_both_u32(&mut bytes, *record + 10, length);
        if has_more {
            bytes[*record + 25] |= MULTI_EXTENT;
        } else {
            bytes[*record + 25] &= !MULTI_EXTENT;
        }
    }
    Fixture {
        bytes,
        expected,
        records,
    }
}

#[test]
fn three_section_file_matches_oracle_and_hadris_reader() {
    let fixture = fixture();
    hadris::verify_image(
        "three-section multi-extent fixture",
        fixture.bytes,
        &fixture.expected,
    )
    .unwrap();
}

#[test]
fn unaligned_non_final_section_matches_oracle_and_hadris_reader() {
    let mut fixture = fixture();
    write_both_u32(
        &mut fixture.bytes,
        fixture.records[0] + 10,
        (SECTOR_SIZE - 1) as u32,
    );
    let EntryData::File(contents) = fixture.expected.entries.get_mut("/LARGE.BIN").unwrap() else {
        unreachable!();
    };
    contents.remove(SECTOR_SIZE - 1);
    hadris::verify_image(
        "unaligned non-final multi-extent section",
        fixture.bytes,
        &fixture.expected,
    )
    .unwrap();
}

#[test]
fn oracle_accepts_per_section_protection_flag() {
    let mut fixture = fixture();
    fixture.bytes[fixture.records[1] + 25] |= 0x10;
    assert_eq!(spec::snapshot(&fixture.bytes).unwrap(), fixture.expected);
}

#[test]
fn oracle_rejects_invalid_multi_extent_chains() {
    type Corrupt = fn(&mut Fixture);

    let cases: [(&str, Corrupt); 2] = [
        ("truncated chain", |fixture| {
            fixture.bytes[fixture.records[2]] = 0;
        }),
        ("changed identifier", |fixture| {
            fixture.bytes[fixture.records[1] + 33] = b'X';
        }),
    ];
    for (name, corrupt) in cases {
        let mut fixture = fixture();
        corrupt(&mut fixture);
        assert!(
            spec::snapshot(&fixture.bytes).is_err(),
            "oracle accepted {name}"
        );
    }
}

/// A file of zeros that starts with `head`.
struct Zeros {
    len: u64,
}

const HEAD: &[u8] = b"head";

impl hadris_io::ErrorType for Zeros {
    type Error = std::io::Error;
}

impl hadris_io::sync::ByteSource for Zeros {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let take = buf.len().min(self.len.saturating_sub(offset) as usize);
        buf[..take].fill(0);
        if offset < HEAD.len() as u64 {
            let start = offset as usize;
            let end = HEAD.len().min(start + take);
            buf[..end - start].copy_from_slice(&HEAD[start..end]);
        }
        Ok(take)
    }
}

/// A host file that skips writing all-zero chunks, so a 4 GiB image stays
/// sparse.
struct SparseFile(File);

static ZEROS: [u8; 64 * 1024] = [0; 64 * 1024];

impl hadris_io::ErrorType for SparseFile {
    type Error = std::io::Error;
}

impl hadris_storage::sync::BlockDevice for SparseFile {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(2048).unwrap()
    }

    fn block_count(&self) -> u64 {
        self.0.metadata().map_or(0, |meta| meta.len() / 2048)
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<std::io::Error>> {
        self.0
            .seek(SeekFrom::Start(first.get() * 2048))
            .and_then(|_| self.0.read_exact(buf))
            .map_err(|err| hadris_io::Error::device(err, "read failed"))
    }

    fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), hadris_io::Error<std::io::Error>> {
        let mut write = || -> std::io::Result<()> {
            let mut offset = first.get() * 2048;
            for chunk in buf.chunks(ZEROS.len()) {
                if chunk != &ZEROS[..chunk.len()] {
                    self.0.seek(SeekFrom::Start(offset))?;
                    self.0.write_all(chunk)?;
                }
                offset += chunk.len() as u64;
            }
            if self.0.metadata()?.len() < offset {
                self.0.set_len(offset)?;
            }
            Ok(())
        };
        write().map_err(|err| hadris_io::Error::device(err, "write failed"))
    }
}

/// Rock Ridge describes every record of a file larger than 4 GiB, so
/// libarchive and libisofs see its name and mode (integrate-1).
#[test]
fn rock_ridge_covers_every_extent_of_a_large_file() {
    let len = (4u64 << 30) + 4096 + 7;
    let mut tree = Tree::new();
    tree.add_file("big.bin", Content::source(Zeros { len }))
        .unwrap();
    tree.add_file("small.txt", Content::bytes("small")).unwrap();
    let options = IsoOptions::default()
        .with_level(IsoLevel::L3)
        .with_rock_ridge(RockRidge::default());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.iso");
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let report = hadris_iso::sync::write(SparseFile(file), &tree, &options).unwrap();
    assert!(report.extent_of("/big.bin").unwrap().len() == len);

    let file = File::open(&path).unwrap();
    let mut iso = hadris_iso::sync::IsoImage::open(SparseFile(file)).unwrap();
    let mut view = iso.view(Namespace::RockRidge).unwrap();
    let node = view.resolve(b"/big.bin", Resolve::Lexical).unwrap();
    let meta = view.stat(node).unwrap();
    assert_eq!(meta.len(), len);
    assert_eq!(meta.permissions().bits(), 0o644);
    let mut head = [0u8; 4];
    view.open(node, OpenMode::Read).unwrap();
    view.read(node, 0, &mut head).unwrap();
    view.close(node).unwrap();
    view.forget(node, 1);
    assert_eq!(&head, HEAD);

    if require_or_skip("bsdtar", "--version") {
        let listing = run_command("bsdtar", vec!["-tvf".into(), path.clone().into()]).unwrap();
        let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
        let line = listing
            .lines()
            .find(|line| line.ends_with("big.bin"))
            .unwrap_or_else(|| panic!("bsdtar lists no big.bin:\n{listing}"));
        assert!(line.starts_with("-rw-r--r--"), "{line}");
        assert!(line.contains(&len.to_string()), "{line}");
    }
    if xorriso::require() {
        let output = xorriso::inspect(&path, &["-lsl", "/"]);
        let listing = String::from_utf8_lossy(&output.stdout).into_owned();
        let line = listing
            .lines()
            .find(|line| line.ends_with("'big.bin'"))
            .unwrap_or_else(|| panic!("xorriso lists no big.bin:\n{listing}"));
        assert!(line.starts_with("-rw-r--r--"), "{line}");
        assert!(line.contains(&len.to_string()), "{line}");
    }
}
