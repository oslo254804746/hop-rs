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
    NewPasswordTooShort,
    ConfirmationMismatch,
}

pub async fn ensure_admin_password(db: &HopDb) -> Result<Option<String>> {
    if let Some(hash) = db.get_setting(ADMIN_PASSWORD_HASH).await? {
        if db.get_admin_password_hash("local-admin").await?.is_none() {
            db.set_admin_password_hash("local-admin", &hash, false)
                .await?;
        }
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
    db.set_admin_password_hash("local-admin", &hash, false)
        .await?;
    db.set_setting(FIRST_RUN_COMPLETED, "true").await?;
    Ok(())
}

pub async fn change_admin_user_password(
    db: &HopDb,
    admin_id: &str,
    current_password: &str,
    new_password: &str,
    confirm_password: &str,
) -> Result<std::result::Result<(), AdminPasswordChangeError>> {
    if new_password.is_empty() {
        return Ok(Err(AdminPasswordChangeError::NewPasswordEmpty));
    }
    if new_password.chars().count() < 12 {
        return Ok(Err(AdminPasswordChangeError::NewPasswordTooShort));
    }
    if new_password != confirm_password {
        return Ok(Err(AdminPasswordChangeError::ConfirmationMismatch));
    }
    if !verify_admin_user_password(db, admin_id, current_password).await? {
        return Ok(Err(AdminPasswordChangeError::CurrentPasswordInvalid));
    }
    let hash = hash_password(new_password)?;
    db.set_admin_password_hash(admin_id, &hash, false).await?;
    if admin_id == "local-admin" {
        db.set_setting(ADMIN_PASSWORD_HASH, &hash).await?;
    }
    Ok(Ok(()))
}

pub async fn verify_admin_user_password(
    db: &HopDb,
    admin_id: &str,
    password: &str,
) -> Result<bool> {
    let hash = match db.get_admin_password_hash(admin_id).await? {
        Some(hash) => Some(hash),
        None if admin_id == "local-admin" => db.get_setting(ADMIN_PASSWORD_HASH).await?,
        None => None,
    };
    let Some(hash) = hash else {
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
            change_admin_user_password(
                &db,
                "local-admin",
                "wrong-pass",
                "new-password-2026",
                "new-password-2026",
            )
            .await
            .unwrap(),
            Err(AdminPasswordChangeError::CurrentPasswordInvalid)
        );
        assert!(verify_admin_user_password(&db, "local-admin", "old-pass")
            .await
            .unwrap());

        assert_eq!(
            change_admin_user_password(
                &db,
                "local-admin",
                "old-pass",
                "new-password",
                "different",
            )
            .await
            .unwrap(),
            Err(AdminPasswordChangeError::ConfirmationMismatch)
        );
        assert!(verify_admin_user_password(&db, "local-admin", "old-pass")
            .await
            .unwrap());

        assert_eq!(
            change_admin_user_password(&db, "local-admin", "old-pass", "", "")
                .await
                .unwrap(),
            Err(AdminPasswordChangeError::NewPasswordEmpty)
        );
        assert!(verify_admin_user_password(&db, "local-admin", "old-pass")
            .await
            .unwrap());

        assert_eq!(
            change_admin_user_password(&db, "local-admin", "old-pass", "short", "short")
                .await
                .unwrap(),
            Err(AdminPasswordChangeError::NewPasswordTooShort)
        );
        assert!(verify_admin_user_password(&db, "local-admin", "old-pass")
            .await
            .unwrap());

        assert_eq!(
            change_admin_user_password(
                &db,
                "local-admin",
                "old-pass",
                "new-password-2026",
                "new-password-2026",
            )
            .await
            .unwrap(),
            Ok(())
        );
        assert!(!verify_admin_user_password(&db, "local-admin", "old-pass")
            .await
            .unwrap());
        assert!(
            verify_admin_user_password(&db, "local-admin", "new-password-2026")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn each_admin_has_an_independent_password_and_first_change_clears_temporary_state() {
        use hop_core::{NewAdminUser, ADMIN_PROFILE_OPERATOR};

        let db = HopDb::in_memory().await.unwrap();
        set_admin_password(&db, "local-password-2026")
            .await
            .unwrap();
        let teammate_hash = hash_password("temporary-password-2026").unwrap();
        let teammate = db
            .add_admin_user(NewAdminUser {
                username: "ops".to_string(),
                display_name: "Operations".to_string(),
                password_hash: teammate_hash,
                access_profile: ADMIN_PROFILE_OPERATOR.to_string(),
                must_change_password: true,
            })
            .await
            .unwrap();

        assert!(
            verify_admin_user_password(&db, "local-admin", "local-password-2026")
                .await
                .unwrap()
        );
        assert!(
            !verify_admin_user_password(&db, &teammate.id, "local-password-2026")
                .await
                .unwrap()
        );
        assert!(
            verify_admin_user_password(&db, &teammate.id, "temporary-password-2026")
                .await
                .unwrap()
        );

        assert_eq!(
            change_admin_user_password(
                &db,
                &teammate.id,
                "temporary-password-2026",
                "operations-password-2026",
                "operations-password-2026",
            )
            .await
            .unwrap(),
            Ok(())
        );
        let updated = db
            .get_admin_user_by_id(&teammate.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!updated.must_change_password);
        assert!(
            verify_admin_user_password(&db, &teammate.id, "operations-password-2026")
                .await
                .unwrap()
        );
        assert!(
            verify_admin_user_password(&db, "local-admin", "local-password-2026")
                .await
                .unwrap()
        );
    }
}
