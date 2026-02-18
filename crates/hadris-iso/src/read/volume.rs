use spin::Mutex;

use super::super::io::{self, IsoCursor, LogicalSector, Read, Seek, try_io_result_option as try_io};
use super::super::volume::VolumeDescriptor;

sync_only! {

pub struct VolumeDescriptorIter<'ctx, DATA: Read + Seek> {
    pub(crate) data: &'ctx Mutex<IsoCursor<DATA>>,
    pub(crate) current_sector: LogicalSector,
    pub(crate) done: bool,
}

impl<DATA: Read + Seek> Iterator for VolumeDescriptorIter<'_, DATA> {
    type Item = io::Result<VolumeDescriptor>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

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

        let descriptor = VolumeDescriptor::new(buf);
        if matches!(descriptor, VolumeDescriptor::End(_)) {
            self.done = true;
        }
        Some(Ok(descriptor))
    }
}

} // sync_only!
