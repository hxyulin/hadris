use hadris_io::SeekFrom;
use hadris_io::sync::{Read, Seek, Write};

use crate::PartitionView;

impl<S: Read + Seek> Read for PartitionView<'_, S> {
    fn read(&mut self, buffer: &mut [u8]) -> hadris_io::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source.seek(SeekFrom::Start(absolute))?;
        let read = self.source.read(&mut buffer[..length])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl<S: Read + Seek> Seek for PartitionView<'_, S> {
    fn seek(&mut self, from: SeekFrom) -> hadris_io::Result<u64> {
        self.position = self.seek_position(from)?;
        Ok(self.position)
    }
}

impl<S: Read + Write + Seek> Write for PartitionView<'_, S> {
    fn write(&mut self, buffer: &[u8]) -> hadris_io::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source.seek(SeekFrom::Start(absolute))?;
        let written = self.source.write(&buffer[..length])?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> hadris_io::Result<()> {
        self.source.flush()
    }
}

#[cfg(test)]
mod tests {
    use hadris_io::sync::{Read, Seek, Write};
    use hadris_io::{SeekFrom, StdIo};
    use std::vec;

    use crate::PartitionView;

    #[test]
    fn partition_view_translates_and_bounds_io() {
        let mut source = StdIo::new(std::io::Cursor::new(vec![0_u8, 1, 2, 3, 4, 5, 6, 7]));
        let mut view = PartitionView::new(&mut source, 2, 4).unwrap();

        let mut buffer = [0_u8; 8];
        assert_eq!(view.read(&mut buffer).unwrap(), 4);
        assert_eq!(&buffer[..4], &[2, 3, 4, 5]);
        assert_eq!(view.read(&mut buffer).unwrap(), 0);

        view.seek(SeekFrom::Start(1)).unwrap();
        view.write_all(&[9, 8]).unwrap();
        let source = view.into_inner();
        assert_eq!(&source.get_ref().get_ref()[..], &[0, 1, 2, 9, 8, 5, 6, 7]);
    }

    #[test]
    fn partition_view_rejects_invalid_ranges_and_seeks() {
        let mut source = StdIo::new(std::io::Cursor::new(vec![0_u8; 8]));
        assert!(PartitionView::new(&mut source, u64::MAX, 2).is_err());
        assert!(PartitionView::new(&mut source, 0, 0).is_err());

        let mut view = PartitionView::new(&mut source, 2, 4).unwrap();
        assert!(view.seek(SeekFrom::Start(5)).is_err());
        assert!(view.seek(SeekFrom::End(-5)).is_err());
    }
}
