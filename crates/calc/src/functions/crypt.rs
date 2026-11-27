//! CryptEncrypt / CryptDecrypt (and their Base64 forms) and RSA signatures.
//!
//! Claris's security guide says CryptEncrypt derives the key with PBKDF2
//! and encrypts with AES-GCM at 128 bits, including an encrypted SHA-256
//! digest of the data. It does not give the parameters or the byte layout,
//! so ours is our own: salt (16) | nonce (12) | AES-128-GCM ciphertext and
//! tag of [SHA-256 (32) | kind (0 text, 1 container) | name length (u32 LE)
//! and name, containers only | data]; key = PBKDF2-HMAC-SHA256 (key text,
//! salt, 100 000 rounds).
// ponytail: not byte-compatible with FileMaker's CryptEncrypt output, which
// is undocumented; data encrypted here only decrypts here.

use super::{encode, txt};
use crate::error::EvalError;
use crate::eval::{Evaluator, R};
use crate::value::{Container, Value};
use aes_gcm::aead::{Aead, KeyInit};
use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{Pkcs1v15Sign, PrivateKey, PublicKey};
use sha2::{Digest, Sha256};

/// Fewer rounds in unit tests: unoptimised SHA-256 makes 100 000 rounds
/// take about a second.
const ROUNDS: u32 = if cfg!(test) { 1_000 } else { 100_000 };
const SALT: usize = 16;
const NONCE: usize = 12;

fn bad(msg: &str) -> EvalError {
    EvalError::generic(msg.to_string())
}

fn cipher(key: &str, salt: &[u8]) -> aes_gcm::Aes128Gcm {
    let mut k = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<Sha256>(key.as_bytes(), salt, ROUNDS, &mut k);
    aes_gcm::Aes128Gcm::new_from_slice(&k).expect("16-byte key")
}

fn encrypt(v: &Value, key: &str) -> Result<Vec<u8>, EvalError> {
    let mut payload = vec![];
    match v {
        Value::Container(c) => {
            payload.push(1);
            payload.extend((c.name.len() as u32).to_le_bytes());
            payload.extend(c.name.as_bytes());
            payload.extend(&c.data);
        }
        v => {
            payload.push(0);
            payload.extend(v.to_text().as_bytes());
        }
    }
    let mut plain = Sha256::digest(&payload).to_vec();
    plain.extend(payload);
    let mut head = [0u8; SALT + NONCE];
    getrandom::fill(&mut head).map_err(|_| bad("no random source"))?;
    let (salt, nonce) = head.split_at(SALT);
    let nonce = aes_gcm::Nonce::try_from(nonce).expect("12-byte nonce");
    let body = cipher(key, salt)
        .encrypt(&nonce, plain.as_slice())
        .map_err(|_| bad("encryption failed"))?;
    Ok([head.as_slice(), &body].concat())
}

fn decrypt(data: &[u8], key: &str) -> R {
    let wrong = || bad("wrong key or damaged data");
    if data.len() < SALT + NONCE + 16 {
        return Err(wrong());
    }
    let (salt, rest) = data.split_at(SALT);
    let (nonce, body) = rest.split_at(NONCE);
    let nonce = aes_gcm::Nonce::try_from(nonce).expect("12-byte nonce");
    let plain = cipher(key, salt)
        .decrypt(&nonce, body)
        .map_err(|_| wrong())?;
    let (digest, payload) = plain.split_at_checked(32).ok_or_else(wrong)?;
    if Sha256::digest(payload).as_slice() != digest {
        return Err(wrong());
    }
    match payload.split_first() {
        Some((0, text)) => Ok(Value::Text(String::from_utf8_lossy(text).into_owned())),
        Some((1, rest)) => {
            let (len, rest) = rest.split_at_checked(4).ok_or_else(wrong)?;
            let len = u32::from_le_bytes(len.try_into().unwrap()) as usize;
            let (name, data) = rest.split_at_checked(len).ok_or_else(wrong)?;
            Ok(Value::Container(Container {
                name: String::from_utf8_lossy(name).into_owned(),
                data: data.to_vec(),
            }))
        }
        _ => Err(wrong()),
    }
}

pub fn crypt_encrypt(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Container(Container {
        name: "encrypted.data".into(),
        data: encrypt(&a[0], &txt(a, 1))?,
    }))
}

pub fn crypt_encrypt_base64(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(encode::b64(&encrypt(&a[0], &txt(a, 1))?)))
}

pub fn crypt_decrypt(_: &mut Evaluator, a: &[Value]) -> R {
    decrypt(&encode::bytes(&a[0]), &txt(a, 1))
}

pub fn crypt_decrypt_base64(_: &mut Evaluator, a: &[Value]) -> R {
    decrypt(&encode::b64_decode(&txt(a, 0)), &txt(a, 1))
}

// ---- RSA signatures --------------------------------------------------------------

/// PKCS #1 v1.5 padding and the digest of `data` for an algorithm name.
fn scheme(alg: &str, data: &[u8]) -> Result<(Pkcs1v15Sign, Vec<u8>), EvalError> {
    fn with<D: Digest + rsa::pkcs8::AssociatedOid>(data: &[u8]) -> (Pkcs1v15Sign, Vec<u8>) {
        (Pkcs1v15Sign::new::<D>(), D::digest(data).to_vec())
    }
    Ok(match alg.to_ascii_uppercase().as_str() {
        "MD5" => with::<md5::Md5>(data),
        "SHA1" => with::<sha1::Sha1>(data),
        "SHA224" => with::<sha2::Sha224>(data),
        "SHA256" => with::<Sha256>(data),
        "SHA384" => with::<sha2::Sha384>(data),
        "SHA512" => with::<sha2::Sha512>(data),
        _ => return Err(bad(&format!("unsupported algorithm {alg}"))),
    })
}

/// (label, headers, DER bytes) of the first PEM block.
type Pem = (String, Vec<(String, String)>, Vec<u8>);

fn pem(text: &str) -> Result<Pem, EvalError> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let begin = text
        .find("-----BEGIN ")
        .ok_or_else(|| bad("not a PEM key"))?;
    let rest = &text[begin + 11..];
    let label_end = rest.find("-----").ok_or_else(|| bad("not a PEM key"))?;
    let label = rest[..label_end].to_string();
    let body_end = rest
        .find(&format!("-----END {label}-----"))
        .ok_or_else(|| bad("unterminated PEM key"))?;
    let body = &rest[label_end + 5..body_end];
    let mut headers = vec![];
    let mut b64 = String::new();
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match line.split_once(':') {
            Some((k, v)) => headers.push((k.trim().to_string(), v.trim().to_string())),
            None => b64.push_str(line),
        }
    }
    Ok((label, headers, encode::b64_decode(&b64)))
}

/// OpenSSL's EVP_BytesToKey with MD5 and one round, as traditional
/// encrypted PEM keys use.
fn bytes_to_key(password: &[u8], salt: &[u8], len: usize) -> Vec<u8> {
    let mut out = vec![];
    let mut prev: Vec<u8> = vec![];
    while out.len() < len {
        prev = md5::Md5::digest([prev.as_slice(), password, salt].concat()).to_vec();
        out.extend(&prev);
    }
    out.truncate(len);
    out
}

fn cbc_decrypt<C>(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, EvalError>
where
    C: cbc::cipher::BlockCipherDecrypt + cbc::cipher::KeyInit,
{
    use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
    cbc::Decryptor::<C>::new_from_slices(key, iv)
        .map_err(|_| bad("invalid key IV"))?
        .decrypt_padded_vec::<cbc::cipher::block_padding::Pkcs7>(data)
        .map_err(|_| bad("wrong key password"))
}

/// Decrypts a traditional `Proc-Type: 4,ENCRYPTED` key body.
fn decrypt_pem(
    headers: &[(String, String)],
    der: Vec<u8>,
    password: &str,
) -> Result<Vec<u8>, EvalError> {
    let Some((_, dek)) = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("DEK-Info"))
    else {
        return Ok(der);
    };
    let (alg, iv) = dek.split_once(',').ok_or_else(|| bad("bad DEK-Info"))?;
    let iv = encode::hex_bytes(iv.trim());
    let salt = iv.get(..8).ok_or_else(|| bad("bad DEK-Info"))?;
    let pass = password.as_bytes();
    match alg.trim().to_ascii_uppercase().as_str() {
        "DES-EDE3-CBC" => cbc_decrypt::<des::TdesEde3>(&bytes_to_key(pass, salt, 24), &iv, &der),
        "DES-CBC" => cbc_decrypt::<des::Des>(&bytes_to_key(pass, salt, 8), &iv, &der),
        "AES-128-CBC" => cbc_decrypt::<aes::Aes128>(&bytes_to_key(pass, salt, 16), &iv, &der),
        "AES-192-CBC" => cbc_decrypt::<aes::Aes192>(&bytes_to_key(pass, salt, 24), &iv, &der),
        "AES-256-CBC" => cbc_decrypt::<aes::Aes256>(&bytes_to_key(pass, salt, 32), &iv, &der),
        other => Err(bad(&format!("unsupported key cipher {other}"))),
    }
}

// ponytail: PKCS #8 "ENCRYPTED PRIVATE KEY" blocks are not supported; they
// need PBES2 (rsa's pkcs5 feature).
fn private_key(text: &str, password: &str) -> Result<PrivateKey, EvalError> {
    let (label, headers, der) = pem(text)?;
    let der = decrypt_pem(&headers, der, password)?;
    let invalid = |_| bad("invalid private key");
    match label.as_str() {
        "RSA PRIVATE KEY" => PrivateKey::from_pkcs1_der(&der).map_err(invalid),
        "PRIVATE KEY" => PrivateKey::from_pkcs8_der(&der).map_err(|_| bad("invalid private key")),
        _ => Err(bad("unsupported private key")),
    }
}

fn public_key(text: &str) -> Result<PublicKey, EvalError> {
    let (label, _, der) = pem(text)?;
    match label.as_str() {
        "RSA PUBLIC KEY" => PublicKey::from_pkcs1_der(&der).map_err(|_| bad("invalid public key")),
        "PUBLIC KEY" => PublicKey::from_public_key_der(&der).map_err(|_| bad("invalid public key")),
        // A private key can verify too.
        _ => Ok(private_key(text, "")?.to_public_key()),
    }
}

/// CryptGenerateSignature ( data ; algorithm ; privateRSAKey ; keyPassword ):
/// RSA PKCS #1 v1.5 signature of the digest, as container data.
// ponytail: PKCS #1 v1.5 is assumed (the documentation does not name the
// padding); it is what OpenSSL's dgst -sign produces.
pub fn crypt_generate_signature(_: &mut Evaluator, a: &[Value]) -> R {
    let key = private_key(&txt(a, 2), &txt(a, 3))?;
    let (padding, hashed) = scheme(&txt(a, 1), &encode::bytes(&a[0]))?;
    let sig = key
        .sign(padding, &hashed)
        .map_err(|_| bad("signing failed"))?;
    Ok(Value::Container(Container {
        name: String::new(),
        data: sig,
    }))
}

/// CryptVerifySignature ( data ; algorithm ; publicRSAKey ; signature ): 1 or 0.
pub fn crypt_verify_signature(_: &mut Evaluator, a: &[Value]) -> R {
    let key = public_key(&txt(a, 2))?;
    let (padding, hashed) = scheme(&txt(a, 1), &encode::bytes(&a[0]))?;
    Ok(Value::bool(
        key.verify(padding, &hashed, &encode::bytes(&a[3])).is_ok(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let c = encrypt(&Value::text("secret ¶ text"), "pw").unwrap();
        assert_eq!(decrypt(&c, "pw").unwrap(), Value::text("secret ¶ text"));
        assert!(decrypt(&c, "other").is_err());
        let file = Value::Container(Container {
            name: "a.bin".into(),
            data: vec![0, 1, 2],
        });
        assert_eq!(decrypt(&encrypt(&file, "k").unwrap(), "k").unwrap(), file);
    }

    #[test]
    fn evp_bytes_to_key() {
        // openssl enc -des-ede3-cbc -k pw -S 0102030405060708 -P
        let k = bytes_to_key(b"pw", &[1, 2, 3, 4, 5, 6, 7, 8], 24);
        assert_eq!(k.len(), 24);
        assert_eq!(
            &k[..16],
            md5::Md5::digest(b"pw\x01\x02\x03\x04\x05\x06\x07\x08").as_slice()
        );
    }
}
