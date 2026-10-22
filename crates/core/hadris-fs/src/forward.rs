/// Generates the `FileSystem` forwarding code of a driver `Type<D>`.
///
/// Internal to the Hadris crates and outside the semver promise. Every form
/// takes method groups: `read` (the required methods), `resolve`, and `write`
/// (the methods whose defaults refuse writes, with `fsync` and `sync`).
///
/// - `shims Type<D: Bound> { groups }`: inherent methods delegating to
///   `async_::FileSystem`, so calls stay unambiguous with both
///   `async_::FileSystem` and `async_::SendFileSystem` in scope.
/// - `send Type<D: Bound> as Trait { groups }`, or `send Type as Trait` for
///   a type without a device parameter: an impl of
///   `async_::SendFileSystem` returning the futures of `async_::FileSystem`.
/// - `async fn Type<D: Bound> as Trait { groups }`, or `fn` in blocking
///   code: an impl of `Trait` forwarding to the inherent methods.
#[doc(hidden)]
#[macro_export]
macro_rules! __forward_filesystem {
    (shims $ty:ident<D: $bound:path> { $($group:ident)* }) => {
        impl<D: $bound> $ty<D> {
            $($crate::__forward_filesystem! { @native [pub] [] $group })*
        }
    };
    ($(#[$attr:meta])* send $ty:ident<D: $bound:path> as $trait:path { $($group:ident)* }) => {
        $(#[$attr])*
        impl<D: $bound> $trait for $ty<D> {
            type DeviceError = D::Error;
            $($crate::__forward_filesystem! { @native [] [+ core::marker::Send] $group })*
        }
    };
    ($(#[$attr:meta])* send $ty:ident as $trait:path { $($group:ident)* }) => {
        $(#[$attr])*
        impl $trait for $ty {
            type DeviceError = <Self as $crate::async_::FileSystem>::DeviceError;
            $($crate::__forward_filesystem! { @native [] [+ core::marker::Send] $group })*
        }
    };
    (async fn $ty:ident<D: $bound:path> as $trait:path { $($group:ident)* }) => {
        impl<D: $bound> $trait for $ty<D> {
            type DeviceError = D::Error;
            $($crate::__forward_filesystem! { @inherent [async] [.await] $group })*
        }
    };
    (fn $ty:ident<D: $bound:path> as $trait:path { $($group:ident)* }) => {
        impl<D: $bound> $trait for $ty<D> {
            type DeviceError = D::Error;
            $($crate::__forward_filesystem! { @inherent [] [] $group })*
        }
    };
    (@native [$($pub:tt)?] [$($send:tt)*] read) => {
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::capabilities`]."] $pub)?
        fn capabilities(&self) -> $crate::Capabilities {
            $crate::async_::FileSystem::capabilities(self)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::root`]."] $pub)?
        fn root(&self) -> $crate::NodeId {
            $crate::async_::FileSystem::root(self)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::statfs`]."] $pub)?
        fn statfs(
            &mut self,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<$crate::FsStats, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::statfs(self)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::label`]."] $pub)?
        fn label<'b>(
            &mut self,
            buf: &'b mut [u8],
        ) -> impl core::future::Future<
            Output = $crate::FsResult<Option<&'b str>, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::label(self, buf)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::lookup`]."] $pub)?
        fn lookup(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<$crate::NodeId, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::lookup(self, dir, name)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::forget`]."] $pub)?
        fn forget(&mut self, node: $crate::NodeId, count: u64) {
            $crate::async_::FileSystem::forget(self, node, count)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::parent`]."] $pub)?
        fn parent(
            &mut self,
            dir: $crate::NodeId,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<$crate::NodeId, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::parent(self, dir)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::stat`]."] $pub)?
        fn stat(
            &mut self,
            node: $crate::NodeId,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<$crate::Metadata, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::stat(self, node)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::readdir`]."] $pub)?
        fn readdir(
            &mut self,
            dir: $crate::NodeId,
            from: $crate::DirCursor,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<Option<$crate::DirEntry>, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::readdir(self, dir, from)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::readlink`]."] $pub)?
        fn readlink<'b>(
            &mut self,
            node: $crate::NodeId,
            buf: &'b mut [u8],
        ) -> impl core::future::Future<
            Output = $crate::FsResult<&'b [u8], <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::readlink(self, node, buf)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::open`]."] $pub)?
        fn open(
            &mut self,
            node: $crate::NodeId,
            mode: $crate::OpenMode,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::open(self, node, mode)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::close`]."] $pub)?
        fn close(
            &mut self,
            node: $crate::NodeId,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::close(self, node)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::read`]."] $pub)?
        fn read(
            &mut self,
            node: $crate::NodeId,
            offset: u64,
            buf: &mut [u8],
        ) -> impl core::future::Future<
            Output = $crate::FsResult<usize, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::read(self, node, offset, buf)
        }
    };
    (@native [$($pub:tt)?] [$($send:tt)*] resolve) => {
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::resolve`]."] $pub)?
        fn resolve(
            &mut self,
            path: &[u8],
            how: $crate::Resolve,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<$crate::NodeId, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::resolve(self, path, how)
        }
    };
    (@native [$($pub:tt)?] [$($send:tt)*] write) => {
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::setattr`]."] $pub)?
        fn setattr(
            &mut self,
            node: $crate::NodeId,
            changes: &$crate::SetAttr,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::setattr(self, node, changes)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::write`]."] $pub)?
        fn write(
            &mut self,
            node: $crate::NodeId,
            offset: u64,
            buf: &[u8],
        ) -> impl core::future::Future<
            Output = $crate::FsResult<usize, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::write(self, node, offset, buf)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::truncate`]."] $pub)?
        fn truncate(
            &mut self,
            node: $crate::NodeId,
            len: u64,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::truncate(self, node, len)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::fsync`]."] $pub)?
        fn fsync(
            &mut self,
            node: $crate::NodeId,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::fsync(self, node)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::create`]."] $pub)?
        fn create(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
            attrs: &$crate::SetAttr,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<$crate::NodeId, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::create(self, dir, name, attrs)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::mkdir`]."] $pub)?
        fn mkdir(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
            attrs: &$crate::SetAttr,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<$crate::NodeId, <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::mkdir(self, dir, name, attrs)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::unlink`]."] $pub)?
        fn unlink(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::unlink(self, dir, name)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::rmdir`]."] $pub)?
        fn rmdir(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::rmdir(self, dir, name)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::rename`]."] $pub)?
        fn rename(
            &mut self,
            from_dir: $crate::NodeId,
            from: &$crate::Name,
            to_dir: $crate::NodeId,
            to: &$crate::Name,
            mode: $crate::RenameMode,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::rename(self, from_dir, from, to_dir, to, mode)
        }
        $(#[doc = "Delegates to [`hadris_fs::async_::FileSystem::sync`]."] $pub)?
        fn sync(
            &mut self,
        ) -> impl core::future::Future<
            Output = $crate::FsResult<(), <Self as $crate::async_::FileSystem>::DeviceError>,
        > $($send)* {
            $crate::async_::FileSystem::sync(self)
        }
    };
    (@inherent [$($async:tt)?] [$($await:tt)*] read) => {
        fn capabilities(&self) -> $crate::Capabilities {
            Self::capabilities(self)
        }
        fn root(&self) -> $crate::NodeId {
            Self::root(self)
        }
        $($async)? fn statfs(
            &mut self,
        ) -> $crate::FsResult<$crate::FsStats, D::Error> {
            Self::statfs(self)$($await)*
        }
        $($async)? fn label<'b>(
            &mut self,
            buf: &'b mut [u8],
        ) -> $crate::FsResult<Option<&'b str>, D::Error> {
            Self::label(self, buf)$($await)*
        }
        $($async)? fn lookup(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
        ) -> $crate::FsResult<$crate::NodeId, D::Error> {
            Self::lookup(self, dir, name)$($await)*
        }
        fn forget(&mut self, node: $crate::NodeId, count: u64) {
            Self::forget(self, node, count)
        }
        $($async)? fn parent(
            &mut self,
            dir: $crate::NodeId,
        ) -> $crate::FsResult<$crate::NodeId, D::Error> {
            Self::parent(self, dir)$($await)*
        }
        $($async)? fn stat(
            &mut self,
            node: $crate::NodeId,
        ) -> $crate::FsResult<$crate::Metadata, D::Error> {
            Self::stat(self, node)$($await)*
        }
        $($async)? fn readdir(
            &mut self,
            dir: $crate::NodeId,
            from: $crate::DirCursor,
        ) -> $crate::FsResult<Option<$crate::DirEntry>, D::Error> {
            Self::readdir(self, dir, from)$($await)*
        }
        $($async)? fn readlink<'b>(
            &mut self,
            node: $crate::NodeId,
            buf: &'b mut [u8],
        ) -> $crate::FsResult<&'b [u8], D::Error> {
            Self::readlink(self, node, buf)$($await)*
        }
        $($async)? fn open(
            &mut self,
            node: $crate::NodeId,
            mode: $crate::OpenMode,
        ) -> $crate::FsResult<(), D::Error> {
            Self::open(self, node, mode)$($await)*
        }
        $($async)? fn close(
            &mut self,
            node: $crate::NodeId,
        ) -> $crate::FsResult<(), D::Error> {
            Self::close(self, node)$($await)*
        }
        $($async)? fn read(
            &mut self,
            node: $crate::NodeId,
            offset: u64,
            buf: &mut [u8],
        ) -> $crate::FsResult<usize, D::Error> {
            Self::read(self, node, offset, buf)$($await)*
        }
    };
    (@inherent [$($async:tt)?] [$($await:tt)*] resolve) => {
        $($async)? fn resolve(
            &mut self,
            path: &[u8],
            how: $crate::Resolve,
        ) -> $crate::FsResult<$crate::NodeId, D::Error> {
            Self::resolve(self, path, how)$($await)*
        }
    };
    (@inherent [$($async:tt)?] [$($await:tt)*] write) => {
        $($async)? fn setattr(
            &mut self,
            node: $crate::NodeId,
            changes: &$crate::SetAttr,
        ) -> $crate::FsResult<(), D::Error> {
            Self::setattr(self, node, changes)$($await)*
        }
        $($async)? fn write(
            &mut self,
            node: $crate::NodeId,
            offset: u64,
            buf: &[u8],
        ) -> $crate::FsResult<usize, D::Error> {
            Self::write(self, node, offset, buf)$($await)*
        }
        $($async)? fn truncate(
            &mut self,
            node: $crate::NodeId,
            len: u64,
        ) -> $crate::FsResult<(), D::Error> {
            Self::truncate(self, node, len)$($await)*
        }
        $($async)? fn fsync(
            &mut self,
            node: $crate::NodeId,
        ) -> $crate::FsResult<(), D::Error> {
            Self::fsync(self, node)$($await)*
        }
        $($async)? fn create(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
            attrs: &$crate::SetAttr,
        ) -> $crate::FsResult<$crate::NodeId, D::Error> {
            Self::create(self, dir, name, attrs)$($await)*
        }
        $($async)? fn mkdir(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
            attrs: &$crate::SetAttr,
        ) -> $crate::FsResult<$crate::NodeId, D::Error> {
            Self::mkdir(self, dir, name, attrs)$($await)*
        }
        $($async)? fn unlink(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
        ) -> $crate::FsResult<(), D::Error> {
            Self::unlink(self, dir, name)$($await)*
        }
        $($async)? fn rmdir(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
        ) -> $crate::FsResult<(), D::Error> {
            Self::rmdir(self, dir, name)$($await)*
        }
        $($async)? fn rename(
            &mut self,
            from_dir: $crate::NodeId,
            from: &$crate::Name,
            to_dir: $crate::NodeId,
            to: &$crate::Name,
            mode: $crate::RenameMode,
        ) -> $crate::FsResult<(), D::Error> {
            Self::rename(self, from_dir, from, to_dir, to, mode)$($await)*
        }
        $($async)? fn sync(
            &mut self,
        ) -> $crate::FsResult<(), D::Error> {
            Self::sync(self)$($await)*
        }
    };
}
