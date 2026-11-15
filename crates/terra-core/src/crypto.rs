//! AES-128-CBC framing for Terraria `.plr` / `.tplr` files.
//!
//! The key and IV are the same 16 bytes: the UTF-16LE encoding of `h3y_gUyZ`.
//! The whole payload is padded with PKCS#7 and encrypted as a single stream.

pub const KEY_BYTES: [u8; 16] = [
    0x68, 0x00, 0x33, 0x00, 0x79, 0x00, 0x5F, 0x00, 0x67, 0x00, 0x55, 0x00, 0x79, 0x00, 0x5A, 0x00,
];

pub const BLOCK: usize = 16;

use aes::Aes128;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};

type Enc = cbc::Encryptor<Aes128>;
type Dec = cbc::Decryptor<Aes128>;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("file is not a whole number of AES blocks ({0} bytes)")]
    NotBlockAligned(usize),
    #[error("decryption failed (bad padding)")]
    BadPadding,
    #[error("encryption failed")]
    Encrypt,
}

/// Decrypt a `.plr` byte stream, stripping PKCS#7 padding.
pub fn decrypt(data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if data.is_empty() || !data.len().is_multiple_of(BLOCK) {
        return Err(CryptoError::NotBlockAligned(data.len()));
    }
    let mut buf = data.to_vec();
    let plain = Dec::new(&KEY_BYTES.into(), &KEY_BYTES.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| CryptoError::BadPadding)?;
    Ok(plain.to_vec())
}

/// Encrypt a payload, applying PKCS#7 padding.
pub fn encrypt(plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut buf = vec![0u8; plain.len() + BLOCK];
    buf[..plain.len()].copy_from_slice(plain);
    let ct = Enc::new(&KEY_BYTES.into(), &KEY_BYTES.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
        .map_err(|_| CryptoError::Encrypt)?;
    Ok(ct.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_utf16_h3y_guyz() {
        let utf16: Vec<u8> = "h3y_gUyZ"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(utf16, KEY_BYTES.to_vec());
    }

    #[test]
    fn roundtrip_various_lengths() {
        for len in [0usize, 1, 15, 16, 17, 3838, 3824] {
            let plain = vec![0x5Au8; len];
            let ct = encrypt(&plain).unwrap();
            assert_eq!(ct.len() % BLOCK, 0);
            assert!(ct.len() > len);
            assert_eq!(decrypt(&ct).unwrap(), plain, "len {len}");
        }
    }

    #[test]
    fn rejects_misaligned_input() {
        assert!(matches!(
            decrypt(&[0u8; 17]),
            Err(CryptoError::NotBlockAligned(17))
        ));
    }
}
