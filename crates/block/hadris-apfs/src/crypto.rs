use aes::Aes128;
use aes::cipher::KeyInit;
use aes_kw::{KwAes128, KwAes256};
use alloc::vec::Vec;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use xts_mode::{Xts128, get_tweak_default};
use zeroize::{Zeroize, Zeroizing};

use crate::{ApfsError, Result};

pub(crate) const CONTAINER_KEYBAG: u32 = 0x6b65_7973;
pub(crate) const VOLUME_KEYBAG: u32 = 0x7265_6373;
const MAX_KEYBAG_BYTES: usize = 1 << 20;
const MAX_RECORDS: usize = 256;

pub(crate) struct VolumeKey(Zeroizing<[u8; 32]>);

impl core::fmt::Debug for VolumeKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("VolumeKey([REDACTED])")
    }
}

impl VolumeKey {
    pub(crate) fn decrypt(&self, data: &mut [u8], first_sector: u64) -> Result<()> {
        decrypt_xts(&self.0[..16], &self.0[16..], data, first_sector)
    }
}

pub(crate) fn decrypt_keybag(data: &mut [u8], uuid: &[u8; 16], first_sector: u64) -> Result<()> {
    decrypt_xts(uuid, uuid, data, first_sector)
}

fn decrypt_xts(key: &[u8], tweak_key: &[u8], data: &mut [u8], first_sector: u64) -> Result<()> {
    if data.len() % 512 != 0 {
        return Err(ApfsError::InvalidValue("encrypted sector alignment"));
    }
    first_sector
        .checked_add((data.len() / 512) as u64)
        .ok_or(ApfsError::AddressOverflow)?;
    let xts = Xts128::new(
        Aes128::new_from_slice(key).map_err(|_| ApfsError::InvalidValue("XTS key"))?,
        Aes128::new_from_slice(tweak_key).map_err(|_| ApfsError::InvalidValue("XTS tweak key"))?,
    );
    xts.decrypt_area(data, 512, u128::from(first_sector), get_tweak_default);
    Ok(())
}

#[derive(Debug)]
struct Entry<'a> {
    uuid: [u8; 16],
    tag: u16,
    data: &'a [u8],
}

/// @hadris-spec Apple-APFS:Encryption
/// @hadris-compliance partial
/// @hadris-note Validates software keybags and unwraps single-key password records; hardware and per-file keys are unsupported.
pub(crate) struct Keybag<'a> {
    entries: Vec<Entry<'a>>,
}

impl<'a> Keybag<'a> {
    pub(crate) fn parse(data: &'a [u8], expected_type: u32) -> Result<Self> {
        if data.len() < 48 || data.len() > MAX_KEYBAG_BYTES {
            return Err(ApfsError::InvalidValue("keybag size"));
        }
        crate::types::checksum::verify_object(data)?;
        if u32::from_le_bytes(data[24..28].try_into().unwrap()) != expected_type {
            return Err(ApfsError::InvalidValue("keybag object type"));
        }
        if u16::from_le_bytes(data[32..34].try_into().unwrap()) != 2 {
            return Err(ApfsError::Unsupported("keybag version"));
        }
        let count = usize::from(u16::from_le_bytes(data[34..36].try_into().unwrap()));
        let bytes = u32::from_le_bytes(data[36..40].try_into().unwrap()) as usize;
        if count > MAX_RECORDS || bytes < 16 || bytes > data.len() - 32 {
            return Err(ApfsError::InvalidValue("keybag entries length"));
        }
        let locker = &data[32..32 + bytes];
        let mut position = 16usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let header = locker
                .get(position..position + 24)
                .ok_or(ApfsError::InputTooSmall)?;
            let length = usize::from(u16::from_le_bytes(header[18..20].try_into().unwrap()));
            let end = position
                .checked_add(24 + length)
                .ok_or(ApfsError::AddressOverflow)?;
            let body = locker
                .get(position + 24..end)
                .ok_or(ApfsError::InputTooSmall)?;
            entries.push(Entry {
                uuid: header[..16].try_into().unwrap(),
                tag: u16::from_le_bytes(header[16..18].try_into().unwrap()),
                data: body,
            });
            position = end.checked_add(15).ok_or(ApfsError::AddressOverflow)? & !15;
            if position > locker.len() {
                return Err(ApfsError::InputTooSmall);
            }
        }
        Ok(Self { entries })
    }

    fn find(&self, uuid: &[u8; 16], tag: u16) -> Result<&'a [u8]> {
        let mut matches = self
            .entries
            .iter()
            .filter(|entry| &entry.uuid == uuid && entry.tag == tag);
        let first = matches
            .next()
            .ok_or(ApfsError::InvalidValue("missing volume keybag entry"))?;
        if matches.next().is_some() {
            return Err(ApfsError::InvalidValue("duplicate volume keybag entry"));
        }
        Ok(first.data)
    }

    pub(crate) fn volume_records_range(&self, uuid: &[u8; 16]) -> Result<(u64, u64)> {
        let range = self.find(uuid, 3)?;
        if range.len() != 16 {
            return Err(ApfsError::InvalidValue("volume keybag range"));
        }
        let start = u64::from_le_bytes(range[..8].try_into().unwrap());
        let count = u64::from_le_bytes(range[8..].try_into().unwrap());
        if count == 0 {
            return Err(ApfsError::InvalidValue("empty volume keybag range"));
        }
        start.checked_add(count).ok_or(ApfsError::AddressOverflow)?;
        Ok((start, count))
    }
}

struct Der<'a>(&'a [u8]);
impl<'a> Der<'a> {
    fn field(&mut self, tag: u8) -> Result<&'a [u8]> {
        if self.0.len() < 2 {
            return Err(ApfsError::InputTooSmall);
        }
        if self.0[0] != tag {
            return Err(ApfsError::InvalidValue("key blob DER tag"));
        }
        let first = self.0[1];
        let (header, length) = if first < 128 {
            (2, usize::from(first))
        } else {
            let width = usize::from(first & 127);
            if width == 0 || width > 4 || self.0.len() < 2 + width {
                return Err(ApfsError::InvalidValue("key blob DER length"));
            }
            if self.0[2] == 0 {
                return Err(ApfsError::InvalidValue("noncanonical DER length"));
            }
            let mut length = 0usize;
            for byte in &self.0[2..2 + width] {
                length = length
                    .checked_mul(256)
                    .and_then(|v| v.checked_add(usize::from(*byte)))
                    .ok_or(ApfsError::AddressOverflow)?;
            }
            if length < 128 {
                return Err(ApfsError::InvalidValue("noncanonical DER length"));
            }
            (2 + width, length)
        };
        let end = header
            .checked_add(length)
            .ok_or(ApfsError::AddressOverflow)?;
        let value = self.0.get(header..end).ok_or(ApfsError::InputTooSmall)?;
        self.0 = &self.0[end..];
        Ok(value)
    }
    fn integer(&mut self, tag: u8) -> Result<u64> {
        let data = self.field(tag)?;
        if data.is_empty()
            || data.len() > 9
            || data[0] & 128 != 0
            || (data.len() > 1 && data[0] == 0 && data[1] & 128 == 0)
        {
            return Err(ApfsError::InvalidValue("key blob integer"));
        }
        let mut value = 0u64;
        for byte in data {
            value = value
                .checked_mul(256)
                .and_then(|v| v.checked_add(u64::from(*byte)))
                .ok_or(ApfsError::AddressOverflow)?;
        }
        Ok(value)
    }
    fn end(self) -> Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(ApfsError::InvalidValue("trailing key blob data"))
        }
    }
}

struct KeyRecord<'a> {
    uuid: [u8; 16],
    flags: u32,
    wrapped: &'a [u8],
    iterations: u64,
    salt: &'a [u8],
}
fn record(data: &[u8], is_kek: bool) -> Result<KeyRecord<'_>> {
    let mut outer = Der(data);
    let mut sequence = Der(outer.field(0x30)?);
    outer.end()?;
    sequence.integer(0x80)?;
    let signature = sequence.field(0x81)?;
    let salt = sequence.field(0x82)?;
    if signature.len() != 32 || salt.len() != 8 {
        return Err(ApfsError::InvalidValue("key blob authentication fields"));
    }
    let mut digest = Sha256::new();
    digest.update([1, 22, 32, 23, 21, 5]);
    digest.update(salt);
    let hmac_key = Zeroizing::new(<[u8; 32]>::from(digest.finalize()));
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(hmac_key.as_ref())
        .map_err(|_| ApfsError::InvalidValue("key blob HMAC key"))?;
    mac.update(sequence.0);
    mac.verify_slice(signature)
        .map_err(|_| ApfsError::InvalidValue("key blob authentication"))?;
    let mut payload = Der(sequence.field(0xa3)?);
    sequence.end()?;
    payload.integer(0x80)?;
    let uuid = payload
        .field(0x81)?
        .try_into()
        .map_err(|_| ApfsError::InvalidValue("key blob UUID"))?;
    let info = payload.field(0x82)?;
    if info.len() < 4 || info.len() > 22 {
        return Err(ApfsError::InvalidValue("key blob info"));
    }
    let flags = u32::from_le_bytes(info[..4].try_into().unwrap());
    if flags & !0x12 != 0 {
        return Err(ApfsError::Unsupported("hardware or unknown key wrapping"));
    }
    let wrapped = payload.field(0x83)?;
    let key_length = if flags & 2 != 0 { 16 } else { 32 };
    if wrapped.len() != key_length + 8 && !(key_length == 16 && wrapped.len() == 40) {
        return Err(ApfsError::InvalidValue("wrapped key length"));
    }
    let (iterations, salt) = if is_kek {
        (payload.integer(0x84)?, payload.field(0x85)?)
    } else {
        (0, &[][..])
    };
    if is_kek && salt.len() != 16 {
        return Err(ApfsError::InvalidValue("password salt length"));
    }
    payload.end()?;
    Ok(KeyRecord {
        uuid,
        flags,
        wrapped: &wrapped[..key_length + 8],
        iterations,
        salt,
    })
}

fn unwrap(key: &[u8], wrapped: &[u8], output: &mut [u8]) -> bool {
    match key.len() {
        16 => KwAes128::new_from_slice(key)
            .is_ok_and(|cipher| cipher.unwrap_key(wrapped, output).is_ok()),
        32 => KwAes256::new_from_slice(key)
            .is_ok_and(|cipher| cipher.unwrap_key(wrapped, output).is_ok()),
        _ => false,
    }
}

pub(crate) fn unlock_volume_key(
    container: &Keybag<'_>,
    records: &Keybag<'_>,
    uuid: &[u8; 16],
    crypto_user: Option<&[u8; 16]>,
    password: &[u8],
    max_iterations: u32,
) -> Result<Option<VolumeKey>> {
    let vek_record = record(container.find(uuid, 2)?, false)?;
    if &vek_record.uuid != uuid {
        return Err(ApfsError::InvalidValue("volume key UUID mismatch"));
    }
    let mut budget = u64::from(max_iterations);
    let mut unsupported = None;
    let mut supported = false;
    for entry in &records.entries {
        if entry.tag != 3 || crypto_user.is_some_and(|user| user != &entry.uuid) {
            continue;
        }
        let kek_record = match record(entry.data, true) {
            Ok(record) => record,
            Err(error @ ApfsError::Unsupported(_)) if crypto_user.is_none() => {
                unsupported = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        supported = true;
        if kek_record.uuid != entry.uuid {
            return Err(ApfsError::InvalidValue("crypto user UUID mismatch"));
        }
        if kek_record.iterations == 0 || kek_record.iterations > budget {
            return Err(ApfsError::Unsupported("password derivation work limit"));
        }
        budget -= kek_record.iterations;
        let mut derived = Zeroizing::new([0u8; 32]);
        pbkdf2::pbkdf2_hmac::<Sha256>(
            password,
            kek_record.salt,
            kek_record.iterations as u32,
            derived.as_mut(),
        );
        let mut kek = Zeroizing::new([0u8; 32]);
        let kek_len = if kek_record.flags & 2 != 0 { 16 } else { 32 };
        if !unwrap(&derived[..kek_len], kek_record.wrapped, &mut kek[..kek_len]) {
            continue;
        }
        let mut vek = Zeroizing::new([0u8; 32]);
        let vek_len = if vek_record.flags & 2 != 0 { 16 } else { 32 };
        if !unwrap(&kek[..kek_len], vek_record.wrapped, &mut vek[..vek_len]) {
            return Err(ApfsError::InvalidValue("volume key unwrap integrity"));
        }
        if vek_len == 16 {
            let mut hash = Sha256::new();
            hash.update(&vek[..16]);
            hash.update(vek_record.uuid);
            let mut digest = hash.finalize();
            vek[16..].copy_from_slice(&digest[..16]);
            digest.zeroize();
        }
        return Ok(Some(VolumeKey(vek)));
    }
    if !supported {
        if let Some(error) = unsupported {
            return Err(error);
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(input: &str) -> Vec<u8> {
        input
            .as_bytes()
            .chunks_exact(2)
            .map(|c| u8::from_str_radix(core::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn rfc3394_unwrap_vector_and_integrity() {
        let key = hex("000102030405060708090a0b0c0d0e0f");
        let mut wrapped = hex("1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5");
        let mut output = [0u8; 16];
        assert!(unwrap(&key, &wrapped, &mut output));
        assert_eq!(output.as_slice(), hex("00112233445566778899aabbccddeeff"));
        wrapped[0] ^= 1;
        assert!(!unwrap(&key, &wrapped, &mut output));
    }
    #[test]
    fn pbkdf2_sha256_known_answer() {
        let mut output = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(b"password", b"salt", 1, &mut output);
        assert_eq!(
            output.as_slice(),
            hex("120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b")
        );
    }
    #[test]
    fn ieee_xts_aes_128_vector_4() {
        let key = hex("2718281828459045235360287471352631415926535897932384626433832795");
        let mut data = hex(
            "27a7479befa1d476489f308cd4cfa6e2a96e4bbe3208ff25287dd3819616e89cc78cf7f5e543445f8333d8fa7f56000005279fa5d8b5e4ad40e736ddb4d35412328063fd2aab53e5ea1e0a9f332500a5df9487d07a5c92cc512c8866c7e860ce93fdf166a24912b422976146ae20ce846bb7dc9ba94a767aaef20c0d61ad02655ea92dc4c4e41a8952c651d33174be51a10c421110e6d81588ede82103a252d8a750e8768defffed9122810aaeb99f9172af82b604dc4b8e51bcb08235a6f4341332e4ca60482a4ba1a03b3e65008fc5da76b70bf1690db4eae29c5f1badd03c5ccf2a55d705ddcd86d449511ceb7ec30bf12b1fa35b913f9f747a8afd1b130e94bff94effd01a91735ca1726acd0b197c4e5b03393697e126826fb6bbde8ecc1e08298516e2c9ed03ff3c1b7860f6de76d4cecd94c8119855ef5297ca67e9f3e7ff72b1e99785ca0a7e7720c5b36dc6d72cac9574c8cbbc2f801e23e56fd344b07f22154beba0f08ce8891e643ed995c94d9a69c9f1b5f499027a78572aeebd74d20cc39881c213ee770b1010e4bea718846977ae119f7a023ab58cca0ad752afe656bb3c17256a9f6e9bf19fdd5a38fc82bbe872c5539edb609ef4f79c203ebb140f2e583cb2ad15b4aa5b655016a8449277dbd477ef2c8d6c017db738b18deb4a427d1923ce3ff262735779a418f20a282df920147beabe421ee5319d0568",
        );
        decrypt_xts(&key[..16], &key[16..], &mut data, 0).unwrap();
        assert!(data.iter().enumerate().all(|(i, b)| *b == i as u8));
    }
    #[test]
    fn invalid_xts_ranges_do_not_change_output() {
        let mut data = [7u8; 511];
        assert!(decrypt_keybag(&mut data, &[0; 16], 0).is_err());
        assert_eq!(data, [7; 511]);
        let mut data = [7u8; 512];
        assert!(decrypt_keybag(&mut data, &[0; 16], u64::MAX).is_err());
        assert_eq!(data, [7; 512]);
    }
    #[test]
    fn bounded_der_rejects_bad_lengths_and_integers() {
        for input in [
            &[0x30, 0x80][..],
            &[0x30, 0x85, 1, 2, 3, 4, 5],
            &[0x30, 0x81, 1, 0],
            &[0x30, 0x82, 0, 128],
        ] {
            assert!(Der(input).field(0x30).is_err());
        }
        for input in [&[0x80, 0][..], &[0x80, 1, 0xff], &[0x80, 2, 0, 1]] {
            assert!(Der(input).integer(0x80).is_err());
        }
    }
    fn seal_keybag(data: &mut [u8]) {
        let checksum = crate::types::checksum::fletcher64(data).unwrap();
        data[..8].copy_from_slice(&checksum.to_le_bytes());
    }
    fn empty_keybag() -> Vec<u8> {
        let mut data = alloc::vec![0;512];
        data[24..28].copy_from_slice(&CONTAINER_KEYBAG.to_le_bytes());
        data[32..34].copy_from_slice(&2u16.to_le_bytes());
        data[36..40].copy_from_slice(&16u32.to_le_bytes());
        seal_keybag(&mut data);
        data
    }
    #[test]
    fn keybag_bounds_and_checksum() {
        let mut data = empty_keybag();
        assert!(Keybag::parse(&data, CONTAINER_KEYBAG).is_ok());
        data[34..36].copy_from_slice(&257u16.to_le_bytes());
        seal_keybag(&mut data);
        assert!(Keybag::parse(&data, CONTAINER_KEYBAG).is_err());
        data = empty_keybag();
        data[36..40].copy_from_slice(&513u32.to_le_bytes());
        seal_keybag(&mut data);
        assert!(Keybag::parse(&data, CONTAINER_KEYBAG).is_err());
        data = empty_keybag();
        data[34..36].copy_from_slice(&1u16.to_le_bytes());
        seal_keybag(&mut data);
        assert!(Keybag::parse(&data, CONTAINER_KEYBAG).is_err());
        data = empty_keybag();
        data[100] ^= 1;
        assert!(Keybag::parse(&data, CONTAINER_KEYBAG).is_err());
        assert!(Keybag::parse(&alloc::vec![0;(1<<20)+4], CONTAINER_KEYBAG).is_err());
    }
    fn tlv(tag: u8, data: &[u8]) -> Vec<u8> {
        let mut output = alloc::vec![tag];
        if data.len() < 128 {
            output.push(data.len() as u8);
        } else {
            output.extend_from_slice(&[0x81, data.len() as u8]);
        }
        output.extend_from_slice(data);
        output
    }
    fn key_blob(payload: &[u8]) -> Vec<u8> {
        let body = tlv(0xa3, payload);
        let key: [u8; 32] = Sha256::digest([1, 22, 32, 23, 21, 5, 0, 0, 0, 0, 0, 0, 0, 0]).into();
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).unwrap();
        mac.update(&body);
        let signature = mac.finalize().into_bytes();
        tlv(
            0x30,
            &[
                tlv(0x80, &[0]),
                tlv(0x81, &signature),
                tlv(0x82, &[0; 8]),
                body,
            ]
            .concat(),
        )
    }
    #[test]
    fn authenticated_records_reject_tampering_and_hardware_keys() {
        let payload = [
            tlv(0x80, &[0]),
            tlv(0x81, &[0; 16]),
            tlv(0x82, &[0; 22]),
            tlv(0x83, &[0; 40]),
        ]
        .concat();
        let mut data = key_blob(&payload);
        assert!(record(&data, false).is_ok());
        let length = data.len();
        data[length - 1] ^= 1;
        assert!(record(&data, false).is_err());
        let payload = [
            tlv(0x80, &[0]),
            tlv(0x81, &[0; 16]),
            tlv(0x82, &[1; 22]),
            tlv(0x83, &[0; 40]),
        ]
        .concat();
        assert!(matches!(
            record(&key_blob(&payload), false),
            Err(ApfsError::Unsupported(_))
        ));
    }
    #[test]
    fn password_work_budget_and_user_selection() {
        let uuid = [4; 16];
        let user = [5; 16];
        let vek_data = key_blob(
            &[
                tlv(0x80, &[0]),
                tlv(0x81, &uuid),
                tlv(0x82, &[0; 22]),
                tlv(0x83, &[0; 40]),
            ]
            .concat(),
        );
        let kek_data = key_blob(
            &[
                tlv(0x80, &[0]),
                tlv(0x81, &user),
                tlv(0x82, &[0; 22]),
                tlv(0x83, &[0; 40]),
                tlv(0x84, &[0x7f, 0xff, 0xff, 0xff]),
                tlv(0x85, &[0; 16]),
            ]
            .concat(),
        );
        let container = Keybag {
            entries: alloc::vec![Entry {
                uuid,
                tag: 2,
                data: &vek_data
            }],
        };
        let records = Keybag {
            entries: alloc::vec![Entry {
                uuid: user,
                tag: 3,
                data: &kek_data
            }],
        };
        assert!(matches!(
            unlock_volume_key(&container, &records, &uuid, None, b"", 100),
            Err(ApfsError::Unsupported(_))
        ));
        assert!(
            unlock_volume_key(&container, &records, &uuid, Some(&[6; 16]), b"", 100)
                .unwrap()
                .is_none()
        );
    }
    fn wrapped_blob(uuid: &[u8; 16], flags: u8, wrapped: &[u8], iterations: Option<u8>) -> Vec<u8> {
        let mut info = [0; 22];
        info[0] = flags;
        let mut payload = [
            tlv(0x80, &[0]),
            tlv(0x81, uuid),
            tlv(0x82, &info),
            tlv(0x83, wrapped),
        ]
        .concat();
        if let Some(iterations) = iterations {
            payload.extend(tlv(0x84, &[iterations]));
            payload.extend(tlv(0x85, &[0; 16]));
        }
        key_blob(&payload)
    }
    #[test]
    fn mixed_crypto_users_skip_unsupported_but_enforce_integrity_and_total_work() {
        let uuid = [4; 16];
        let user = [5; 16];
        let hardware_user = [6; 16];
        let mut derived = [0; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(b"password", &[0; 16], 1, &mut derived);
        let kek = [7; 32];
        let vek = [8; 32];
        let mut wrapped_kek = [0; 40];
        let mut wrapped_vek = [0; 40];
        KwAes256::new_from_slice(&derived)
            .unwrap()
            .wrap_key(&kek, &mut wrapped_kek)
            .unwrap();
        KwAes256::new_from_slice(&kek)
            .unwrap()
            .wrap_key(&vek, &mut wrapped_vek)
            .unwrap();
        let vek_data = wrapped_blob(&uuid, 0, &wrapped_vek, None);
        let supported_data = wrapped_blob(&user, 0, &wrapped_kek, Some(1));
        let mut hardware_data = wrapped_blob(&hardware_user, 1, &wrapped_kek, Some(1));
        let container = Keybag {
            entries: alloc::vec![Entry {
                uuid,
                tag: 2,
                data: &vek_data
            }],
        };
        {
            let records = Keybag {
                entries: alloc::vec![
                    Entry {
                        uuid: hardware_user,
                        tag: 3,
                        data: &hardware_data
                    },
                    Entry {
                        uuid: user,
                        tag: 3,
                        data: &supported_data
                    }
                ],
            };
            let key = unlock_volume_key(&container, &records, &uuid, None, b"password", 1)
                .unwrap()
                .unwrap();
            assert_eq!(key.0.as_ref(), &vek);
            assert!(matches!(
                unlock_volume_key(
                    &container,
                    &records,
                    &uuid,
                    Some(&hardware_user),
                    b"password",
                    1
                ),
                Err(ApfsError::Unsupported(_))
            ));
            assert!(
                unlock_volume_key(&container, &records, &uuid, None, b"incorrect", 1)
                    .unwrap()
                    .is_none()
            );
        }
        {
            let records = Keybag {
                entries: alloc::vec![Entry {
                    uuid: hardware_user,
                    tag: 3,
                    data: &hardware_data
                }],
            };
            assert!(matches!(
                unlock_volume_key(&container, &records, &uuid, None, b"password", 1),
                Err(ApfsError::Unsupported(_))
            ));
        }
        let final_byte = hardware_data.len() - 1;
        hardware_data[final_byte] ^= 1;
        let records = Keybag {
            entries: alloc::vec![
                Entry {
                    uuid: hardware_user,
                    tag: 3,
                    data: &hardware_data
                },
                Entry {
                    uuid: user,
                    tag: 3,
                    data: &supported_data
                }
            ],
        };
        assert!(matches!(
            unlock_volume_key(&container, &records, &uuid, None, b"password", 1),
            Err(ApfsError::InvalidValue("key blob authentication"))
        ));
        let records = Keybag {
            entries: alloc::vec![
                Entry {
                    uuid: user,
                    tag: 3,
                    data: &supported_data
                },
                Entry {
                    uuid: user,
                    tag: 3,
                    data: &supported_data
                }
            ],
        };
        assert!(matches!(
            unlock_volume_key(&container, &records, &uuid, None, b"incorrect", 1),
            Err(ApfsError::Unsupported("password derivation work limit"))
        ));
    }
    #[cfg(feature = "std")]
    #[test]
    fn native_password_keybags() {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(path) = std::env::var("HADRIS_APFS_ENCRYPTED_IMAGE") else {
            return;
        };
        let mut file = std::fs::File::open(path).unwrap();
        let mut header = [0u8; 4096];
        file.read_exact(&mut header).unwrap();
        let uuid: [u8; 16] = header[72..88].try_into().unwrap();
        let start = u64::from_le_bytes(header[1296..1304].try_into().unwrap());
        let count = u64::from_le_bytes(header[1304..1312].try_into().unwrap());
        let mut data = alloc::vec![0; count as usize * 4096];
        file.seek(SeekFrom::Start(start * 4096)).unwrap();
        file.read_exact(&mut data).unwrap();
        decrypt_keybag(&mut data, &uuid, start * 8).unwrap();
        let bag = Keybag::parse(&data, CONTAINER_KEYBAG).unwrap();
        let vuuid = bag
            .entries
            .iter()
            .find(|entry| entry.tag == 2)
            .unwrap()
            .uuid;
        let (start, count) = bag.volume_records_range(&vuuid).unwrap();
        let mut records_data = alloc::vec![0;count as usize*4096];
        file.seek(SeekFrom::Start(start * 4096)).unwrap();
        file.read_exact(&mut records_data).unwrap();
        decrypt_keybag(&mut records_data, &vuuid, start * 8).unwrap();
        let records = Keybag::parse(&records_data, VOLUME_KEYBAG).unwrap();
        assert!(
            unlock_volume_key(&bag, &records, &vuuid, None, b"incorrect", 1_000_000)
                .unwrap()
                .is_none()
        );
        let key = unlock_volume_key(
            &bag,
            &records,
            &vuuid,
            None,
            b"hadris-public-fixture-password",
            1_000_000,
        )
        .unwrap()
        .unwrap();
        let mut empty = [];
        key.decrypt(&mut empty, 0).unwrap();
    }
}
