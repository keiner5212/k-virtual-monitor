//! Server configuration.
//!
//! Every knob lives in `~/.config/kvm/config.toml`. The defaults are the
//! ones the protocol was sized for: 1408x792, 20 fps, a window of 8 frames.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// TCP port for the data channel.
    pub port: u16,
    /// Address the server binds to.
    pub bind: String,
    /// Name shown on the phone.
    pub server_name: String,
    /// Frame width in pixels.
    pub width: u16,
    /// Frame height in pixels.
    pub height: u16,
    /// Target frame rate.
    pub fps: u16,
    /// JPEG quality used by the fallback codec.
    pub jpeg_quality: u8,
    /// Preferred codec: `h264` or `jpeg`.
    pub codec: String,
    /// Maximum number of authenticated sessions at the same time.
    pub max_sessions: usize,
    /// Maximum number of handshakes in progress at the same time.
    pub max_pending_handshakes: usize,
    /// Seconds a connection has to finish the handshake.
    pub handshake_timeout_secs: u64,
    /// Seconds a session may stay silent before it is closed.
    pub session_idle_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            port: 28252,
            bind: "0.0.0.0".to_string(),
            server_name: "k-virtual-monitor".to_string(),
            width: 1408,
            height: 792,
            fps: 20,
            jpeg_quality: 70,
            codec: "h264".to_string(),
            max_sessions: 4,
            max_pending_handshakes: 8,
            handshake_timeout_secs: 10,
            session_idle_timeout_secs: 120,
        }
    }
}

impl ServerConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(ServerConfig::default());
        }
        let text = std::fs::read_to_string(path)?;
        let config: ServerConfig = toml::from_str(&text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.width == 0 || self.height == 0 {
            return Err(ConfigError::Invalid(
                "width and height must be positive".into(),
            ));
        }
        if self.fps == 0 || self.fps > 120 {
            return Err(ConfigError::Invalid("fps must be between 1 and 120".into()));
        }
        if !(1..=100).contains(&self.jpeg_quality) {
            return Err(ConfigError::Invalid(
                "jpeg_quality must be between 1 and 100".into(),
            ));
        }
        if self.max_sessions == 0 {
            return Err(ConfigError::Invalid("max_sessions must be positive".into()));
        }
        Ok(())
    }
}

/// Path of the configuration file, following the XDG base directory spec.
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".config")
        });
    base.join("kvm").join("config.toml")
}

/// Path of the paired device store.
pub fn device_store_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".local").join("state")
        });
    base.join("kvm").join("devices.json")
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read the configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot parse the configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        ServerConfig::default().validate().unwrap();
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        let error = toml::from_str::<ServerConfig>("nope = 1").unwrap_err();
        assert!(error.to_string().contains("nope"));
    }

    #[test]
    fn a_zero_fps_is_rejected() {
        let config = ServerConfig {
            fps: 0,
            ..ServerConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
