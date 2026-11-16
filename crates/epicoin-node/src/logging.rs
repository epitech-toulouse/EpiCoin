//! Logging setup shared by the EpiCoin binaries.

use clap::ValueEnum;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    /// Human-readable, for development.
    Pretty,
    /// One JSON object per line, for production.
    Json,
}

/// Installs the global logger. The level comes from `RUST_LOG` (default `info`).
pub fn init(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Pretty => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}
