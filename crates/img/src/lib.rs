use std::{fs::File, path::PathBuf};

pub enum PartitionScheme {
    Mbr,
    Gpt,
    Unknown,
}

pub struct Image {
    data: Vec<u8>,
    partition_scheme: PartitionScheme,
}

impl Image {
    pub fn create_new(size: u64, partition_scheme: PartitionScheme) -> Self {
        let mut zeroes = vec![0u8; size as usize];
        match partition_scheme {
            PartitionScheme::Mbr => {
                assert!(
                    size >= 512,
                    "MBR partition table must be at least 512 bytes"
                );
                assert!(
                    size % 512 == 0,
                    "MBR partition table must be a multiple of 512 bytes"
                );
                assert!(
                    size <= u32::MAX as u64,
                    "MBR partition table must be less than 2^32 bytes"
                );
                let table = mbr::PartitionTable::default();
                zeroes[446..510].copy_from_slice(&table.to_le_bytes());
            }
            _ => {}
        }
        Self {
            data: zeroes,
            partition_scheme,
        }
    }

    pub fn write_mbr(&mut self, _table: &mbr::PartitionTable) {
        // We can just copy the bytes from the 446 byte of the image
        unimplemented!()
    }

    pub fn write_binary(&mut self, data: &[u8], start: u64) {
        assert!(start < u32::MAX as u64);
        assert!(start + data.len() as u64 <= self.data.len() as u64);

        self.data[start as usize..start as usize + data.len()].copy_from_slice(data);
    }

    pub fn write_byte(&mut self, byte: u8, index: u64) {
        assert!(index < u32::MAX as u64);
        self.data[index as usize] = byte;
    }

    pub fn write_to_file(&self, path: PathBuf) {
        let mut file = File::create(path).unwrap();
        use std::io::Write;
        file.write_all(&self.data).unwrap();
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}
