//! Encryption at rest (`HBAMe` files): FORMAT.md, "Encryption at rest".
//!
//! Read-only: [`decrypt`] turns an encrypted image into the plain `HBAM7`
//! image FileMaker's own "remove encryption" writes, in memory.

use crate::{Error, SECTOR};
use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::NoPadding};
use hmac::{KeyInit, Mac};

/// Header fields (sector 0).
const CIPHER: usize = 48;
const KDF: usize = 49;
const HINT: std::ops::Range<usize> = 50..250;
const SALT: std::ops::Range<usize> = 250..266;
const CHECK: std::ops::Range<usize> = 266..298;
/// Then the shared id (UTF-16LE, NUL-terminated) from 298; a plain
/// header is all zeros up to 512.
const FIELDS_END: usize = 512;

/// Cipher byte: AES-256-CBC pages (1 would be AES-128, never seen).
const AES_256: u8 = 2;
/// KDF byte: PBKDF2-HMAC-SHA1.
const PBKDF2_SHA1: u8 = 1;
const CHECK_ROUNDS: u32 = 20_000;
const KEY_ROUNDS: u32 = 18_000;
/// Each page: ciphertext, then its IV, then a truncated HMAC-SHA256.
const BODY: usize = SECTOR - 32;

pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.get(15..20) == Some(b"HBAMe")
}

fn utf16z(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// The password hint stored in the header, if any.
pub fn hint(bytes: &[u8]) -> Option<String> {
    let h = utf16z(bytes.get(HINT)?);
    (!h.is_empty()).then_some(h)
}

/// FileMaker hashes the password as UTF-16LE after canonical decomposition
/// (NFD), case kept.
fn password_bytes(password: &str) -> Vec<u8> {
    icu_normalizer::DecomposingNormalizerBorrowed::new_nfd()
        .normalize(password)
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Decrypts an `HBAMe` image with its encryption password.
pub fn decrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, Error> {
    if bytes.len() < 2 * SECTOR || !bytes.len().is_multiple_of(SECTOR) {
        return Err(Error::BadSize);
    }
    if !is_encrypted(bytes) {
        return Err(Error::NotFmp12);
    }
    if bytes[CIPHER] != AES_256 || bytes[KDF] != PBKDF2_SHA1 {
        return Err(Error::Security(format!(
            "unsupported encryption (cipher {}, key derivation {})",
            bytes[CIPHER], bytes[KDF]
        )));
    }
    let pw = password_bytes(password);
    let salt = &bytes[SALT];
    let check = pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 32>(&pw, salt, CHECK_ROUNDS);
    // Not secret (it is in the header), so a plain comparison is fine.
    if check[..] != bytes[CHECK] {
        return Err(Error::EncryptionPassword);
    }
    // Two 32-byte keys, 37 bytes apart in one PBKDF2 output: AES, then HMAC.
    let keys = pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 74>(&pw, salt, KEY_ROUNDS);
    let (aes_key, mac_key) = (&keys[..32], &keys[37..69]);

    let mut out = Vec::with_capacity(bytes.len());
    // Header: FileMaker's decrypted copy says HBAM7 and clears the fields.
    let mut head = bytes[..SECTOR].to_vec();
    head[15..20].copy_from_slice(b"HBAM7");
    head[CIPHER..FIELDS_END].fill(0);
    out.extend_from_slice(&head);

    for (i, page) in bytes.as_chunks::<SECTOR>().0.iter().enumerate().skip(1) {
        let bad = || Error::Sector {
            id: i as u32,
            source: Box::new(Error::Security(
                "damaged encrypted page (integrity check failed)".into(),
            )),
        };
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(mac_key).map_err(|_| bad())?;
        mac.update(&page[..BODY + 16]);
        mac.verify_truncated_left(&page[BODY + 16..])
            .map_err(|_| bad())?;
        let mut body = page[..BODY].to_vec();
        cbc::Decryptor::<aes::Aes256>::new_from_slices(aes_key, &page[BODY..BODY + 16])
            .map_err(|_| bad())?
            .decrypt_padded::<NoPadding>(&mut body)
            .map_err(|_| bad())?;
        // The page's payload is 32 bytes shorter than a plain sector's:
        // its free-byte count (offset 14) grows by the padding.
        let free = u16::from_be_bytes([body[14], body[15]]).saturating_add(32);
        body[14..16].copy_from_slice(&free.to_be_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&[0; 32]);
    }
    Ok(out)
}
