use core::ops::DerefMut;

use hadris_io::Parsable;
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
        tracing::trace!(
            "attempting to read volume descriptor at offset: {:#x}",
            _current_offset
        );

        match try_io!(VolumeDescriptor::parse(data.deref_mut())) {
            VolumeDescriptor::End(_) => None,
            other => Some(Ok(other)),
        }
    }
}
