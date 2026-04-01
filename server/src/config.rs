use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppConfigError {
    #[error("Missing environment variable: {0}")]
    MissingVar(String),

    #[error("Invalid port number: {0}")]
    InvalidPort(String),
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub surreal_url: String,
    pub surreal_ns: String,
    pub surreal_db: String,
    pub surreal_user: String,
    pub surreal_pass: String,
    pub jwt_secret: String,
    pub jwt_expiry_hours: u64,
    pub server_host: String,
    pub server_port: u16,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppConfigError> {
        dotenvy::dotenv().ok();

        Ok(Self {
            surreal_url: require_env("SURREAL_URL")?,
            surreal_ns: require_env("SURREAL_NS")?,
            surreal_db: require_env("SURREAL_DB")?,
            surreal_user: require_env("SURREAL_USER")?,
            surreal_pass: require_env("SURREAL_PASS")?,
            jwt_secret: require_env("JWT_SECRET")?,
            jwt_expiry_hours: require_env("JWT_EXPIRY_HOURS")?
                .parse::<u64>()
                .map_err(|_| AppConfigError::InvalidPort("JWT_EXPIRY_HOURS".into()))?,
            server_host: require_env("SERVER_HOST")?,
            server_port: require_env("SERVER_PORT")?
                .parse::<u16>()
                .map_err(|_| AppConfigError::InvalidPort("SERVER_PORT".into()))?,
        })
    }
}

fn require_env(key: &str) -> Result<String, AppConfigError> {
    std::env::var(key).map_err(|_| AppConfigError::MissingVar(key.into()))
}
