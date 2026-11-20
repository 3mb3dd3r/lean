//! Windows DPAPI, current-user scope: encrypts the tokens in `accounts.json` so only this
//! Windows user on this PC can read them back.

use anyhow::{bail, Result};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

/// Extra entropy: another program calling CryptUnprotectData on these blobs also needs this value.
const ENTROPY: &[u8] = b"lean-launcher accounts v1";

fn blob(data: &[u8]) -> Result<CRYPT_INTEGER_BLOB> {
    let len = u32::try_from(data.len())?;
    let ptr = if data.is_empty() { std::ptr::null_mut() } else { data.as_ptr() as *mut u8 };
    Ok(CRYPT_INTEGER_BLOB { cbData: len, pbData: ptr })
}

/// Copies the blob DPAPI allocated, optionally wipes it, then frees it with LocalFree.
///
/// # Safety
/// `out` must be an output blob filled by CryptProtectData or CryptUnprotectData.
unsafe fn take(out: CRYPT_INTEGER_BLOB, wipe: bool) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let len = out.cbData as usize;
    let data = std::slice::from_raw_parts(out.pbData, len).to_vec();
    if wipe {
        std::ptr::write_bytes(out.pbData, 0, len);
    }
    LocalFree(out.pbData.cast());
    data
}

pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
    let input = blob(plain)?;
    let entropy = blob(ENTROPY)?;
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: the input blobs point at live slices; DPAPI allocates `out`, which `take` frees.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        bail!("Windows could not encrypt the data (DPAPI: {})", std::io::Error::last_os_error());
    }
    Ok(unsafe { take(out, false) })
}

pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>> {
    let input = blob(cipher)?;
    let entropy = blob(ENTROPY)?;
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: as in `protect`; the description out-pointer is not requested.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        bail!("Windows could not decrypt the data (DPAPI: {})", std::io::Error::last_os_error());
    }
    Ok(unsafe { take(out, true) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_the_real_api() {
        let secret = b"M.C123_test-refresh-token!*$ with some length to it";
        let enc = protect(secret).unwrap();
        assert!(enc.len() > secret.len());
        assert!(!enc.windows(secret.len()).any(|w| w == secret), "ciphertext must not contain the plaintext");
        assert_eq!(unprotect(&enc).unwrap(), secret);
        // Same input twice gives different blobs (random salt), both decrypt.
        let enc2 = protect(secret).unwrap();
        assert_ne!(enc, enc2);
        assert_eq!(unprotect(&enc2).unwrap(), secret);
    }

    #[test]
    fn damaged_or_foreign_blobs_fail() {
        let mut enc = protect(b"token").unwrap();
        let last = enc.len() - 1;
        enc[last] ^= 0x55;
        assert!(unprotect(&enc).is_err());
        assert!(unprotect(b"not a dpapi blob").is_err());
        assert!(unprotect(&[]).is_err());
    }
}
