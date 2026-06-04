use std::{fs, path::Path};

use anyhow::{bail, Context, Result};
use russh::keys::{load_secret_key, ssh_key::LineEnding, Algorithm, PrivateKey};

pub fn load_or_generate(path: &Path, key_type: &str) -> Result<PrivateKey> {
    if path.exists() {
        return load_secret_key(path, None).with_context(|| format!("load host key {}", path.display()));
    }
    if key_type != "ed25519" {
        bail!("MVP only supports ed25519 hop host keys");
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut rng = rand10::rng();
    let key = PrivateKey::random(&mut rng, Algorithm::Ed25519).context("generate ed25519 host key")?;
    let encoded = key.to_openssh(LineEnding::LF).context("encode host key")?;
    fs::write(path, encoded.as_bytes()).with_context(|| format!("write host key {}", path.display()))?;
    set_key_permissions(path)?;
    Ok(key)
}

#[cfg(unix)]
fn set_key_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_key_permissions(_path: &Path) -> Result<()> {
    Ok(())
}
