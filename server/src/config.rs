use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppConfigError {
    #[error("Missing environment variable: {0}")]
    MissingVar(String),

    #[error("Invalid value for {key}: {reason}")]
    InvalidValue { key: String, reason: String },

    #[error("Insecure configuration for {key}: {reason}")]
    InsecureForProduction { key: String, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}

impl Environment {
    pub fn is_production(self) -> bool {
        matches!(self, Environment::Production)
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub env: Environment,
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
        // dotenvy is invoked once from main.rs (not here) so tests can
        // exercise from_env() against a controlled process env without the
        // .env file silently re-populating removed vars.

        let env = match std::env::var("NEXUS_ENV")
            .unwrap_or_else(|_| "development".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "production" | "prod" => Environment::Production,
            "development" | "dev" | "" => Environment::Development,
            other => {
                return Err(AppConfigError::InvalidValue {
                    key: "NEXUS_ENV".into(),
                    reason: format!("unknown environment: {other}"),
                });
            }
        };

        let jwt_secret = require_env("JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            return Err(AppConfigError::InvalidValue {
                key: "JWT_SECRET".into(),
                reason: "must be at least 32 characters".into(),
            });
        }

        let surreal_user = require_env("SURREAL_USER")?;
        let surreal_pass = require_env("SURREAL_PASS")?;

        let secure_cookies = std::env::var("SECURE_COOKIES")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let cors_origin_raw = std::env::var("CORS_ORIGIN");

        if env.is_production() {
            // Reject default / weak credentials.
            if surreal_user == "root" {
                return Err(AppConfigError::InsecureForProduction {
                    key: "SURREAL_USER".into(),
                    reason: "must not be the default 'root' in production".into(),
                });
            }
            if surreal_pass == "root" {
                return Err(AppConfigError::InsecureForProduction {
                    key: "SURREAL_PASS".into(),
                    reason: "must not be the default 'root' in production".into(),
                });
            }
            if surreal_pass.len() < 16 {
                return Err(AppConfigError::InsecureForProduction {
                    key: "SURREAL_PASS".into(),
                    reason: "must be at least 16 characters in production".into(),
                });
            }
            if !secure_cookies {
                return Err(AppConfigError::InsecureForProduction {
                    key: "SECURE_COOKIES".into(),
                    reason: "must be 'true' in production".into(),
                });
            }
            if cors_origin_raw.is_err() {
                return Err(AppConfigError::InsecureForProduction {
                    key: "CORS_ORIGIN".into(),
                    reason: "must be explicitly set in production (no localhost default)".into(),
                });
            }
            // Recommended: detect example placeholder secrets.
            if jwt_secret.starts_with("change-me") {
                return Err(AppConfigError::InsecureForProduction {
                    key: "JWT_SECRET".into(),
                    reason: "must not be the example placeholder in production".into(),
                });
            }
        }

        let cors_origin = cors_origin_raw.unwrap_or_else(|_| "http://localhost:3000".to_string());

        Ok(Self {
            env,
            surreal_url: require_env("SURREAL_URL")?,
            surreal_ns: require_env("SURREAL_NS")?,
            surreal_db: require_env("SURREAL_DB")?,
            surreal_user,
            surreal_pass,
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
            secure_cookies,
            cors_origin,
            server_host: require_env("SERVER_HOST")?,
            server_port: require_env("SERVER_PORT")?.parse::<u16>().map_err(|_| {
                AppConfigError::InvalidValue {
                    key: "SERVER_PORT".into(),
                    reason: "must be a valid port number".into(),
                }
            })?,
        })
    }
}

fn require_env(key: &str) -> Result<String, AppConfigError> {
    std::env::var(key).map_err(|_| AppConfigError::MissingVar(key.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests in this module mutate the process environment. A static Mutex
    // serializes them against each other AND against any other env-touching
    // test in the same binary, since `cargo test` runs `#[test]` functions
    // in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for k in [
            "NEXUS_ENV",
            "SURREAL_URL",
            "SURREAL_NS",
            "SURREAL_DB",
            "SURREAL_USER",
            "SURREAL_PASS",
            "JWT_SECRET",
            "ACCESS_TOKEN_EXPIRY_MINUTES",
            "REFRESH_TOKEN_EXPIRY_DAYS",
            "REDIS_URL",
            "SERVER_HOST",
            "SERVER_PORT",
            "SECURE_COOKIES",
            "CORS_ORIGIN",
        ] {
            std::env::remove_var(k);
        }
    }

    fn set_minimum_dev_env() {
        clear_env();
        std::env::set_var("NEXUS_ENV", "development");
        std::env::set_var("SURREAL_URL", "ws://localhost:8000");
        std::env::set_var("SURREAL_NS", "nexus");
        std::env::set_var("SURREAL_DB", "nexus");
        std::env::set_var("SURREAL_USER", "root");
        std::env::set_var("SURREAL_PASS", "root");
        std::env::set_var("JWT_SECRET", "a".repeat(32));
        std::env::set_var("ACCESS_TOKEN_EXPIRY_MINUTES", "15");
        std::env::set_var("REFRESH_TOKEN_EXPIRY_DAYS", "7");
        std::env::set_var("REDIS_URL", "redis://localhost:6379");
        std::env::set_var("SERVER_HOST", "127.0.0.1");
        std::env::set_var("SERVER_PORT", "3001");
    }

    // NOTE: these tests mutate the process environment. We hold ENV_LOCK
    // for the entire duration to serialize against any other env-touching
    // test, and exercise all cases in one function so the lock is held once.
    #[test]
    fn env_validation_cases() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Case: dev defaults work
        set_minimum_dev_env();
        let cfg = AppConfig::from_env().expect("dev config should load");
        assert_eq!(cfg.env, Environment::Development);

        // Case: production rejects root user
        set_minimum_dev_env();
        std::env::set_var("NEXUS_ENV", "production");
        std::env::set_var("SECURE_COOKIES", "true");
        std::env::set_var("CORS_ORIGIN", "https://example.com");
        std::env::set_var("SURREAL_PASS", "a-very-strong-password-1");
        let err = AppConfig::from_env().expect_err("must reject root user");
        assert!(
            matches!(err, AppConfigError::InsecureForProduction { ref key, .. } if key == "SURREAL_USER")
        );

        // Case: production rejects short surreal pass
        set_minimum_dev_env();
        std::env::set_var("NEXUS_ENV", "production");
        std::env::set_var("SURREAL_USER", "nexus-admin");
        std::env::set_var("SURREAL_PASS", "short");
        std::env::set_var("SECURE_COOKIES", "true");
        std::env::set_var("CORS_ORIGIN", "https://example.com");
        let err = AppConfig::from_env().expect_err("must reject short pass");
        assert!(
            matches!(err, AppConfigError::InsecureForProduction { ref key, .. } if key == "SURREAL_PASS")
        );

        // Case: production requires SECURE_COOKIES
        set_minimum_dev_env();
        std::env::set_var("NEXUS_ENV", "production");
        std::env::set_var("SURREAL_USER", "nexus-admin");
        std::env::set_var("SURREAL_PASS", "a-very-strong-password-1");
        std::env::set_var("SECURE_COOKIES", "false");
        std::env::set_var("CORS_ORIGIN", "https://example.com");
        let err = AppConfig::from_env().expect_err("must reject insecure cookies");
        assert!(
            matches!(err, AppConfigError::InsecureForProduction { ref key, .. } if key == "SECURE_COOKIES")
        );

        // Case: production requires CORS_ORIGIN
        set_minimum_dev_env();
        std::env::set_var("NEXUS_ENV", "production");
        std::env::set_var("SURREAL_USER", "nexus-admin");
        std::env::set_var("SURREAL_PASS", "a-very-strong-password-1");
        std::env::set_var("SECURE_COOKIES", "true");
        std::env::remove_var("CORS_ORIGIN");
        let err = AppConfig::from_env().expect_err("must require CORS_ORIGIN");
        assert!(
            matches!(err, AppConfigError::InsecureForProduction { ref key, .. } if key == "CORS_ORIGIN")
        );

        // Case: production rejects placeholder JWT secret
        set_minimum_dev_env();
        std::env::set_var("NEXUS_ENV", "production");
        std::env::set_var("SURREAL_USER", "nexus-admin");
        std::env::set_var("SURREAL_PASS", "a-very-strong-password-1");
        std::env::set_var("SECURE_COOKIES", "true");
        std::env::set_var("CORS_ORIGIN", "https://example.com");
        std::env::set_var(
            "JWT_SECRET",
            "change-me-to-a-random-64-char-string-that-is-long",
        );
        let err = AppConfig::from_env().expect_err("must reject placeholder secret");
        assert!(
            matches!(err, AppConfigError::InsecureForProduction { ref key, .. } if key == "JWT_SECRET")
        );

        // Cleanup so we don't pollute other tests.
        clear_env();
    }
}
