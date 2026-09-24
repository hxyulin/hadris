#![allow(dead_code)]

use hadris_cpio::raw::{NewcFields, NewcHeader};
use hadris_cpio::sync::{CpioReader, write};
use hadris_cpio::{CpioOptions, Error, Format, ReaderOptions};
use hadris_fs::FileType;
use hadris_fs::tree::Tree;
use hadris_io::sync::Read;
use hadris_io::{Cursor, StdIo};

pub type ReadError = Error<<Cursor<'static> as hadris_io::ErrorType>::Error>;

/// One entry as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadEntry {
    pub name: String,
    pub file_type: FileType,
    pub mode: u32,
    pub ino: u64,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u64,
    pub rdev: (u32, u32),
    pub data: Vec<u8>,
}

pub fn archive(tree: &Tree, format: Format) -> Vec<u8> {
    let mut out = StdIo::new(Vec::new());
    let report = write(&mut out, tree, &CpioOptions::default().with_format(format)).unwrap();
    let bytes = out.into_inner();
    assert_eq!(report.size_bytes(), bytes.len() as u64);
    bytes
}

pub fn read_all_with(bytes: &[u8], options: ReaderOptions) -> Result<Vec<ReadEntry>, ReadError> {
    let mut reader = CpioReader::with_options(Cursor::new(bytes), options);
    let mut out = Vec::new();
    while let Some(mut entry) = reader.next_entry()? {
        let mut data = vec![0u8; entry.len() as usize];
        entry.read_exact(&mut data).map_err(|err| match err {
            hadris_io::ExactError::Io(err) => err,
            _ => panic!("entry ended early"),
        })?;
        out.push(ReadEntry {
            name: entry.name_str().unwrap().to_string(),
            file_type: entry.file_type(),
            mode: entry.mode(),
            ino: entry.ino(),
            nlink: entry.nlink(),
            uid: entry.uid(),
            gid: entry.gid(),
            mtime: entry.mtime(),
            rdev: (entry.rdev().major(), entry.rdev().minor()),
            data,
        });
    }
    Ok(out)
}

pub fn read_all(bytes: &[u8]) -> Result<Vec<ReadEntry>, ReadError> {
    read_all_with(bytes, ReaderOptions::new())
}

/// A `newc` entry with `name`, `mode` and `data`, padded as the format says.
pub fn newc_entry(name: &[u8], mode: u32, data: &[u8], crc: Option<u32>) -> Vec<u8> {
    let fields = NewcFields {
        ino: 1,
        mode,
        nlink: 1,
        filesize: data.len() as u32,
        namesize: name.len() as u32 + 1,
        check: crc.unwrap_or(0),
        ..NewcFields::default()
    };
    let mut out = NewcHeader::new(crc.is_some(), &fields).0.to_vec();
    out.extend_from_slice(name);
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

pub fn trailer() -> Vec<u8> {
    newc_entry(b"TRAILER!!!", 0, b"", None)
}
