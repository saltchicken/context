use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

/// Uses untagged deserialization to be fully backwards-compatible
/// with users who have single strings defined in their current config.toml
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum InstructionsConfig {
    Single(String),
    Map(HashMap<String, String>),
}

#[derive(Deserialize, Debug, Default)]
pub struct UserConfig {
    pub git_root: Option<bool>,
    #[serde(default)]
    pub instructions: Option<InstructionsConfig>,
}

pub fn load_config() -> Result<UserConfig> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    let context_dir = config_dir.join("context");
    let config_path = context_dir.join("config.toml");

    if !config_path.exists() {
        if fs::create_dir_all(&context_dir).is_ok() {
            let default_toml = include_str!("../assets/config.example.toml");
            if let Err(e) = fs::write(&config_path, default_toml) {
                log::warn!("Failed to write default config.toml: {}", e);
            } else {
                log::info!("Created default config file at {:?}", config_path);
            }
        }
        return Ok(UserConfig::default());
    }

    let content = fs::read_to_string(&config_path)
        .context(format!("Failed to read config at {:?}", config_path))?;

    let parsed: UserConfig = toml::from_str(&content).context("Failed to parse config.toml")?;
    Ok(parsed)
}
