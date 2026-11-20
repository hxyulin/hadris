use crate::error::Detail;

pub(crate) const CHUNK_BYTES: usize = 4096;

pub(crate) struct Chunk {
    pub(crate) len: usize,
    compressed: bool,
}

impl Chunk {
    pub(crate) fn header(word: u16) -> Result<Option<Self>, Detail> {
        if word == 0 {
            return Ok(None);
        }
        if word & 0x7000 != 0x3000 {
            return Err(Detail::CompressionData);
        }
        Ok(Some(Self {
            len: usize::from(word & 0x0FFF) + 1,
            compressed: word & 0x8000 != 0,
        }))
    }

    pub(crate) fn decode(
        &self,
        input: &[u8],
        output: &mut [u8; CHUNK_BYTES],
    ) -> Result<(), Detail> {
        if input.len() != self.len {
            return Err(Detail::CompressionData);
        }
        output.fill(0);
        if !self.compressed {
            output[..input.len()].copy_from_slice(input);
            return Ok(());
        }
        let mut source = input;
        let mut written = 0usize;
        while let Some((&flags, rest)) = source.split_first() {
            source = rest;
            for bit in 0..8 {
                let Some(&byte) = source.first() else { break };
                if flags & (1 << bit) == 0 {
                    *output.get_mut(written).ok_or(Detail::CompressionData)? = byte;
                    written += 1;
                    source = &source[1..];
                    continue;
                }
                let second = *source.get(1).ok_or(Detail::CompressionData)?;
                let word = usize::from(u16::from_le_bytes([byte, second]));
                source = &source[2..];
                let mut shift = 12;
                let mut position = written.saturating_sub(1);
                while position >= 16 {
                    shift -= 1;
                    position >>= 1;
                }
                let distance = (word >> shift) + 1;
                let len = (word & ((1 << shift) - 1)) + 3;
                if distance > written || written + len > output.len() {
                    return Err(Detail::CompressionData);
                }
                for i in written..written + len {
                    output[i] = output[i - distance];
                }
                written += len;
            }
        }
        Ok(())
    }
}
