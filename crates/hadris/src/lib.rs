pub use hadris_core::file::{FileAttributes, OpenOptions};

#[cfg(feature = "fat")]
pub use hadris_fat as fat;

#[cfg(not(any(feature = "fat")))]
compile_error!("No file system selected");

pub enum FileSystemType {
    #[cfg(feature = "fat")]
    Fat32,
}

pub struct FileSystem<'ctx> {
    fs: Box<dyn 'ctx + hadris_core::internal::FileSystemFull>,
}

impl<'ctx> FileSystem<'ctx> {
    pub fn create_with_bytes(ty: FileSystemType, bytes: &'ctx mut [u8]) -> Self {
        match ty {
            #[cfg(feature = "fat")]
            FileSystemType::Fat32 => Self::new_f32_with_bytes(bytes),
        }
    }

    pub fn read_from_bytes(ty: FileSystemType, bytes: &'ctx mut [u8]) -> Self {
        match ty {
            #[cfg(feature = "fat")]
            FileSystemType::Fat32 => Self::read_f32_from_bytes(bytes),
        }
    }

    #[cfg(feature = "fat")]
    pub fn new_f32_with_bytes(bytes: &'ctx mut [u8]) -> Self {
        let sectors = bytes.len() / 512;
        let ops = fat::structures::Fat32Ops::recommended_config_for(sectors as u32);
        Self {
            fs: Box::new(fat::FileSystem::new_f32(ops, bytes)),
        }
    }

    #[cfg(feature = "fat")]
    pub fn read_f32_from_bytes(bytes: &'ctx mut [u8]) -> Self {
        Self {
            fs: Box::new(fat::FileSystem::read_from_bytes(bytes).unwrap()),
        }
    }

    pub fn open_file(&mut self, path: &str, options: OpenOptions) -> Result<File, ()> {
        Ok(File {
            file: self.fs.open(path, options)?,
        })
    }

    pub fn create_file(&mut self, path: &str, attributes: FileAttributes) -> Result<File, ()> {
        Ok(File {
            file: self.fs.create(path, attributes)?,
        })
    }
}

pub struct File {
    file: hadris_core::File,
}

impl File {
    pub fn read(&self, fs: &FileSystem, buffer: &mut [u8]) -> Result<usize, ()> {
        self.file.read(fs.fs.as_ref(), buffer)
    }

    pub fn write(&self, fs: &mut FileSystem, buffer: &[u8]) -> Result<usize, ()> {
        self.file.write(fs.fs.as_mut(), buffer)
    }
}
