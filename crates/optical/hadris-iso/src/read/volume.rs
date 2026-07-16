use super::super::io::{self, IsoCursor, LogicalSector, Read, Seek};
use super::super::volume::VolumeDescriptor;
use spin::Mutex;

io_transform! {

/// Represents VolumeDescriptorIter.
pub struct VolumeDescriptorIter<'ctx, DATA: Read + Seek> {
    pub(crate) data: &'ctx Mutex<IsoCursor<DATA>>,
    pub(crate) current_sector: LogicalSector,
    pub(crate) done: bool,
}

impl<DATA: Read + Seek> VolumeDescriptorIter<'_, DATA> {
    /// Reads the next volume descriptor.
    pub async fn next_descriptor(&mut self) -> io::Result<Option<VolumeDescriptor>> {
        if self.done {
            return Ok(None);
        }

        let mut data = self.data.lock();
        let _current_offset = data.seek_sector(self.current_sector).await?;
        self.current_sector += 1;

        #[cfg(feature = "std")]
        tracing::trace!(
            "attempting to read volume descriptor at offset: {:#x}",
            _current_offset
        );

        // Read the raw sector data and parse into VolumeDescriptor
        let mut buf = [0u8; 2048];
        data.read_exact(&mut buf).await?;

        let descriptor = VolumeDescriptor::new(buf);
        if matches!(descriptor, VolumeDescriptor::End(_)) {
            self.done = true;
        }
        Ok(Some(descriptor))
    }
}

} // io_transform!

sync_only! {
impl<DATA: Read + Seek> Iterator for VolumeDescriptorIter<'_, DATA> {
    type Item = io::Result<VolumeDescriptor>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_descriptor().transpose()
    }
}
} // sync_only!
