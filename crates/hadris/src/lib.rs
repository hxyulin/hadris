pub use hadris_core::OpenMode;
#[cfg(feature = "fat")]
pub use hadris_fat as fat;

#[cfg(not(any(feature = "fat")))]
compile_error!("No file system selected");

pub enum FileSystemType {
    #[cfg(feature = "fat")]
    Fat,
}

pub struct FileSystem<'ctx> {
    fs: Box<dyn 'ctx + hadris_core::internal::FileSystemFull>,
}

impl<'ctx> FileSystem<'ctx> {
    pub fn with_bytes(ty: FileSystemType, bytes: &'ctx mut [u8]) -> Self {
        match ty {
            #[cfg(feature = "fat")]
            FileSystemType::Fat => Self::new_f32_with_bytes(bytes),
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

    pub fn open_file(&mut self, path: &str, mode: OpenMode) -> Result<File, ()> {
        Ok(File {
            file: self.fs.open(path, mode)?,
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
