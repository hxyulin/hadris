//! APFS Fletcher-64 checksum helpers.

use crate::types::le_u64;

/// Computes the APFS Fletcher-64 checksum over an object block.
///
/// APFS stores the checksum in the first eight bytes; those bytes are skipped
/// for checksum purposes.
///
/// # Errors
///
/// Returns [`ApfsError::InvalidValue`](crate::ApfsError::InvalidValue) if
/// the input is shorter than eight bytes or its length is not divisible by four.
pub fn fletcher64(data: &[u8]) -> crate::Result<u64> {
    if data.len() < 8 || data.len() % 4 != 0 {
        return Err(crate::ApfsError::InvalidValue("checksum block length"));
    }

    let mut lo: u64 = 0;
    let mut hi: u64 = 0;
    let (words, _) = data[8..].as_chunks::<4>();
    for &word in words {
        let value = u64::from(u32::from_le_bytes(word));
        lo = lo.wrapping_add(value);
        hi = hi.wrapping_add(lo);
    }

    let c1 = 0xffff_ffffu64.wrapping_sub((lo.wrapping_add(hi)) % 0xffff_ffff);
    let c2 = 0xffff_ffffu64.wrapping_sub((lo.wrapping_add(c1)) % 0xffff_ffff);
    Ok((c2 << 32) | c1)
}

/// Verifies the APFS object checksum in `data`.
///
/// # Errors
///
/// Returns [`ApfsError::InputTooSmall`](crate::ApfsError::InputTooSmall) for
/// input shorter than eight bytes, a length error as [`fletcher64`] does for
/// input not divisible by four, or
/// [`ApfsError::ChecksumMismatch`](crate::ApfsError::ChecksumMismatch)
/// when the stored checksum differs from the computed value.
pub fn verify_object(data: &[u8]) -> crate::Result<()> {
    let expected = le_u64(data, 0)?;
    let actual = fletcher64(data)?;
    if expected == actual {
        Ok(())
    } else {
        Err(crate::ApfsError::ChecksumMismatch { expected, actual })
    }
}

#[cfg(test)]
mod tests {
    use super::{fletcher64, verify_object};
    use crate::ApfsError;

    #[test]
    fn checksum_length_and_stored_prefix_follow_the_object_contract() {
        for len in [0, 4, 7, 9, 10, 11, 13] {
            assert!(matches!(
                fletcher64(&[0; 16][..len]),
                Err(ApfsError::InvalidValue("checksum block length"))
            ));
        }
        let mut data = [0u8; 12];
        data[8..].copy_from_slice(&1u32.to_le_bytes());
        let expected = 0x0000_0001_ffff_fffdu64;
        assert_eq!(fletcher64(&data).unwrap(), expected);
        data[..8].copy_from_slice(&expected.to_le_bytes());
        assert_eq!(fletcher64(&data).unwrap(), expected);
        verify_object(&data).unwrap();
        data[8] = 2;
        assert!(matches!(
            verify_object(&data),
            Err(ApfsError::ChecksumMismatch { .. })
        ));
    }
}
