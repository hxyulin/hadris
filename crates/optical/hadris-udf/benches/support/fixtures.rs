use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, Extent, MountOptions, Name, Node, Tree};
use hadris_storage::{BlockSize, MemDevice};
use hadris_udf::raw::{FileIdentifierDescriptor, ShortAd, Tag, U32Le, tag};
use hadris_udf::{UdfOptions, sync::UdfFs};

const BLOCK: usize = 2048;

pub struct Fixture {
    pub bytes: Vec<u8>,
    pub aeds: Vec<u64>,
    pub entries: usize,
    pub names: Vec<Vec<u8>>,
    pub payload: Vec<u8>,
    pub layout: &'static str,
}

fn sector(bytes: &mut [u8], block: u64) -> &mut [u8] {
    &mut bytes[block as usize * BLOCK..(block as usize + 1) * BLOCK]
}
fn ad(len: u32, kind: u32, block: u64, partition: u64) -> [u8; 8] {
    let descriptor = ShortAd {
        length: U32Le::new(len | kind << 30),
        position: U32Le::new((block - partition) as u32),
    };
    *bytemuck::from_bytes(bytemuck::bytes_of(&descriptor))
}

struct Layout {
    run: usize,
    chain: bool,
    directory: bool,
}

fn fragment(
    bytes: &mut [u8],
    icb: u64,
    partition: u64,
    free: &mut u64,
    layout: Layout,
) -> Vec<u64> {
    let Layout {
        run,
        chain,
        directory,
    } = layout;
    let entry = sector(bytes, icb);
    let len = u64::from_le_bytes(entry[56..64].try_into().unwrap()) as usize;
    let start = partition + u64::from(u32::from_le_bytes(entry[180..184].try_into().unwrap()));
    let mut data = bytes[start as usize * BLOCK..start as usize * BLOCK + len].to_vec();
    let mut pieces = Vec::new();
    for chunk in data.chunks(run) {
        pieces.push((*free, chunk.len()));
        *free += chunk.len().div_ceil(BLOCK) as u64 + 1;
    }
    if directory {
        let mut pos = 0;
        while pos < data.len() {
            let fid: FileIdentifierDescriptor = bytemuck::pod_read_unaligned(&data[pos..pos + 38]);
            let end = pos + fid.total_len();
            let location = pieces[pos / run].0 - partition + ((pos % run) / BLOCK) as u64;
            Tag::seal(
                &mut data[pos..end],
                tag::FILE_IDENTIFIER,
                fid.tag.version.get(),
                location as u32,
                usize::from(fid.tag.crc_length.get()),
            );
            pos = end;
        }
    }
    for ((block, _), chunk) in pieces.iter().zip(data.chunks(run)) {
        let at = *block as usize * BLOCK;
        bytes[at..at + chunk.len()].copy_from_slice(chunk);
    }
    let mut aeds = Vec::new();
    let descriptors = if chain {
        for i in 1..pieces.len() {
            let (previous, size) = pieces[i - 1];
            let at = previous + size.div_ceil(BLOCK) as u64;
            let (block, size) = pieces[i];
            let mut ads = ad(size as u32, 0, block, partition).to_vec();
            if i + 1 < pieces.len() {
                let next = block + size.div_ceil(BLOCK) as u64;
                ads.extend_from_slice(&ad(BLOCK as u32, 3, next, partition));
            }
            let prev = aeds.last().map_or(0, |&block| (block - partition) as u32);
            let buf = sector(bytes, at);
            buf.fill(0);
            buf[16..20].copy_from_slice(&prev.to_le_bytes());
            buf[20..24].copy_from_slice(&(ads.len() as u32).to_le_bytes());
            buf[24..24 + ads.len()].copy_from_slice(&ads);
            Tag::seal(
                buf,
                tag::ALLOCATION_EXTENT,
                2,
                (at - partition) as u32,
                8 + ads.len(),
            );
            aeds.push(at);
        }
        let (first, size) = pieces[0];
        let mut ads = ad(size as u32, 0, first, partition).to_vec();
        if let Some(&aed) = aeds.first() {
            ads.extend_from_slice(&ad(BLOCK as u32, 3, aed, partition));
        }
        ads
    } else {
        pieces
            .iter()
            .flat_map(|&(block, size)| ad(size as u32, 0, block, partition))
            .collect()
    };
    let entry = sector(bytes, icb);
    assert!(176 + descriptors.len() <= BLOCK);
    entry[172..176].copy_from_slice(&(descriptors.len() as u32).to_le_bytes());
    entry[176..].fill(0);
    entry[176..176 + descriptors.len()].copy_from_slice(&descriptors);
    let original = Tag::read(entry).unwrap();
    Tag::seal(
        entry,
        tag::FILE_ENTRY,
        original.version.get(),
        original.location.get(),
        160 + descriptors.len(),
    );
    aeds
}

pub fn image(entries: usize, layout: &'static str) -> Fixture {
    let mut tree = Tree::new();
    let mut names = Vec::new();
    for i in 0..entries {
        let name = format!("entry-{i:04}");
        tree.insert(&name, Node::file(Content::empty())).unwrap();
        names.push(name.into_bytes());
    }
    let payload: Vec<u8> = (0..1024 * 1024).map(|i| (i / BLOCK % 251) as u8).collect();
    tree.insert("payload", Node::file(Content::bytes(payload.clone())))
        .unwrap();
    names.push(b"payload".to_vec());
    let initial = hadris_udf::plan(&tree, &UdfOptions::default()).unwrap();
    let options = UdfOptions::default().with_min_blocks(initial.size() / BLOCK as u64 + 700);
    let report = hadris_udf::plan(&tree, &options).unwrap();
    let mut dev = MemDevice::new(
        vec![0; report.size() as usize],
        BlockSize::new(BLOCK as u32).unwrap(),
    );
    hadris_udf::sync::write(&mut dev, &tree, &options).unwrap();
    let mut bytes = dev.into_inner();
    let mut aeds = Vec::new();
    if layout != "contiguous" {
        let mut fs = UdfFs::mount(
            MemDevice::new(bytes.clone(), BlockSize::new(BLOCK as u32).unwrap()),
            MountOptions::new(),
        )
        .unwrap();
        let partition = u64::from(fs.info().partitions()[0].start());
        let end = partition + u64::from(fs.info().partitions()[0].len());
        let mut free = end - 696;
        let used = report
            .files()
            .flat_map(|(_, extents)| extents)
            .map(|extent| extent.end().div_ceil(BLOCK as u64))
            .max()
            .unwrap();
        assert!(used < free);
        let root = fs.root();
        let file = fs.lookup(root, Name::new("payload")).unwrap();
        let mut record = [Extent::new(0, 0)];
        fs.records(root, &mut record).unwrap();
        let root_icb = record[0].offset() / BLOCK as u64;
        fs.records(file, &mut record).unwrap();
        let file_icb = record[0].offset() / BLOCK as u64;
        let chain = layout == "fragmented-aed";
        aeds.extend(fragment(
            &mut bytes,
            root_icb,
            partition,
            &mut free,
            Layout {
                run: BLOCK,
                chain,
                directory: true,
            },
        ));
        aeds.extend(fragment(
            &mut bytes,
            file_icb,
            partition,
            &mut free,
            Layout {
                run: 4 * BLOCK,
                chain,
                directory: false,
            },
        ));
        assert!(free <= end);
        aeds.sort_unstable();
    }
    Fixture {
        bytes,
        aeds,
        entries,
        names,
        payload,
        layout,
    }
}
