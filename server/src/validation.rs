use crate::error::AppError;

pub fn validate_username(username: &str) -> Result<(), AppError> {
    if username.len() < 3 || username.len() > 32 {
        return Err(AppError::BadRequest(
            "Username must be 3-32 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::BadRequest(
            "Username can only contain alphanumeric characters and underscores".into(),
        ));
    }
    Ok(())
}

pub fn validate_password(password: &str) -> Result<(), AppError> {
    if password.len() < 8 || password.len() > 128 {
        return Err(AppError::BadRequest(
            "Password must be 8-128 characters".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_min_length_is_three() {
        assert!(validate_username("ab").is_err());
        assert!(validate_username("abc").is_ok());
    }

    #[test]
    fn username_max_length_is_thirty_two() {
        assert!(validate_username(&"a".repeat(32)).is_ok());
        assert!(validate_username(&"a".repeat(33)).is_err());
    }

    #[test]
    fn username_rejects_non_alphanumeric() {
        assert!(validate_username("a-b").is_err());
        assert!(validate_username("a.b").is_err());
        assert!(validate_username("a b").is_err());
        assert!(validate_username("hello!").is_err());
    }

    #[test]
    fn username_accepts_underscore_and_alphanumeric() {
        assert!(validate_username("user_123").is_ok());
        assert!(validate_username("ABC_xyz").is_ok());
    }

    #[test]
    fn password_min_length_is_eight() {
        assert!(validate_password("1234567").is_err());
        assert!(validate_password("12345678").is_ok());
    }

    #[test]
    fn password_max_length_is_one_twenty_eight() {
        assert!(validate_password(&"x".repeat(128)).is_ok());
        assert!(validate_password(&"x".repeat(129)).is_err());
    }
}
