use spin::Mutex;

use crate::{
    io::{self, IsoCursor, LogicalSector, Read, Seek, try_io_result_option as try_io},
    volume::VolumeDescriptor,
};

pub struct VolumeDescriptorIter<'ctx, DATA: Read + Seek> {
    pub(crate) data: &'ctx Mutex<IsoCursor<DATA>>,
    pub(crate) current_sector: LogicalSector,
}

impl<DATA: Read + Seek> Iterator for VolumeDescriptorIter<'_, DATA> {
    type Item = io::Result<VolumeDescriptor>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut data = self.data.lock();
        let _current_offset = try_io!(data.seek_sector(self.current_sector));
        self.current_sector += 1;

        #[cfg(feature = "std")]
        tracing::trace!(
            "attempting to read volume descriptor at offset: {:#x}",
            _current_offset
        );

        // Read the raw sector data and parse into VolumeDescriptor
        let mut buf = [0u8; 2048];
        try_io!(data.read_exact(&mut buf));

        match VolumeDescriptor::new(buf) {
            VolumeDescriptor::End(_) => None,
            other => Some(Ok(other)),
        }
    }
}
