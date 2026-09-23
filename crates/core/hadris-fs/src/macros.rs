/// Implements `FsDriver` for a type whose inherent methods have the same
/// names and signatures.
///
/// A format crate writes each node method once, as an inherent method, and
/// raw users call it with no trait import. The first argument is the mode,
/// `sync`, `async` or `async_send`, which a crate's per-mode module passes.
///
/// - The required methods (`capabilities`, `root`, `lookup`,
///   `node_metadata`, `read_dir_entry`, `read_at`, `stats`, `forget`) are
///   always forwarded.
/// - The write methods (`create`, `remove`, `rename`, `write_at`, `set_len`,
///   `set_metadata`, `sync_node`, `sync`) are forwarded unless `read_only` is
///   given, in which case they keep their `ReadOnly` defaults.
/// - The optional methods (`parent`, `read_link`, `resolve`, `open_node`,
///   `close_node`) are forwarded only when named in `also = [..]`, so a
///   driver never forwards a method it lacks.
///
/// Methods added to `FsDriver` after 3.0 join the optional list, never the
/// write list, so a format that uses this macro keeps compiling.
///
/// ```ignore
/// hadris_fs::impl_fs_driver!(sync, impl[D: BlockDevice] FatFs<D>, error = D::Error; also = [parent]);
/// hadris_fs::impl_fs_driver!(async, impl['a, D: BlockDevice] IsoFs<'a, D>, error = D::Error, read_only; also = [parent, read_link]);
/// ```
#[macro_export]
macro_rules! impl_fs_driver {
    (sync, $($rest:tt)*) => {
        $crate::impl_fs_driver!(@parse [] [] [$crate::sync] $($rest)*);
    };
    (async, $($rest:tt)*) => {
        $crate::impl_fs_driver!(@parse [async] [.await] [$crate::r#async] $($rest)*);
    };
    (async_send, $($rest:tt)*) => {
        $crate::impl_fs_driver!(@parse [async] [.await] [$crate::async_send] $($rest)*);
    };
    (@parse [$($as:tt)*] [$($aw:tt)*] [$($m:tt)*]
        impl[$($g:tt)*] $ty:ty, error = $err:ty $(, $ro:ident)? $(; also = [$($also:ident),* $(,)?])?
    ) => {
        impl<$($g)*> $($m)*::FsDriver for $ty {
            type DeviceError = $err;

            fn capabilities(&self) -> $crate::Capabilities {
                <$ty>::capabilities(self)
            }
            fn root(&self) -> $crate::NodeId {
                <$ty>::root(self)
            }
            $($as)* fn lookup(&mut self, dir: $crate::NodeId, name: &$crate::Name) -> $crate::FsResult<$crate::NodeId, $err> {
                <$ty>::lookup(self, dir, name) $($aw)*
            }
            $($as)* fn node_metadata(&mut self, node: $crate::NodeId) -> $crate::FsResult<$crate::Metadata, $err> {
                <$ty>::node_metadata(self, node) $($aw)*
            }
            $($as)* fn read_dir_entry(
                &mut self,
                dir: $crate::NodeId,
                cursor: &mut $crate::DirCursor,
                name: &mut $crate::NameBuf,
            ) -> $crate::FsResult<::core::option::Option<$crate::DirEntry>, $err> {
                <$ty>::read_dir_entry(self, dir, cursor, name) $($aw)*
            }
            $($as)* fn read_at(&mut self, node: $crate::NodeId, offset: u64, buf: &mut [u8]) -> $crate::FsResult<usize, $err> {
                <$ty>::read_at(self, node, offset, buf) $($aw)*
            }
            $($as)* fn stats(&mut self) -> $crate::FsResult<$crate::FsStats, $err> {
                <$ty>::stats(self) $($aw)*
            }
            fn forget(&mut self, node: $crate::NodeId) {
                <$ty>::forget(self, node)
            }
            $crate::impl_fs_driver!(@write [$($as)*] [$($aw)*] $ty, $err $(, $ro)?);
            $crate::impl_fs_driver!(@also_list [$($as)*] [$($aw)*] $ty, $err; $($($also),*)?);
        }
    };
    (@also_list [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty;) => {};
    (@also_list [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty; $first:ident $(, $rest:ident)*) => {
        $crate::impl_fs_driver!(@also [$($as)*] [$($aw)*] $ty, $err, $first);
        $crate::impl_fs_driver!(@also_list [$($as)*] [$($aw)*] $ty, $err; $($rest),*);
    };
    (@write [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty, read_only) => {};
    (@write [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty) => {
        $($as)* fn create(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
            kind: $crate::NewNode<'_>,
            meta: &$crate::SetMetadata,
        ) -> $crate::FsResult<$crate::NodeId, $err> {
            <$ty>::create(self, dir, name, kind, meta) $($aw)*
        }
        $($as)* fn remove(
            &mut self,
            dir: $crate::NodeId,
            name: &$crate::Name,
            kind: $crate::RemoveKind,
        ) -> $crate::FsResult<(), $err> {
            <$ty>::remove(self, dir, name, kind) $($aw)*
        }
        $($as)* fn rename(
            &mut self,
            from_dir: $crate::NodeId,
            from: &$crate::Name,
            to_dir: $crate::NodeId,
            to: &$crate::Name,
            flags: $crate::RenameFlags,
        ) -> $crate::FsResult<(), $err> {
            <$ty>::rename(self, from_dir, from, to_dir, to, flags) $($aw)*
        }
        $($as)* fn write_at(&mut self, node: $crate::NodeId, offset: u64, buf: &[u8]) -> $crate::FsResult<usize, $err> {
            <$ty>::write_at(self, node, offset, buf) $($aw)*
        }
        $($as)* fn set_len(&mut self, node: $crate::NodeId, len: u64) -> $crate::FsResult<(), $err> {
            <$ty>::set_len(self, node, len) $($aw)*
        }
        $($as)* fn set_metadata(&mut self, node: $crate::NodeId, changes: &$crate::SetMetadata) -> $crate::FsResult<(), $err> {
            <$ty>::set_metadata(self, node, changes) $($aw)*
        }
        $($as)* fn sync_node(&mut self, node: $crate::NodeId) -> $crate::FsResult<(), $err> {
            <$ty>::sync_node(self, node) $($aw)*
        }
        $($as)* fn sync(&mut self) -> $crate::FsResult<(), $err> {
            <$ty>::sync(self) $($aw)*
        }
    };
    (@also [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty, parent) => {
        $($as)* fn parent(&mut self, dir: $crate::NodeId) -> $crate::FsResult<$crate::NodeId, $err> {
            <$ty>::parent(self, dir) $($aw)*
        }
    };
    (@also [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty, read_link) => {
        $($as)* fn read_link(&mut self, link: $crate::NodeId, buf: &mut [u8]) -> $crate::FsResult<usize, $err> {
            <$ty>::read_link(self, link, buf) $($aw)*
        }
    };
    (@also [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty, open_node) => {
        $($as)* fn open_node(&mut self, node: $crate::NodeId) -> $crate::FsResult<(), $err> {
            <$ty>::open_node(self, node) $($aw)*
        }
    };
    (@also [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty, close_node) => {
        fn close_node(&mut self, node: $crate::NodeId) {
            <$ty>::close_node(self, node)
        }
    };
    (@also [$($as:tt)*] [$($aw:tt)*] $ty:ty, $err:ty, resolve) => {
        $($as)* fn resolve(&mut self, path: &str) -> $crate::FsResult<$crate::NodeId, $err> {
            <$ty>::resolve(self, path) $($aw)*
        }
    };
}
