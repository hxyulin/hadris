use std::mem::MaybeUninit;

pub struct ArrayVec<T, const N: usize> {
    data: [MaybeUninit<T>; N],
    len: usize,
}

impl<T, const N: usize> ArrayVec<T, N> {
    pub const fn new() -> Self {
        Self {
            data: [const { MaybeUninit::uninit() }; N],
            len: 0,
        }
    }
}

#[cfg(all(feature = "std", test))]
mod tests {
    use super::*;

    #[test]
    fn test_vec_push() {
        let vec = ArrayVec::<u8, 16>::new();
    }
}
