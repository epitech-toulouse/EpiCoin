//! Configuration, read from environment variables (see `.env.example`).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context};
use epicoin_core::{parse_epc, Amount, Keypair};

use crate::vault::MasterKey;

pub enum MailerConfig {
    /// Log emails instead of sending them (development).
    Console,
    Smtp {
        url: String,
        from: String,
    },
}

/// Everything the API needs to start. Holds secrets: deliberately not `Debug`.
pub struct Config {
    pub database_url: String,
    pub node_url: String,
    pub listen: SocketAddr,
    pub mailer: MailerConfig,
    pub master_key: MasterKey,
    pub previous_master_key: Option<MasterKey>,
    /// The chain's admin key: signs authority updates.
    pub admin_key: Keypair,
    /// The shop treasury key: signs refunds.
    pub treasury_key: Keypair,
    pub settings: Settings,
}

/// The non-secret part of the configuration, available to request handlers.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Public base URL of the site, used in emailed links. No trailing slash.
    pub public_url: String,
    /// Only emails of this domain can have an account.
    pub allowed_email_domain: String,
    /// Mark the session cookie `Secure`. Only disable for local development over HTTP.
    pub cookie_secure: bool,
    pub session_ttl: Duration,
    pub invite_ttl: Duration,
    pub reset_ttl: Duration,
    /// Mint quota per period of a staff member without a personal quota, in units.
    pub default_mint_quota: Amount,
    /// Take the client address from `X-Forwarded-For` (set when behind the nginx proxy).
    pub trust_proxy: bool,
    /// Where product images are stored.
    pub upload_dir: PathBuf,
}

impl Settings {
    /// Sensible values for tests and local development.
    pub fn development() -> Self {
        Self {
            public_url: "http://localhost:8088".into(),
            allowed_email_domain: "epitech.eu".into(),
            cookie_secure: false,
            session_ttl: Duration::from_secs(7 * 24 * 3600),
            invite_ttl: Duration::from_secs(7 * 24 * 3600),
            reset_ttl: Duration::from_secs(3600),
            default_mint_quota: 500 * epicoin_core::UNITS_PER_EPC,
            trust_proxy: false,
            upload_dir: PathBuf::from("./data/uploads"),
        }
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Builds the configuration from any source of variables (the environment, or a map in
    /// tests). Error messages name the variable, never its value.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let get = |name: &str| lookup(name).filter(|v| !v.trim().is_empty());
        let required = |name: &str| get(name).with_context(|| format!("{name} is not set"));
        let flag = |name: &str, default: bool| -> anyhow::Result<bool> {
            match get(name).as_deref() {
                None => Ok(default),
                Some("true" | "1" | "yes") => Ok(true),
                Some("false" | "0" | "no") => Ok(false),
                Some(_) => bail!("{name} must be true or false"),
            }
        };
        let hours = |name: &str, default: u64| -> anyhow::Result<Duration> {
            let value = match get(name) {
                None => default,
                Some(text) => text
                    .parse()
                    .ok()
                    .filter(|h| *h > 0)
                    .with_context(|| format!("{name} must be a positive number of hours"))?,
            };
            Ok(Duration::from_secs(value * 3600))
        };
        let keypair = |name: &str| -> anyhow::Result<Keypair> {
            Keypair::from_secret_hex(&required(name)?)
                .ok()
                .with_context(|| format!("{name} must be 64 hex characters"))
        };
        let master = |name: &str, value: String| -> anyhow::Result<MasterKey> {
            MasterKey::from_hex(&value)
                .ok()
                .with_context(|| format!("{name} must be 64 hex characters"))
        };

        let database_url = get("EPICOIN_DATABASE_URL")
            .or_else(|| get("DATABASE_URL"))
            .context("EPICOIN_DATABASE_URL is not set")?;
        let listen = get("EPICOIN_API_LISTEN")
            .unwrap_or_else(|| "127.0.0.1:8090".into())
            .parse()
            .context("EPICOIN_API_LISTEN must be an address like 0.0.0.0:8090")?;

        let mailer = match get("EPICOIN_MAILER").as_deref().unwrap_or("console") {
            "console" => MailerConfig::Console,
            "smtp" => MailerConfig::Smtp {
                url: required("EPICOIN_SMTP_URL")?,
                from: required("EPICOIN_MAIL_FROM")?,
            },
            _ => bail!("EPICOIN_MAILER must be `console` or `smtp`"),
        };

        let default_mint_quota = match get("EPICOIN_DEFAULT_MINT_QUOTA_EPC") {
            None => 500 * epicoin_core::UNITS_PER_EPC,
            Some(text) => parse_epc(&text)
                .context("EPICOIN_DEFAULT_MINT_QUOTA_EPC must be an EPC amount, e.g. 500")?,
        };

        let public_url = required("EPICOIN_PUBLIC_URL")?
            .trim_end_matches('/')
            .to_owned();
        let cookie_secure = flag("EPICOIN_COOKIE_SECURE", true)?;
        if cookie_secure && !public_url.starts_with("https://") {
            bail!(
                "EPICOIN_PUBLIC_URL is not HTTPS: browsers would drop the Secure session cookie. \
                 Use an https:// URL, or set EPICOIN_COOKIE_SECURE=false for local development"
            );
        }

        Ok(Self {
            database_url,
            node_url: get("EPICOIN_NODE_URL").unwrap_or_else(|| "http://127.0.0.1:7400".into()),
            listen,
            mailer,
            master_key: master("EPICOIN_MASTER_KEY", required("EPICOIN_MASTER_KEY")?)?,
            previous_master_key: get("EPICOIN_MASTER_KEY_PREVIOUS")
                .map(|value| master("EPICOIN_MASTER_KEY_PREVIOUS", value))
                .transpose()?,
            admin_key: keypair("EPICOIN_ADMIN_KEY")?,
            treasury_key: keypair("EPICOIN_TREASURY_KEY")?,
            settings: Settings {
                public_url,
                allowed_email_domain: get("EPICOIN_ALLOWED_EMAIL_DOMAIN")
                    .unwrap_or_else(|| "epitech.eu".into())
                    .trim_start_matches('@')
                    .to_ascii_lowercase(),
                cookie_secure,
                session_ttl: hours("EPICOIN_SESSION_TTL_HOURS", 7 * 24)?,
                invite_ttl: hours("EPICOIN_INVITE_TTL_HOURS", 7 * 24)?,
                reset_ttl: hours("EPICOIN_RESET_TTL_HOURS", 1)?,
                default_mint_quota,
                trust_proxy: flag("EPICOIN_TRUST_PROXY", false)?,
                upload_dir: get("EPICOIN_UPLOAD_DIR")
                    .unwrap_or_else(|| "./data/uploads".into())
                    .into(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const SECRET_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SECRET_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SECRET_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn base() -> HashMap<&'static str, String> {
        HashMap::from([
            (
                "EPICOIN_DATABASE_URL",
                "postgres://u:p@localhost/db".to_owned(),
            ),
            ("EPICOIN_PUBLIC_URL", "https://epicoin.example/".to_owned()),
            ("EPICOIN_MASTER_KEY", SECRET_A.to_owned()),
            ("EPICOIN_ADMIN_KEY", SECRET_B.to_owned()),
            ("EPICOIN_TREASURY_KEY", SECRET_C.to_owned()),
        ])
    }

    fn load(vars: &HashMap<&'static str, String>) -> anyhow::Result<Config> {
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn minimal_configuration_gets_safe_defaults() {
        let config = load(&base()).unwrap();
        assert!(matches!(config.mailer, MailerConfig::Console));
        assert_eq!(config.settings.public_url, "https://epicoin.example");
        assert_eq!(config.settings.allowed_email_domain, "epitech.eu");
        assert!(config.settings.cookie_secure);
        assert!(!config.settings.trust_proxy);
        assert_eq!(config.settings.default_mint_quota, 50_000);
        assert_eq!(config.settings.session_ttl, Duration::from_secs(604_800));
        assert_eq!(config.listen.port(), 8090);
        assert!(config.previous_master_key.is_none());
    }

    #[test]
    fn missing_or_malformed_secrets_are_reported_by_name_only() {
        for name in [
            "EPICOIN_MASTER_KEY",
            "EPICOIN_ADMIN_KEY",
            "EPICOIN_TREASURY_KEY",
        ] {
            let mut vars = base();
            vars.remove(name);
            let error = format!("{:#}", load(&vars).err().unwrap());
            assert!(error.contains(name), "{error}");

            let mut vars = base();
            vars.insert(name, "not-hex-but-very-secret".into());
            let error = format!("{:#}", load(&vars).err().unwrap());
            assert!(error.contains(name), "{error}");
            assert!(!error.contains("very-secret"), "{error}");
        }
    }

    #[test]
    fn secure_cookie_requires_https() {
        let mut vars = base();
        vars.insert("EPICOIN_PUBLIC_URL", "http://localhost:8088".into());
        assert!(load(&vars).is_err());
        vars.insert("EPICOIN_COOKIE_SECURE", "false".into());
        assert!(!load(&vars).unwrap().settings.cookie_secure);
    }

    #[test]
    fn smtp_mailer_needs_its_settings() {
        let mut vars = base();
        vars.insert("EPICOIN_MAILER", "smtp".into());
        assert!(load(&vars).is_err());
        vars.insert("EPICOIN_SMTP_URL", "smtp://mailpit:1025".into());
        vars.insert("EPICOIN_MAIL_FROM", "EpiCoin <noreply@epitech.eu>".into());
        assert!(matches!(
            load(&vars).unwrap().mailer,
            MailerConfig::Smtp { .. }
        ));
        vars.insert("EPICOIN_MAILER", "pigeon".into());
        assert!(load(&vars).is_err());
    }

    #[test]
    fn overrides_are_parsed() {
        let mut vars = base();
        vars.insert("EPICOIN_ALLOWED_EMAIL_DOMAIN", "@Example.ORG".into());
        vars.insert("EPICOIN_DEFAULT_MINT_QUOTA_EPC", "120.50".into());
        vars.insert("EPICOIN_SESSION_TTL_HOURS", "12".into());
        vars.insert("EPICOIN_TRUST_PROXY", "true".into());
        vars.insert("EPICOIN_MASTER_KEY_PREVIOUS", SECRET_C.into());
        let config = load(&vars).unwrap();
        assert_eq!(config.settings.allowed_email_domain, "example.org");
        assert_eq!(config.settings.default_mint_quota, 12_050);
        assert_eq!(config.settings.session_ttl, Duration::from_secs(43_200));
        assert!(config.settings.trust_proxy);
        assert!(config.previous_master_key.is_some());

        vars.insert("EPICOIN_SESSION_TTL_HOURS", "0".into());
        assert!(load(&vars).is_err());
        vars.insert("EPICOIN_SESSION_TTL_HOURS", "12".into());
        vars.insert("EPICOIN_TRUST_PROXY", "maybe".into());
        assert!(load(&vars).is_err());
    }
}
