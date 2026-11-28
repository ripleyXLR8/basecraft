//! Encryption at rest (`HBAMe` files): FORMAT.md, "Encryption at rest".
//!
//! [`decrypt`] turns an encrypted image into the plain `HBAM7` image
//! FileMaker's own "remove encryption" writes, in memory; [`encrypt`]
//! writes a plain image encrypted, as FileMaker does.

use crate::{Error, SECTOR};
use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding::NoPadding};
use hmac::{KeyInit, Mac};

/// Header fields (sector 0).
const CIPHER: usize = 48;
const KDF: usize = 49;
const HINT: std::ops::Range<usize> = 50..250;
const SALT: std::ops::Range<usize> = 250..266;
const CHECK: std::ops::Range<usize> = 266..298;
/// Then the shared id (UTF-16LE, NUL-terminated) from 298; a plain
/// header is all zeros up to 512.
const SHARED_ID: std::ops::Range<usize> = 298..FIELDS_END;
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

fn put_utf16z(dst: &mut [u8], s: &str) -> Result<(), Error> {
    let b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    // Room for the terminator.
    if b.len() + 2 > dst.len() {
        return Err(Error::Security(format!("too long for the header: {s}")));
    }
    dst[..b.len()].copy_from_slice(&b);
    Ok(())
}

/// The password hint stored in the header, if any.
pub fn hint(bytes: &[u8]) -> Option<String> {
    let h = utf16z(bytes.get(HINT)?);
    (!h.is_empty()).then_some(h)
}

/// How a file is encrypted: its password and header fields (cipher, key
/// derivation, hint, shared id). Kept from the file it was opened from, so
/// that saving writes it encrypted again; the password stays in memory.
#[derive(Clone, PartialEq, Eq)]
pub struct Encryption {
    password: String,
    /// Header bytes 48..512; salt and check value are renewed on write.
    fields: Vec<u8>,
}

impl std::fmt::Debug for Encryption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Encryption { .. }")
    }
}

impl Encryption {
    /// New AES-256 encryption, the header FileMaker's "enable encryption"
    /// writes.
    pub fn new(password: &str, hint: &str, shared_id: &str) -> Result<Self, Error> {
        let mut h = vec![0; FIELDS_END];
        h[CIPHER] = AES_256;
        h[KDF] = PBKDF2_SHA1;
        put_utf16z(&mut h[HINT], hint)?;
        put_utf16z(&mut h[SHARED_ID], shared_id)?;
        Ok(Encryption {
            password: password.to_string(),
            fields: h.split_off(CIPHER),
        })
    }

    /// The [`Encryption`] of an `HBAMe` image opened with `password`.
    pub(crate) fn of(bytes: &[u8], password: &str) -> Self {
        let mut fields = bytes[CIPHER..FIELDS_END].to_vec();
        fields[SALT.start - CIPHER..CHECK.end - CIPHER].fill(0);
        Encryption {
            password: password.to_string(),
            fields,
        }
    }

    /// The password hint (empty when none).
    pub fn hint(&self) -> String {
        utf16z(&self.fields[HINT.start - CIPHER..HINT.end - CIPHER])
    }

    /// The shared id.
    pub fn shared_id(&self) -> String {
        utf16z(&self.fields[SHARED_ID.start - CIPHER..])
    }
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

fn check_value(pw: &[u8], salt: &[u8]) -> [u8; 32] {
    pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 32>(pw, salt, CHECK_ROUNDS)
}

/// The AES and HMAC keys of a password and salt: two 32-byte keys, 37
/// bytes apart in one PBKDF2 output.
fn page_keys(pw: &[u8], salt: &[u8]) -> [u8; 74] {
    pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 74>(pw, salt, KEY_ROUNDS)
}

/// HMAC-SHA256 over a page's ciphertext and IV.
fn page_mac(mac_key: &[u8], page: &[u8]) -> Option<hmac::Hmac<sha2::Sha256>> {
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(mac_key).ok()?;
    mac.update(&page[..BODY + 16]);
    Some(mac)
}

fn random(buf: &mut [u8]) -> Result<(), Error> {
    getrandom::fill(buf).map_err(|_| Error::Security("no random source".into()))
}

/// Encrypts a plain `HBAM7` image with `enc`: the store is packed again
/// into payloads 32 bytes shorter, then every body sector becomes one
/// encrypted page with a fresh IV and MAC, under a new salt.
pub fn encrypt(plain: &[u8], enc: &Encryption) -> Result<Vec<u8>, Error> {
    let file = crate::File::from_bytes(plain.to_vec())?;
    let entries = crate::write::entries(&file)?;
    let mut out = crate::write::build_in(&plain[..SECTOR], &entries, crate::write::PAYLOAD - 32);
    out[15..20].copy_from_slice(b"HBAMe");
    out[CIPHER..FIELDS_END].copy_from_slice(&enc.fields);
    let mut salt = [0; 16];
    random(&mut salt)?;
    out[SALT].copy_from_slice(&salt);
    let pw = password_bytes(&enc.password);
    out[CHECK].copy_from_slice(&check_value(&pw, &salt));
    let keys = page_keys(&pw, &salt);
    let (aes_key, mac_key) = (&keys[..32], &keys[37..69]);
    let bad = || Error::Security("cannot encrypt the file".into());
    for page in out.as_chunks_mut::<SECTOR>().0.iter_mut().skip(1) {
        // Free bytes of the shorter payload (packed with 32 bytes to
        // spare, so the tail left out is zeros).
        let free = u16::from_be_bytes([page[14], page[15]])
            .checked_sub(32)
            .ok_or_else(bad)?;
        page[14..16].copy_from_slice(&free.to_be_bytes());
        let mut iv = [0; 16];
        random(&mut iv)?;
        cbc::Encryptor::<aes::Aes256>::new_from_slices(aes_key, &iv)
            .map_err(|_| bad())?
            .encrypt_padded::<NoPadding>(&mut page[..BODY], BODY)
            .map_err(|_| bad())?;
        page[BODY..BODY + 16].copy_from_slice(&iv);
        let tag = page_mac(mac_key, page)
            .ok_or_else(bad)?
            .finalize()
            .into_bytes();
        page[BODY + 16..].copy_from_slice(&tag[..16]);
    }
    Ok(out)
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
    // Not secret (it is in the header), so a plain comparison is fine.
    if check_value(&pw, salt)[..] != bytes[CHECK] {
        return Err(Error::EncryptionPassword);
    }
    let keys = page_keys(&pw, salt);
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
        page_mac(mac_key, page)
            .ok_or_else(bad)?
            .verify_truncated_left(&page[BODY + 16..])
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
