use super::*;
use crate::{WalkEntry, WalkFrame};

/// The deepest level `Walk::new` reaches.
#[cfg(feature = "alloc")]
const MAX_DEPTH: usize = 1024;

io_transform! {

/// A depth-first walk below a directory on the bare driver: each
/// directory's entry comes before its children, which are listed in
/// `readdir` order.
///
/// The walk pins nothing. It lists directories by the ids `readdir`
/// returned, which stay valid while the tree does not change; a change
/// during the walk may give [`ErrorKind::NotFound`]. A directory whose id
/// equals one of its ancestors' fails with [`ErrorKind::Corrupt`], so a
/// loop in a damaged image ends instead of repeating. Going deeper than
/// the stack allows fails with [`ErrorKind::LimitExceeded`]. After an
/// error `next` returns `None`.
#[derive(Debug)]
pub struct Walk<'s> {
    frames: Frames<'s>,
    len: usize,
    /// The directory `next` just returned, to descend into.
    pending: Option<NodeId>,
    done: bool,
}

#[derive(Debug)]
enum Frames<'s> {
    Lent(&'s mut [WalkFrame]),
    #[cfg(feature = "alloc")]
    Heap(alloc::vec::Vec<WalkFrame>),
}

impl Frames<'_> {
    fn capacity(&self) -> usize {
        match self {
            Self::Lent(frames) => frames.len(),
            #[cfg(feature = "alloc")]
            Self::Heap(_) => MAX_DEPTH,
        }
    }

    fn get(&self, index: usize) -> Option<(NodeId, DirCursor)> {
        match self {
            Self::Lent(frames) => frames.get(index).and_then(|frame| frame.0),
            #[cfg(feature = "alloc")]
            Self::Heap(frames) => frames.get(index).and_then(|frame| frame.0),
        }
    }

    fn set(&mut self, index: usize, frame: (NodeId, DirCursor)) {
        match self {
            Self::Lent(frames) => frames[index] = WalkFrame(Some(frame)),
            #[cfg(feature = "alloc")]
            Self::Heap(frames) => {
                if index == frames.len() {
                    frames.push(WalkFrame(Some(frame)));
                } else {
                    frames[index] = WalkFrame(Some(frame));
                }
            }
        }
    }
}

#[cfg(feature = "alloc")]
impl Walk<'static> {
    /// Walks everything below `dir`, which is not itself returned, on a
    /// heap stack of up to 1024 levels.
    pub fn new(dir: NodeId) -> Self {
        let mut frames = Frames::Heap(alloc::vec::Vec::new());
        frames.set(0, (dir, DirCursor::START));
        Self {
            frames,
            len: 1,
            pending: None,
            done: false,
        }
    }
}

impl<'s> Walk<'s> {
    /// Walks everything below `dir` on the caller's stack, one level per
    /// frame, without allocating. An empty stack fails the first `next`
    /// with [`ErrorKind::LimitExceeded`].
    pub fn with_stack(dir: NodeId, stack: &'s mut [WalkFrame]) -> Self {
        let mut frames = Frames::Lent(stack);
        let len = if frames.capacity() == 0 {
            0
        } else {
            frames.set(0, (dir, DirCursor::START));
            1
        };
        Self {
            frames,
            len,
            pending: None,
            done: false,
        }
    }

    /// Stops the walk from descending into the directory `next` just
    /// returned.
    pub fn skip_dir(&mut self) {
        self.pending = None;
    }

    /// The next entry, or `None` when the walk is done.
    pub async fn next<F: FileSystem + ?Sized>(&mut self, fs: &mut F) -> FsResult<Option<WalkEntry>, F::DeviceError> {
        if self.done {
            return Ok(None);
        }
        match self.step(fs).await {
            Ok(Some(entry)) => Ok(Some(entry)),
            other => {
                self.done = true;
                other
            }
        }
    }

    async fn step<F: FileSystem + ?Sized>(&mut self, fs: &mut F) -> FsResult<Option<WalkEntry>, F::DeviceError> {
        if self.len == 0 {
            return Err(ErrorKind::LimitExceeded.into());
        }
        if let Some(dir) = self.pending.take() {
            if (0..self.len).any(|index| self.frames.get(index).is_some_and(|(id, _)| id == dir)) {
                return Err(ErrorKind::Corrupt.into());
            }
            if self.len >= self.frames.capacity() {
                return Err(ErrorKind::LimitExceeded.into());
            }
            self.frames.set(self.len, (dir, DirCursor::START));
            self.len += 1;
        }
        while self.len > 0 {
            let top = self.len - 1;
            let Some((dir, cursor)) = self.frames.get(top) else {
                return Err(ErrorKind::Corrupt.into());
            };
            match fs.readdir(dir, cursor).await? {
                Some(entry) => {
                    self.frames.set(top, (dir, entry.next_cursor()));
                    if entry.file_type() == FileType::Dir {
                        self.pending = Some(entry.node());
                    }
                    return Ok(Some(WalkEntry::new(entry, self.len as u32)));
                }
                None => self.len -= 1,
            }
        }
        Ok(None)
    }
}

}
