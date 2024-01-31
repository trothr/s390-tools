// SPDX-License-Identifier: MIT
//
// Copyright IBM Corp. 2023

use crate::{error::Result, secret::Secret, Error};
use openssl::{
    derive::Deriver,
    ec::{EcGroup, EcKey},
    hash::{DigestBytes, MessageDigest},
    md::MdRef,
    nid::Nid,
    pkey::{HasPublic, Id, PKey, PKeyRef, Private, Public},
    pkey_ctx::{HkdfMode, PkeyCtx},
    rand::rand_bytes,
    rsa::Padding,
    sign::{Signer, Verifier},
    symm::{encrypt_aead, Cipher},
};
use std::{convert::TryInto, ops::Range};

/// An AES256-key that will purge itself out of the memory when going out of scope
///
pub type Aes256Key = Secret<[u8; 32]>;
pub(crate) const AES_256_GCM_TAG_SIZE: usize = 16;

/// Types of symmetric keys, to specify during construction.
///
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymKeyType {
    /// AES 256 key (32 bytes)
    Aes256,
}

/// Types of symmetric keys
///
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymKey {
    /// AES 256 key (32 bytes)
    Aes256(Aes256Key),
}

impl SymKey {
    /// Generates a random symmetric key.
    ///
    /// * `key_tp` - type of the symmetric key
    ///
    /// # Errors
    ///
    /// This function will return an error if the Key cannot be generated.
    pub fn random(key_tp: SymKeyType) -> Result<Self> {
        match key_tp {
            SymKeyType::Aes256 => Ok(Self::Aes256(random_array().map(|v| v.into())?)),
        }
    }

    /// Returns a reference to the value of this [`SymKey`].
    pub fn value(&self) -> &[u8] {
        match self {
            Self::Aes256(key) => key.value(),
        }
    }
}

impl Aes256Key {
    /// Generates an AES256 key from an digest (hash).
    ///
    /// # Panics
    ///
    /// Panics if `digset` is not 32 bytes long.
    fn from_digest(digest: DigestBytes) -> Self {
        let key: [u8; 32] = digest
            .as_ref()
            .try_into()
            .expect("Unexpected OpenSSl Error. Sha256 hash not 32 bytes long");
        key.into()
    }
}

impl From<Aes256Key> for SymKey {
    fn from(value: Aes256Key) -> Self {
        Self::Aes256(value)
    }
}

/// Performs an hkdf according to RFC 5869.
/// See [`OpenSSL HKDF`]()
///
/// # Errors
///
/// This function will return an OpenSSL error if the key could not be generated.
pub fn hkdf_rfc_5869<const COUNT: usize>(
    md: &MdRef,
    ikm: &[u8],
    salt: &[u8],
    info: &[u8],
) -> Result<[u8; COUNT]> {
    let mut ctx = PkeyCtx::new_id(Id::HKDF)?;
    ctx.derive_init()?;
    ctx.set_hkdf_mode(HkdfMode::EXTRACT_THEN_EXPAND)?;
    ctx.set_hkdf_md(md)?;
    ctx.set_hkdf_salt(salt)?;
    ctx.set_hkdf_key(ikm)?;
    ctx.add_hkdf_info(info)?;

    let mut res = [0; COUNT];
    ctx.derive(Some(&mut res))?;
    Ok(res)
}

/// Derive a symmetric key from a private and a public key.
///
/// # Errors
///
/// This function will return an error if something went bad in OpenSSL.
pub fn derive_key(k1: &PKey<Private>, k2: &PKey<Public>) -> Result<Aes256Key> {
    let mut der = Deriver::new(k1)?;
    der.set_peer(k2)?;
    let mut key = der.derive_to_vec()?;
    key.extend([0, 0, 0, 1]);
    let secr = Secret::new(key);

    Ok(Aes256Key::from_digest(hash(
        MessageDigest::sha256(),
        secr.value(),
    )?))
}

/// Generate a random array.
///
/// # Errors
///
/// This function will return an error if the entropy source fails or is not available.
pub fn random_array<const COUNT: usize>() -> Result<[u8; COUNT]> {
    let mut rand = [0; COUNT];
    rand_bytes(&mut rand)?;
    Ok(rand)
}

/// Generate a new random EC-SECP521R1 key.
///
/// # Errors
///
/// This function will return an error if the key could not be generated by OpenSSL.
pub fn gen_ec_key() -> Result<PKey<Private>> {
    let group = EcGroup::from_curve_name(Nid::SECP521R1)?;
    let key: EcKey<Private> = EcKey::generate(&group)?;
    PKey::from_ec_key(key).map_err(Error::Crypto)
}

/// Encrypt confidential Data with a symmetric key and provida a gcm tag.
///
/// * `key` - symmetric key used for encryption
/// * `iv` - initialisation vector
/// * `aad` - additional authentic data
/// * `conf` - data to be encrypted
///
/// # Returns
/// [`Vec<u8>`] with the following content:
/// 1. `aad`
/// 2. `encr(conf)`
/// 3. `aes gcm tag`
///
/// # Errors
///
/// This function will return an error if the data could not be encrypted by OpenSSL.
pub fn encrypt_aes_gcm(
    key: &SymKey,
    iv: &[u8],
    aad: &[u8],
    conf: &[u8],
) -> Result<(Vec<u8>, Range<usize>, Range<usize>, Range<usize>)> {
    let mut tag = vec![0xff; AES_256_GCM_TAG_SIZE];
    let encr = match key {
        SymKey::Aes256(key) => encrypt_aead(
            Cipher::aes_256_gcm(),
            key.value(),
            Some(iv),
            aad,
            conf,
            &mut tag,
        )?,
    };

    let mut res = vec![0; aad.len() + encr.len() + tag.len()];
    let aad_range = Range {
        start: 0,
        end: aad.len(),
    };
    let encr_range = Range {
        start: aad.len(),
        end: aad.len() + encr.len(),
    };
    let tag_range = Range {
        start: aad.len() + encr.len(),
        end: aad.len() + encr.len() + tag.len(),
    };

    res[aad_range.clone()].copy_from_slice(aad);
    res[encr_range.clone()].copy_from_slice(&encr);
    res[tag_range.clone()].copy_from_slice(&tag);
    Ok((res, aad_range, encr_range, tag_range))
}

/// Calculate the hash of a slice.
///
/// # Errors
///
/// This function will return an error if OpenSSL could not compute the hash.
pub fn hash(t: MessageDigest, data: &[u8]) -> Result<DigestBytes> {
    openssl::hash::hash(t, data).map_err(Error::Crypto)
}

/// Calculate a digital signature scheme.
///
/// Calculates the digital signature of the provided message using the signing key. [`Id::EC`],
/// and [`Id::RSA`] keys are supported. For [`Id::RSA`] [`Padding::PKCS1_PSS`] is used.
///
/// # Errors
///
/// This function will return an error if OpenSSL could not compute the signature.
pub fn sign_msg(skey: &PKeyRef<Private>, dgst: MessageDigest, msg: &[u8]) -> Result<Vec<u8>> {
    match skey.id() {
        Id::EC => {
            let mut sgn = Signer::new(dgst, skey)?;
            sgn.sign_oneshot_to_vec(msg).map_err(Error::Crypto)
        }
        Id::RSA => {
            let mut sgn = Signer::new(dgst, skey)?;
            sgn.set_rsa_padding(Padding::PKCS1_PSS)?;
            sgn.sign_oneshot_to_vec(msg).map_err(Error::Crypto)
        }
        _ => Err(Error::UnsupportedSigningKey),
    }
}

/// Verify the digital signature of a message.
///
/// Verifies the digital signature of the provided message using the signing key.
/// [`Id::EC`] and [`Id::RSA`] keys are supported. For [`Id::RSA`] [`Padding::PKCS1_PSS`] is used.
///
/// # Returns
/// true if signature could be verified, false otherwise
///
/// # Errors
///
/// This function will return an error if OpenSSL could not compute the signature.
pub fn verify_signature<T: HasPublic>(
    skey: &PKeyRef<T>,
    dgst: MessageDigest,
    msg: &[u8],
    sign: &[u8],
) -> Result<bool> {
    match skey.id() {
        Id::EC => {
            let mut ctx = Verifier::new(dgst, skey)?;
            ctx.update(msg)?;
            ctx.verify(sign).map_err(Error::Crypto)
        }
        Id::RSA => {
            let mut ctx = Verifier::new(dgst, skey)?;
            ctx.set_rsa_padding(Padding::PKCS1_PSS)?;
            ctx.verify_oneshot(sign, msg).map_err(Error::Crypto)
        }
        _ => Err(Error::UnsupportedVerificationKey),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{get_test_asset, test_utils::*};

    #[test]
    fn sign_ec() {
        let (ec_key, _) = get_test_keys();

        let data = "sample".as_bytes();
        let sign = sign_msg(&ec_key, MessageDigest::sha512(), data).unwrap();
        assert!(sign.len() <= 139 && sign.len() >= 137);

        assert!(verify_signature(&ec_key, MessageDigest::sha512(), data, &sign).unwrap());
    }

    #[test]
    fn sign_rsa_2048() {
        let keypair = get_test_asset!("keys/rsa2048key.pem");
        let keypair = PKey::private_key_from_pem(keypair).unwrap();

        let data = "sample".as_bytes();
        let sign = sign_msg(&keypair, MessageDigest::sha512(), data).unwrap();
        assert_eq!(256, sign.len());

        assert!(verify_signature(&keypair, MessageDigest::sha512(), data, &sign).unwrap());
    }

    #[test]
    fn sign_rsa_3072() {
        let keypair = get_test_asset!("keys/rsa3072key.pem");
        let keypair = PKey::private_key_from_pem(keypair).unwrap();

        let data = "sample".as_bytes();
        let sign = sign_msg(&keypair, MessageDigest::sha512(), data).unwrap();
        assert_eq!(384, sign.len());

        assert!(verify_signature(&keypair, MessageDigest::sha512(), data, &sign).unwrap());
    }

    #[test]
    fn derive_key() {
        let (cust_key, host_key) = get_test_keys();

        let exp_key: Aes256Key = [
            0x75, 0x32, 0x77, 0x55, 0x8f, 0x3b, 0x60, 0x3, 0x41, 0x9e, 0xf2, 0x49, 0xae, 0x3c,
            0x4b, 0x55, 0xaa, 0xd7, 0x7d, 0x9, 0xd9, 0x7f, 0xdd, 0x1f, 0xc8, 0x8f, 0xd8, 0xf0,
            0xcf, 0x22, 0xf1, 0x49,
        ]
        .into();

        let calc_key = super::derive_key(&cust_key, &host_key).unwrap();

        assert_eq!(&calc_key, &exp_key);
    }

    #[test]
    fn hkdf_rfc_5869() {
        use openssl::md::Md;
        // RFC 6869 test vector 1
        let ikm = [0x0bu8; 22];
        let salt: [u8; 13] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        ];
        let info: [u8; 10] = [0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9];
        let exp: [u8; 42] = [
            0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
            0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
            0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65,
        ];
        let res: [u8; 42] = super::hkdf_rfc_5869(Md::sha256(), &ikm, &salt, &info).unwrap();

        assert_eq!(exp, res);
    }

    #[test]
    fn encrypt_aes_256_gcm() {
        let aes_gcm_key = [
            0xee, 0xbc, 0x1f, 0x57, 0x48, 0x7f, 0x51, 0x92, 0x1c, 0x04, 0x65, 0x66, 0x5f, 0x8a,
            0xe6, 0xd1, 0x65, 0x8b, 0xb2, 0x6d, 0xe6, 0xf8, 0xa0, 0x69, 0xa3, 0x52, 0x02, 0x93,
            0xa5, 0x72, 0x07, 0x8f,
        ];
        let aes_gcm_iv = [
            0x99, 0xaa, 0x3e, 0x68, 0xed, 0x81, 0x73, 0xa0, 0xee, 0xd0, 0x66, 0x84,
        ];
        let aes_gcm_plain = [
            0xf5, 0x6e, 0x87, 0x05, 0x5b, 0xc3, 0x2d, 0x0e, 0xeb, 0x31, 0xb2, 0xea, 0xcc, 0x2b,
            0xf2, 0xa5,
        ];
        let aes_gcm_aad = [
            0x4d, 0x23, 0xc3, 0xce, 0xc3, 0x34, 0xb4, 0x9b, 0xdb, 0x37, 0x0c, 0x43, 0x7f, 0xec,
            0x78, 0xde,
        ];
        let aes_gcm_res = vec![
            0x4d, 0x23, 0xc3, 0xce, 0xc3, 0x34, 0xb4, 0x9b, 0xdb, 0x37, 0x0c, 0x43, 0x7f, 0xec,
            0x78, 0xde, 0xf7, 0x26, 0x44, 0x13, 0xa8, 0x4c, 0x0e, 0x7c, 0xd5, 0x36, 0x86, 0x7e,
            0xb9, 0xf2, 0x17, 0x36, 0x67, 0xba, 0x05, 0x10, 0x26, 0x2a, 0xe4, 0x87, 0xd7, 0x37,
            0xee, 0x62, 0x98, 0xf7, 0x7e, 0x0c,
        ];

        let (res, ..) = encrypt_aes_gcm(
            &SymKey::Aes256(aes_gcm_key.into()),
            &aes_gcm_iv,
            &aes_gcm_aad,
            &aes_gcm_plain,
        )
        .unwrap();
        assert_eq!(res, aes_gcm_res);
    }
}
