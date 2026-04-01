use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::auth::jwt::{validate_token, Claims};
use crate::error::AppError;
use crate::AppState;

pub struct AuthUser(pub Claims);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing authorization header".into()))?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("Invalid authorization format".into()))?;

        let claims = validate_token(token, &state.config.jwt_secret)?;

        Ok(AuthUser(claims))
    }
}
