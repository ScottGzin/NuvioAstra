//! Platform cryptography used by the account (no heavyweight KMS):
//! - Windows: DPAPI (`CryptProtectData`, current user scope) protects the session file: only this
//!   Windows user on this computer can read it. `BCryptHash`/`BCryptGenRandom` for PIN and salts.
//! - Linux: AES-256-GCM with a per-user key file (mode 0600) gives the same semantics: only this
//!   user on this computer can decrypt the session. SHA-256 / getrandom come from pure crates.
//!
//! Chosen over the OS credential store because Supabase access tokens can exceed its blob limit.

const ENTROPY: &[u8] = b"NuvioAstra.Account.v1";

#[cfg(windows)]
mod platform {
    use super::ENTROPY;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        BCryptGenRandom, BCryptHash, CryptProtectData, CryptUnprotectData, BCRYPT_SHA256_ALG_HANDLE,
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: input/entropy point to live slices for the duration of the call; the output buffer is
        // allocated by DPAPI and released with LocalFree after copying.
        unsafe {
            CryptProtectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
                .map_err(|e| e.message())?;
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            Ok(bytes)
        }
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(cipher);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`.
        unsafe {
            CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
                .map_err(|e| e.message())?;
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            Ok(bytes)
        }
    }

    pub fn sha256_hex(value: &str) -> String {
        let mut out = [0u8; 32];
        // SAFETY: pseudo-handle algorithm, slices valid for the call.
        let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, value.as_bytes(), &mut out) };
        assert!(status.is_ok(), "BCryptHash failed");
        out.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn random_bytes(n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        // SAFETY: system RNG, buffer valid for the call.
        let status = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        assert!(status.is_ok(), "BCryptGenRandom failed");
        buf
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ENTROPY;
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::PathBuf;

    const NONCE_LEN: usize = 12;

    /// Per-user key, stored next to the app data with 0600 permissions (and a 0700 directory).
    /// Losing it invalidates the session file, which is the same outcome as losing a DPAPI profile.
    fn key_path() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir);
        base.join("NuvioAstra").join(".account.key")
    }

    fn key() -> Result<[u8; 32], String> {
        let path = key_path();
        if let Ok(bytes) = std::fs::read(&path) {
            return bytes.try_into().map_err(|_| "arquivo de chave da conta inválido".to_string());
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let key = key()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let mut raw_nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut raw_nonce).map_err(|e| e.to_string())?;
        let body = cipher
            .encrypt(Nonce::from_slice(&raw_nonce), aes_gcm::aead::Payload { msg: plain, aad: ENTROPY })
            .map_err(|_| "falha ao proteger a sessão".to_string())?;
        let mut out = Vec::with_capacity(NONCE_LEN + body.len());
        out.extend_from_slice(&raw_nonce);
        out.extend_from_slice(&body);
        Ok(out)
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
        if cipher.len() <= NONCE_LEN {
            return Err("sessão protegida corrompida".into());
        }
        let key = key()?;
        let aes = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        aes.decrypt(
            Nonce::from_slice(&cipher[..NONCE_LEN]),
            aes_gcm::aead::Payload { msg: &cipher[NONCE_LEN..], aad: ENTROPY },
        )
        .map_err(|_| "sessão protegida inválida ou chave diferente".into())
    }

    pub fn sha256_hex(value: &str) -> String {
        let digest = Sha256::digest(value.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn random_bytes(n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        getrandom::fill(&mut buf).expect("system RNG");
        buf
    }
}

pub use platform::{protect, random_bytes, sha256_hex, unprotect};

/// Lowercase alphanumeric string of `n` characters.
pub fn random_token(n: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    random_bytes(n).iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_round_trip_and_tamper() {
        let secret = br#"{"refresh_token":"abc"}"#;
        let cipher = protect(secret).unwrap();
        assert!(!cipher.windows(3).any(|w| w == b"abc"), "not stored in clear");
        assert_eq!(unprotect(&cipher).unwrap(), secret);
        let mut bad = cipher.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xff;
        assert!(unprotect(&bad).is_err());
    }

    #[test]
    fn sha256_matches_known_vector_and_original_pin_format() {
        assert_eq!(sha256_hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(random_token(32).len(), 32);
    }
}
