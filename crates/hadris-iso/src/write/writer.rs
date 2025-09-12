use std::vec::Vec;

use crate::directory::DirectoryRef;

pub struct DirectoryId {
    indices: Vec<usize>,
}

impl DirectoryId {
    pub fn push(&mut self, index: usize) {
        self.indices.push(index);
    }

    pub fn pop(&mut self) -> usize {
        self.indices.pop().expect("directory underflow")
    }
}

#[derive(Debug)]
pub struct WrittenFiles<'a> {
    root: WrittenDirectory<'a>,
}

impl<'a> WrittenFiles<'a> {
    pub const fn new() -> Self {
        static ROOT: &'static str = "";
        Self {
            root: WrittenDirectory::new(ROOT),
        }
    }

    pub fn root_dir(&self) -> DirectoryId {
        DirectoryId {
            indices: Vec::new(),
        }
    }

    pub fn root_ref(&self) -> DirectoryRef {
        self.root.entry.expect("did not write root directory!")
    }

    pub fn get_parent(&self, id: &DirectoryId) -> &WrittenDirectory<'a> {
        let mut dir = &self.root;
        for index in &id.indices[0..id.indices.len() - 1] {
            dir = &dir.dirs[*index];
        }
        dir
    }

    pub fn get_mut(&mut self, id: &DirectoryId) -> &mut WrittenDirectory<'a> {
        let mut dir = &mut self.root;
        for index in &id.indices {
            dir = &mut dir.dirs[*index];
        }
        dir
    }
}

#[derive(Debug)]
pub struct WrittenDirectory<'a> {
    pub name: &'a str,
    pub entry: Option<DirectoryRef>,
    pub dirs: Vec<WrittenDirectory<'a>>,
    pub files: Vec<WrittenFile<'a>>,
}

impl<'a> WrittenDirectory<'a> {
    pub const fn new(name: &'a str) -> Self {
        Self {
            name,
            entry: None,
            dirs: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn push_dir(&mut self, name: &'a str) -> usize {
        self.dirs.push(Self::new(name));
        self.dirs.len() - 1
    }
}

#[derive(Debug)]
pub struct WrittenFile<'a> {
    pub name: &'a str,
    pub entry: DirectoryRef,
}
