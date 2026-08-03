//! APFS Fletcher-64 checksum helpers.

use crate::types::le_u64;

/// Computes the APFS Fletcher-64 checksum over an object block.
///
/// APFS stores the checksum in the first eight bytes; those bytes are skipped
/// for checksum purposes.
pub fn fletcher64(data: &[u8]) -> crate::Result<u64> {
    if data.len() < 8 || !data.len().is_multiple_of(4) {
        return Err(crate::ApfsError::InvalidValue("checksum block length"));
    }

    let mut lo: u64 = 0;
    let mut hi: u64 = 0;
    for chunk in data[8..].chunks_exact(4) {
        let value = u32::from_le_bytes(chunk.try_into().expect("u32 chunk")) as u64;
        lo = lo.wrapping_add(value);
        hi = hi.wrapping_add(lo);
    }

    let c1 = 0xffff_ffffu64.wrapping_sub((lo.wrapping_add(hi)) % 0xffff_ffff);
    let c2 = 0xffff_ffffu64.wrapping_sub((lo.wrapping_add(c1)) % 0xffff_ffff);
    Ok((c2 << 32) | c1)
}

/// Verifies the APFS object checksum in `data`.
pub fn verify_object(data: &[u8]) -> crate::Result<()> {
    let expected = le_u64(data, 0)?;
    let actual = fletcher64(data)?;
    if expected == actual {
        Ok(())
    } else {
        Err(crate::ApfsError::ChecksumMismatch { expected, actual })
    }
}
