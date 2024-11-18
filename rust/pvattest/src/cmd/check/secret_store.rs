// SPDX-License-Identifier: MIT
//
// Copyright IBM Corp. 2024

use std::path::{Path, PathBuf};

use anyhow::Result;
use log::info;
use openssl::hash::DigestBytes;
use openssl::hash::{hash, MessageDigest};
use pv::{misc::read_file, secret::AddSecretRequest};
use serde::Serialize;

use super::{bail_check, CheckState};
use crate::{additional::AttestationResult, cli::CheckOpt};

#[derive(Debug, Serialize)]
pub struct SecretStoreCheck<'a> {
    add_secret_requests: &'a [PathBuf],
    locked: bool,
}

const REQUEST_TAG_SIZE: usize = 16;

fn secret_store_hash<A: AsRef<Path>>(asrcbs: &[A], locked: bool) -> Result<DigestBytes> {
    let mut requests = Vec::with_capacity(asrcbs.len() * REQUEST_TAG_SIZE + 1);
    for asrcb in asrcbs {
        let asrcb = read_file(asrcb, "Add-secret request")?;
        let mut tag = AddSecretRequest::bin_tag(&asrcb)?;
        requests.append(&mut tag);
    }
    requests.push(locked as u8);
    Ok(hash(MessageDigest::sha512(), &requests)?)
}

pub fn secret_store_check<'a>(
    opt: &'a CheckOpt,
    att_res: &AttestationResult,
) -> Result<CheckState<SecretStoreCheck<'a>>> {
    // The locked flag is the feature gate of this check
    let locked = match opt.secret_store_locked {
        None => return Ok(CheckState::None),
        Some(state) => state,
    };

    let att_store_hash = match att_res
        .add_fields
        .as_ref()
        .and_then(|add| add.secret_store_hash())
    {
        Some(h) => h,
        None => bail_check!(
            "The Attestation response contains no secret-store-hash, but checking was enabled"
        ),
    };

    if secret_store_hash(&opt.secret, locked)?.as_ref() != att_store_hash.as_ref() {
        bail_check!("The calculated secret-store-hash does not match with the provided hash");
    }
    info!("✓ Secret Store hash");

    Ok(CheckState::Data(SecretStoreCheck {
        add_secret_requests: &opt.secret,
        locked,
    }))
}
#[cfg(test)]
mod test {
    use super::secret_store_hash;

    const ASRCB_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/assets/asrcb");

    #[test]
    fn hash() {
        let asrcbs = [
            format!("{ASRCB_DIR}/assoc_derived_default_cuid_one"),
            format!("{ASRCB_DIR}/assoc_simple_default_cuid_one"),
            format!("{ASRCB_DIR}/null_none_default_cuid_one"),
            format!("{ASRCB_DIR}/null_none_default_ncuid_one"),
            format!("{ASRCB_DIR}/null_simple_default_cuid_one"),
            format!("{ASRCB_DIR}/assoc_none_default_cuid_one"),
            format!("{ASRCB_DIR}/null_derived_default_cuid_one"),
            format!("{ASRCB_DIR}/null_none_default_cuid_seven"),
            format!("{ASRCB_DIR}/null_none_dump_cuid_one"),
        ];
        let hash = secret_store_hash(&asrcbs, true).unwrap();
        let exp = [
            0xd0, 0x48, 0x70, 0x2b, 0x4a, 0x79, 0x47, 0x8b, 0x98, 0x5e, 0x92, 0xe7, 0xed, 0xff,
            0x45, 0x3f, 0x63, 0xf2, 0x4, 0x4e, 0x7d, 0x72, 0xfa, 0xf1, 0x2e, 0xfd, 0x2e, 0xae,
            0xa0, 0xcd, 0x5, 0x5, 0x55, 0x9e, 0xd6, 0x66, 0x5a, 0x6, 0xf6, 0xb4, 0xf9, 0xc6, 0xfc,
            0xf9, 0xf2, 0x96, 0xe, 0x6c, 0xd3, 0xc3, 0xcc, 0x8c, 0xaf, 0xe5, 0x7a, 0xc8, 0x40,
            0x6e, 0x61, 0x6b, 0xf9, 0x52, 0x95, 0x17,
        ];
        assert_eq!(&exp, hash.as_ref());

        let hash = secret_store_hash(&asrcbs, false).unwrap();
        let exp = [
            0x51, 0xce, 0x62, 0xaf, 0x1f, 0x67, 0xb9, 0xe3, 0x25, 0x4b, 0x18, 0x4e, 0x33, 0xb2,
            0xaa, 0xd3, 0x10, 0x7, 0x58, 0x1a, 0x39, 0xe9, 0x9c, 0xde, 0xb0, 0x29, 0x98, 0xa3,
            0xb6, 0x7f, 0xf4, 0x56, 0xc4, 0x4a, 0x5, 0xee, 0x7d, 0x68, 0xe2, 0x4d, 0xfd, 0x43,
            0x6f, 0x2b, 0xe4, 0xc1, 0xe9, 0xf5, 0xc1, 0x1, 0x64, 0x68, 0xda, 0x64, 0x1, 0x5e, 0x9f,
            0x9f, 0xa3, 0x15, 0x6e, 0x11, 0xd, 0x6c,
        ];
        assert_eq!(&exp, hash.as_ref());
    }

    #[test]
    fn hash_empty() {
        let hash = secret_store_hash::<&str>(&[], true).unwrap();
        let exp = [
            0x7b, 0x54, 0xb6, 0x68, 0x36, 0xc1, 0xfb, 0xdd, 0x13, 0xd2, 0x44, 0x1d, 0x9e, 0x14,
            0x34, 0xdc, 0x62, 0xca, 0x67, 0x7f, 0xb6, 0x8f, 0x5f, 0xe6, 0x6a, 0x46, 0x4b, 0xaa,
            0xde, 0xcd, 0xbd, 0x0, 0x57, 0x6f, 0x8d, 0x6b, 0x5a, 0xc3, 0xbc, 0xc8, 0x8, 0x44, 0xb7,
            0xd5, 0xb, 0x1c, 0xc6, 0x60, 0x34, 0x44, 0xbb, 0xe7, 0xcf, 0xcf, 0x8f, 0xc0, 0xaa,
            0x1e, 0xe3, 0xc6, 0x36, 0xd9, 0xe3, 0x39,
        ];
        assert_eq!(&exp, hash.as_ref());

        let hash = secret_store_hash::<&str>(&[], false).unwrap();
        let exp = [
            0xb8, 0x24, 0x4d, 0x2, 0x89, 0x81, 0xd6, 0x93, 0xaf, 0x7b, 0x45, 0x6a, 0xf8, 0xef,
            0xa4, 0xca, 0xd6, 0x3d, 0x28, 0x2e, 0x19, 0xff, 0x14, 0x94, 0x2c, 0x24, 0x6e, 0x50,
            0xd9, 0x35, 0x1d, 0x22, 0x70, 0x4a, 0x80, 0x2a, 0x71, 0xc3, 0x58, 0xb, 0x63, 0x70,
            0xde, 0x4c, 0xeb, 0x29, 0x3c, 0x32, 0x4a, 0x84, 0x23, 0x34, 0x25, 0x57, 0xd4, 0xe5,
            0xc3, 0x84, 0x38, 0xf0, 0xe3, 0x69, 0x10, 0xee,
        ];
        assert_eq!(&exp, hash.as_ref());
    }
}
