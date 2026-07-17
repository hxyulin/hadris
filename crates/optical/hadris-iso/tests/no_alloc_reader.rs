#![cfg(all(feature = "std", feature = "sync", feature = "read"))]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;

use bytemuck::Zeroable;
use hadris_iso::directory::{DirectoryRecord, DirectoryRecordHeader, DirectoryRef, FileFlags};
use hadris_iso::io::LogicalSector;
use hadris_iso::joliet::JolietLevel;
use hadris_iso::read::{IsoNamespace, IsoReader};
use hadris_iso::types::{U16LsbMsb, U32LsbMsb};
use hadris_iso::volume::{
    PrimaryVolumeDescriptor, SupplementaryVolumeDescriptor, VolumeDescriptorHeader,
    VolumeDescriptorSetTerminator, VolumeDescriptorType,
};

struct CountingAllocator;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const BLOCK: usize = 2048;

fn root_header(extent: u32) -> DirectoryRecordHeader {
    DirectoryRecordHeader {
        len: 34,
        extended_attr_record: 0,
        extent: U32LsbMsb::new(extent),
        data_len: U32LsbMsb::new(BLOCK as u32),
        flags: FileFlags::DIRECTORY.bits(),
        volume_sequence_number: U16LsbMsb::new(1),
        file_identifier_len: 1,
        ..DirectoryRecordHeader::default()
    }
}

fn put(image: &mut [u8], sector: usize, offset: usize, bytes: &[u8]) {
    let start = sector * BLOCK + offset;
    image[start..start + bytes.len()].copy_from_slice(bytes);
}

fn fixture() -> Vec<u8> {
    let mut image = vec![0_u8; 32 * BLOCK];

    let mut pvd = PrimaryVolumeDescriptor::new("NOALLOC", 32);
    pvd.dir_record.header = root_header(20);
    put(&mut image, 16, 0, bytemuck::bytes_of(&pvd));

    let mut svd = SupplementaryVolumeDescriptor::zeroed();
    svd.header = VolumeDescriptorHeader::new(VolumeDescriptorType::SupplementaryVolumeDescriptor);
    svd.escape_sequences = JolietLevel::Level3.escape_sequence();
    svd.logical_block_size = U16LsbMsb::new(BLOCK as u16);
    svd.volume_sequence_number = U16LsbMsb::new(1);
    svd.dir_record.header = root_header(23);
    svd.file_structure_version = 1;
    put(&mut image, 17, 0, bytemuck::bytes_of(&svd));
    put(
        &mut image,
        18,
        0,
        VolumeDescriptorSetTerminator::new().to_bytes(),
    );

    let mut first = DirectoryRecord::new(
        b"PAYLOAD.BIN;1",
        &[],
        DirectoryRef {
            extent: LogicalSector(21),
            size: 5,
        },
        FileFlags::NOT_FINAL,
    );
    first.header_mut().volume_sequence_number = U16LsbMsb::new(1);
    let mut second = DirectoryRecord::new(
        b"PAYLOAD.BIN;1",
        &[],
        DirectoryRef {
            extent: LogicalSector(22),
            size: 6,
        },
        FileFlags::empty(),
    );
    second.header_mut().volume_sequence_number = U16LsbMsb::new(1);
    put(&mut image, 20, 0, first.to_bytes());
    put(&mut image, 20, first.size(), second.to_bytes());
    put(&mut image, 21, 0, b"hello");
    put(&mut image, 22, 0, b" world");

    let joliet_name = [
        0x65, 0xe5, 0x67, 0x2c, 0x00, b'.', 0x00, b'T', 0x00, b'X', 0x00, b'T', 0x00, b';', 0x00,
        b'1',
    ];
    let mut joliet = DirectoryRecord::new(
        &joliet_name,
        &[],
        DirectoryRef {
            extent: LogicalSector(24),
            size: 7,
        },
        FileFlags::empty(),
    );
    joliet.header_mut().volume_sequence_number = U16LsbMsb::new(1);
    put(&mut image, 23, 0, joliet.to_bytes());
    put(&mut image, 24, 0, b"unicode");
    image
}

fn exercise_navigation_and_streaming(
    image: &[u8],
    primary_output: &mut [u8; 11],
    joliet_output: &mut [u8; 7],
) {
    let mut reader = IsoReader::open(hadris_io::Cursor::new(image)).unwrap();
    assert_eq!(
        reader.preferred_root().namespace(),
        IsoNamespace::Joliet(JolietLevel::Level3)
    );

    let primary = reader.primary_root();
    let entry = reader
        .find_path_in(primary, "payload.bin")
        .unwrap()
        .unwrap();
    assert!(entry.is_multi_extent());
    assert_eq!(entry.total_size(), 11);
    let mut file = reader.open_file(&entry).unwrap();
    for byte in &mut *primary_output {
        assert_eq!(file.read_chunk(core::slice::from_mut(byte)).unwrap(), 1);
    }
    assert_eq!(file.read_chunk(&mut primary_output[..1]).unwrap(), 0);

    let entry = reader.find_path("日本.TXT").unwrap().unwrap();
    let mut file = reader.open_file(&entry).unwrap();
    assert_eq!(file.read_chunk(joliet_output).unwrap(), 7);
}

#[test]
fn navigation_and_streaming_allocate_nothing() {
    let _guard = TEST_LOCK.lock().unwrap();
    let image = fixture();
    let mut warm_primary_output = [0_u8; 11];
    let mut warm_joliet_output = [0_u8; 7];
    exercise_navigation_and_streaming(&image, &mut warm_primary_output, &mut warm_joliet_output);

    let mut primary_output = [0_u8; 11];
    let mut joliet_output = [0_u8; 7];
    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::SeqCst);
    exercise_navigation_and_streaming(&image, &mut primary_output, &mut joliet_output);

    COUNTING.store(false, Ordering::SeqCst);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    assert_eq!(&primary_output, b"hello world");
    assert_eq!(&joliet_output, b"unicode");
}

#[test]
fn malformed_record_length_is_rejected() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut image = fixture();
    image[20 * BLOCK] = 2;
    let mut reader = IsoReader::open(hadris_io::Cursor::new(image.as_slice())).unwrap();
    let error = reader
        .find_path_in(reader.primary_root(), "PAYLOAD.BIN")
        .unwrap_err();
    assert_eq!(error.kind(), hadris_iso::ErrorKind::InvalidData);
}
