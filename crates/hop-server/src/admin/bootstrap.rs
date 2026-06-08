use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use hop_core::HopDb;
use rand::{distributions::Alphanumeric, Rng};

pub const ADMIN_PASSWORD_HASH: &str = "admin_password_hash";
pub const FIRST_RUN_COMPLETED: &str = "first_run_completed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminPasswordChangeError {
    CurrentPasswordInvalid,
    NewPasswordEmpty,
    ConfirmationMismatch,
}

pub async fn ensure_admin_password(db: &HopDb) -> Result<Option<String>> {
    if db.get_setting(ADMIN_PASSWORD_HASH).await?.is_some() {
        return Ok(None);
    }
    let password = generate_password();
    set_admin_password(db, &password).await?;
    Ok(Some(password))
}

pub async fn reset_admin_password(db: &HopDb) -> Result<String> {
    let password = generate_password();
    set_admin_password(db, &password).await?;
    Ok(password)
}

pub async fn set_admin_password(db: &HopDb, password: &str) -> Result<()> {
    let hash = hash_password(password)?;
    db.set_setting(ADMIN_PASSWORD_HASH, &hash).await?;
    db.set_setting(FIRST_RUN_COMPLETED, "true").await?;
    Ok(())
}

pub async fn change_admin_password(
    db: &HopDb,
    current_password: &str,
    new_password: &str,
    confirm_password: &str,
) -> Result<std::result::Result<(), AdminPasswordChangeError>> {
    if new_password.is_empty() {
        return Ok(Err(AdminPasswordChangeError::NewPasswordEmpty));
    }
    if new_password != confirm_password {
        return Ok(Err(AdminPasswordChangeError::ConfirmationMismatch));
    }
    if !verify_admin_password(db, current_password).await? {
        return Ok(Err(AdminPasswordChangeError::CurrentPasswordInvalid));
    }
    set_admin_password(db, new_password).await?;
    Ok(Ok(()))
}

pub async fn verify_admin_password(db: &HopDb, password: &str) -> Result<bool> {
    let Some(hash) = db.get_setting(ADMIN_PASSWORD_HASH).await? else {
        return Ok(false);
    };
    verify_password(&hash, password)
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow!("hash admin password: {err}"))?
        .to_string())
}

pub fn verify_password(hash: &str, password: &str) -> Result<bool> {
    let parsed =
        PasswordHash::new(hash).map_err(|err| anyhow!("parse admin password hash: {err}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn generate_password() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_verifies_and_rejects_wrong_password() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password(&hash, "correct horse").unwrap());
        assert!(!verify_password(&hash, "wrong horse").unwrap());
        assert!(!hash.contains("correct horse"));
    }

    #[tokio::test]
    async fn change_admin_password_requires_current_password_and_confirmation() {
        let db = HopDb::in_memory().await.unwrap();
        set_admin_password(&db, "old-pass").await.unwrap();

        assert_eq!(
            change_admin_password(&db, "wrong-pass", "new-pass", "new-pass")
                .await
                .unwrap(),
            Err(AdminPasswordChangeError::CurrentPasswordInvalid)
        );
        assert!(verify_admin_password(&db, "old-pass").await.unwrap());

        assert_eq!(
            change_admin_password(&db, "old-pass", "new-pass", "different")
                .await
                .unwrap(),
            Err(AdminPasswordChangeError::ConfirmationMismatch)
        );
        assert!(verify_admin_password(&db, "old-pass").await.unwrap());

        assert_eq!(
            change_admin_password(&db, "old-pass", "", "")
                .await
                .unwrap(),
            Err(AdminPasswordChangeError::NewPasswordEmpty)
        );
        assert!(verify_admin_password(&db, "old-pass").await.unwrap());

        assert_eq!(
            change_admin_password(&db, "old-pass", "new-pass", "new-pass")
                .await
                .unwrap(),
            Ok(())
        );
        assert!(!verify_admin_password(&db, "old-pass").await.unwrap());
        assert!(verify_admin_password(&db, "new-pass").await.unwrap());
    }
}
