use super::filesystem::FileSystem;
use super::paths::write_all_at;
use super::tree::ContentReader;
use crate::{
    Capabilities, ErrorKind, Field, FileType, FsResult, Name, Node, NodeId, OpenMode, PathError,
    Report, SetAttr, Stored, Tree, TreeEntry, Warning, WarningKind,
};
use alloc::vec::Vec;

/// Bytes moved per read.
const CHUNK: usize = 64 * 1024;

/// How many nodes lost each field.
#[derive(Default)]
struct Dropped([u64; 5]);

const DROPPABLE: [Field; 5] = [
    Field::Created,
    Field::Modified,
    Field::Accessed,
    Field::Permissions,
    Field::Owner,
];

impl Dropped {
    fn add(&mut self, field: Field) {
        if let Some(index) = DROPPABLE.iter().position(|&f| f == field) {
            self.0[index] += 1;
        }
    }

    fn report(&self, report: &mut Report) {
        for (field, &count) in DROPPABLE.iter().zip(&self.0) {
            if count > 0 {
                report.push_warning(
                    Warning::new(WarningKind::Dropped(*field), "not stored by the target")
                        .with_count(count),
                );
            }
        }
    }
}

/// The attributes a filesystem with `caps` keeps from `attrs`, split into
/// what creation takes and the times and attribute bits, which are set once
/// the data is written, since writing sets bits such as FAT's archive bit. Permissions the format stores only in part come back separately,
/// to be tried on their own. Fields the target drops are counted.
fn split_attrs(
    attrs: &SetAttr,
    caps: &Capabilities,
    dropped: &mut Dropped,
) -> (SetAttr, SetAttr, Option<SetAttr>) {
    let mut create = SetAttr::new();
    let mut times = SetAttr::new();
    let mut partial = None;
    let stored = |field| caps.stores(field) != Stored::No;
    for (field, time) in [
        (Field::Created, attrs.created()),
        (Field::Modified, attrs.modified()),
        (Field::Accessed, attrs.accessed()),
    ] {
        let Some(time) = time else { continue };
        if !stored(field) {
            dropped.add(field);
            continue;
        }
        times = match field {
            Field::Created => times.with_created(time),
            Field::Modified => times.with_modified(time),
            _ => times.with_accessed(time),
        };
    }
    if let Some(permissions) = attrs.permissions() {
        match caps.stores(Field::Permissions) {
            Stored::Yes => create = create.with_permissions(permissions),
            Stored::No => dropped.add(Field::Permissions),
            _ => partial = Some(SetAttr::new().with_permissions(permissions)),
        }
    }
    if let Some(owner) = attrs.owner() {
        match caps.stores(Field::Owner) {
            Stored::Yes => create = create.with_owner(owner),
            _ => dropped.add(Field::Owner),
        }
    }
    if let Some(attributes) = attrs.attributes()
        && stored(Field::Attributes)
    {
        times = times.with_attributes(attributes);
    }
    (create, times, partial)
}

io_transform! {

/// Sets `partial`, permissions the target stores only in part; a value it
/// cannot hold is counted as dropped.
async fn try_partial<F: FileSystem + ?Sized>(
    fs: &mut F,
    node: NodeId,
    partial: Option<SetAttr>,
    dropped: &mut Dropped,
) -> FsResult<(), F::DeviceError> {
    let Some(partial) = partial else { return Ok(()) };
    match fs.setattr(node, &partial).await {
        Err(err) if err.kind() == ErrorKind::Unsupported => {
            dropped.add(Field::Permissions);
            Ok(())
        }
        other => other,
    }
}

/// Writes the content of `node` into the open file `to`.
async fn copy_data<F: FileSystem + ?Sized>(fs: &mut F, node: &Node, to: NodeId, buf: &mut [u8]) -> Result<(), PathError> {
    let Some(content) = node.content() else { return Ok(()) };
    let mut reader = ContentReader::open(content).await?;
    let mut offset = 0;
    while offset < reader.len() {
        let n = reader.read_at(offset, buf).await?;
        if n == 0 {
            return Err(PathError::new(ErrorKind::Corrupt, "file content ended early"));
        }
        write_all_at(fs, to, offset, &buf[..n]).await?;
        offset += n as u64;
    }
    Ok(())
}

async fn copy_file_contents<F: FileSystem + ?Sized>(
    fs: &mut F,
    node: &Node,
    to: NodeId,
    times: &SetAttr,
    partial: Option<SetAttr>,
    dropped: &mut Dropped,
    buf: &mut [u8],
) -> Result<(), PathError> {
    copy_data(fs, node, to, buf).await?;
    if !times.is_empty() {
        fs.setattr(to, times).await?;
    }
    try_partial(fs, to, partial, dropped).await?;
    Ok(())
}

}
