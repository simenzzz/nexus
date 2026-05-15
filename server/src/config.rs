use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppConfigError {
    #[error("Missing environment variable: {0}")]
    MissingVar(String),

    #[error("Invalid value for {key}: {reason}")]
    InvalidValue { key: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub surreal_url: String,
    pub surreal_ns: String,
    pub surreal_db: String,
    pub surreal_user: String,
    pub surreal_pass: String,
    pub jwt_secret: String,
    pub access_token_expiry_minutes: u64,
    pub refresh_token_expiry_days: u64,
    pub redis_url: String,
    pub server_host: String,
    pub server_port: u16,
    pub secure_cookies: bool,
    pub cors_origin: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppConfigError> {
        dotenvy::dotenv().ok();

        let jwt_secret = require_env("JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            return Err(AppConfigError::InvalidValue {
                key: "JWT_SECRET".into(),
                reason: "must be at least 32 characters".into(),
            });
        }

        Ok(Self {
            surreal_url: require_env("SURREAL_URL")?,
            surreal_ns: require_env("SURREAL_NS")?,
            surreal_db: require_env("SURREAL_DB")?,
            surreal_user: require_env("SURREAL_USER")?,
            surreal_pass: require_env("SURREAL_PASS")?,
            jwt_secret,
            access_token_expiry_minutes: require_env("ACCESS_TOKEN_EXPIRY_MINUTES")?
                .parse::<u64>()
                .map_err(|_| AppConfigError::InvalidValue {
                    key: "ACCESS_TOKEN_EXPIRY_MINUTES".into(),
                    reason: "must be a valid positive integer".into(),
                })?,
            refresh_token_expiry_days: require_env("REFRESH_TOKEN_EXPIRY_DAYS")?
                .parse::<u64>()
                .map_err(|_| AppConfigError::InvalidValue {
                    key: "REFRESH_TOKEN_EXPIRY_DAYS".into(),
                    reason: "must be a valid positive integer".into(),
                })?,
            redis_url: require_env("REDIS_URL")?,
            secure_cookies: std::env::var("SECURE_COOKIES")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            cors_origin: std::env::var("CORS_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:3000".to_string()),
            server_host: require_env("SERVER_HOST")?,
            server_port: require_env("SERVER_PORT")?
                .parse::<u16>()
                .map_err(|_| AppConfigError::InvalidValue {
                    key: "SERVER_PORT".into(),
                    reason: "must be a valid port number".into(),
                })?,
        })
    }
}

fn require_env(key: &str) -> Result<String, AppConfigError> {
    std::env::var(key).map_err(|_| AppConfigError::MissingVar(key.into()))
}
