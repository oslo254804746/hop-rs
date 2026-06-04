use std::{fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroize;

use crate::{errors::HopCoreError, Result};

const ENVELOPE_VERSION: &str = "v1";
const ENVELOPE_ALG: &str = "xchacha20poly1305";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

#[derive(Debug, Clone)]
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
    pub fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub fn load_or_create_master_key(path: &Path) -> Result<MasterKey> {
    if path.exists() {
        let raw = fs::read_to_string(path)?;
        let decoded = STANDARD
            .decode(raw.trim())
            .map_err(|err| HopCoreError::Crypto(format!("invalid secret key base64: {err}")))?;
        let bytes: [u8; KEY_LEN] = decoded
            .try_into()
            .map_err(|_| HopCoreError::Crypto("secret key must decode to 32 bytes".to_string()))?;
        return Ok(MasterKey::from_bytes(bytes));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let key = MasterKey::generate();
    fs::write(path, STANDARD.encode(key.expose()))?;
    set_secret_permissions(path)?;
    Ok(key)
}

#[cfg(unix)]
fn set_secret_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_secret_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn encrypt_envelope(master: &MasterKey, context: &str, plaintext: &[u8]) -> Result<String> {
    let key = derive_key(master, context)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|err| HopCoreError::Crypto(format!("encrypt failed: {err}")))?;
    Ok(format!(
        "{ENVELOPE_VERSION}:{ENVELOPE_ALG}:{}:{}",
        STANDARD.encode(nonce),
        STANDARD.encode(ciphertext)
    ))
}

pub fn decrypt_envelope(master: &MasterKey, context: &str, envelope: &str) -> Result<Vec<u8>> {
    let parts: Vec<&str> = envelope.split(':').collect();
    if parts.len() != 4 {
        return Err(HopCoreError::Crypto("malformed envelope".to_string()));
    }
    if parts[0] != ENVELOPE_VERSION || parts[1] != ENVELOPE_ALG {
        return Err(HopCoreError::Crypto("unsupported envelope version or algorithm".to_string()));
    }
    let nonce = STANDARD
        .decode(parts[2])
        .map_err(|err| HopCoreError::Crypto(format!("invalid nonce base64: {err}")))?;
    if nonce.len() != NONCE_LEN {
        return Err(HopCoreError::Crypto("nonce must be 24 bytes".to_string()));
    }
    let ciphertext = STANDARD
        .decode(parts[3])
        .map_err(|err| HopCoreError::Crypto(format!("invalid ciphertext base64: {err}")))?;
    let key = derive_key(master, context)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    cipher
        .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| HopCoreError::Crypto(format!("decrypt failed: {err}")))
}

fn derive_key(master: &MasterKey, context: &str) -> Result<[u8; KEY_LEN]> {
    let hk = Hkdf::<Sha256>::new(Some(b"hop-credential-envelope-v1"), master.expose());
    let mut out = [0u8; KEY_LEN];
    hk.expand(context.as_bytes(), &mut out)
        .map_err(|err| HopCoreError::Crypto(format!("hkdf expand failed: {err}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = MasterKey::generate();
        let envelope = encrypt_envelope(&key, "cred-1:password", b"secret").unwrap();
        let clear = decrypt_envelope(&key, "cred-1:password", &envelope).unwrap();
        assert_eq!(clear, b"secret");
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let key = MasterKey::generate();
        let wrong_key = MasterKey::generate();
        let envelope = encrypt_envelope(&key, "cred-1:password", b"secret").unwrap();
        let err = decrypt_envelope(&wrong_key, "cred-1:password", &envelope).unwrap_err();
        assert!(err.to_string().contains("decrypt failed"));
    }

    #[test]
    fn wrong_context_fails_to_decrypt() {
        let key = MasterKey::generate();
        let envelope = encrypt_envelope(&key, "cred-1:password", b"secret").unwrap();
        let err = decrypt_envelope(&key, "cred-2:password", &envelope).unwrap_err();
        assert!(err.to_string().contains("decrypt failed"));
    }

    #[test]
    fn malformed_envelope_is_rejected() {
        let key = MasterKey::generate();
        let err = decrypt_envelope(&key, "ctx", "v1:xchacha20poly1305:not-enough").unwrap_err();
        assert!(err.to_string().contains("malformed envelope"));
    }

    #[test]
    fn nonce_is_unique_for_same_plaintext() {
        let key = MasterKey::generate();
        let first = encrypt_envelope(&key, "cred-1:password", b"secret").unwrap();
        let second = encrypt_envelope(&key, "cred-1:password", b"secret").unwrap();
        assert_ne!(first, second);
    }
}
