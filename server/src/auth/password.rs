use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use crate::error::AppError;

pub async fn hash_password(password: &str) -> Result<String, AppError> {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AppError::Internal(format!("Password hashing failed: {e}")))
    })
    .await
    .map_err(|e| AppError::Internal(format!("Spawn blocking failed: {e}")))?
}

pub async fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let password = password.to_string();
    let hash = hash.to_string();
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&hash)
            .map_err(|e| AppError::Internal(format!("Invalid password hash: {e}")))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await
    .map_err(|e| AppError::Internal(format!("Spawn blocking failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_and_verify_roundtrip() {
        let hash = hash_password("correct horse battery staple").await.unwrap();
        assert!(verify_password("correct horse battery staple", &hash)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn wrong_password_fails_verification() {
        let hash = hash_password("password123").await.unwrap();
        assert!(!verify_password("password124", &hash).await.unwrap());
    }

    #[tokio::test]
    async fn each_hash_uses_a_fresh_salt() {
        let h1 = hash_password("same").await.unwrap();
        let h2 = hash_password("same").await.unwrap();
        assert_ne!(
            h1, h2,
            "two hashes of the same password must differ (random salt)"
        );
    }

    #[tokio::test]
    async fn invalid_hash_format_returns_internal_error() {
        let result = verify_password("anything", "not-an-argon2-hash").await;
        assert!(matches!(result, Err(AppError::Internal(_))));
    }
}
