pub struct FailingDevice;
impl hadris_io::ErrorType for FailingDevice {
    type Error = core::convert::Infallible;
}
#[cfg(feature = "sync")]
impl hadris_storage::sync::BlockDevice for FailingDevice {
    fn block_size(&self) -> hadris_storage::BlockSize {
        hadris_storage::BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        8
    }
    fn read_blocks(
        &mut self,
        _: hadris_storage::BlockIndex,
        _: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        Err(hadris_io::ErrorKind::InvalidInput.into())
    }
}
#[cfg(feature = "async")]
impl hadris_storage::async_::BlockDevice for FailingDevice {
    type State = ();
    fn cancel(&mut self, _: &mut ()) {}
    fn block_size(&self) -> hadris_storage::BlockSize {
        hadris_storage::BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        8
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut (),
        _: &mut core::task::Context<'_>,
        _: hadris_storage::BlockIndex,
        _: &mut [u8],
    ) -> core::task::Poll<Result<(), hadris_io::Error<Self::Error>>> {
        core::task::Poll::Ready(Err(hadris_io::ErrorKind::InvalidInput.into()))
    }
}
