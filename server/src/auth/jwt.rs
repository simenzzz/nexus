use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub token_type: String,
    pub exp: usize,
    pub iat: usize,
}

pub fn create_access_token(
    user_id: &str,
    secret: &str,
    expiry_minutes: u64,
) -> Result<String, AppError> {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        token_type: "access".to_string(),
        iat: now,
        exp: now + (expiry_minutes as usize * 60),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(token)
}

pub fn validate_access_token(token: &str, secret: &str) -> Result<Claims, AppError> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;

    if token_data.claims.token_type != "access" {
        return Err(AppError::Unauthorized("Invalid token type".into()));
    }

    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-not-for-production";

    #[test]
    fn sign_and_verify_roundtrip() {
        let token = create_access_token("user42", SECRET, 15).expect("sign");
        let claims = validate_access_token(&token, SECRET).expect("verify");
        assert_eq!(claims.sub, "user42");
        assert_eq!(claims.token_type, "access");
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn wrong_secret_fails_validation() {
        let token = create_access_token("user42", SECRET, 15).expect("sign");
        let result = validate_access_token(&token, "different-secret");
        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[test]
    fn expired_token_rejected() {
        // Issue a token that expired one hour ago by manually constructing claims.
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = Claims {
            sub: "user42".into(),
            token_type: "access".into(),
            iat: now - 7200,
            exp: now - 3600,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("sign");

        let result = validate_access_token(&token, SECRET);
        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[test]
    fn non_access_token_type_rejected() {
        // Hand-craft a token with token_type = "refresh".
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = Claims {
            sub: "user42".into(),
            token_type: "refresh".into(),
            iat: now,
            exp: now + 3600,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("sign");

        let result = validate_access_token(&token, SECRET);
        match result {
            Err(AppError::Unauthorized(msg)) => assert!(msg.contains("token type")),
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }
}
