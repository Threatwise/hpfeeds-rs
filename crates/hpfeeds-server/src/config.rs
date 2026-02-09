use serde::Deserialize;
use std::fs;
use anyhow::Result;

#[derive(Debug, Deserialize, Clone)]
pub struct UserConfig {
    pub ident: String,
    pub secret: String,
    pub pub_channels: Vec<String>,
    pub sub_channels: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub users: Vec<UserConfig>,
}

pub fn load_config(path: &str) -> Result<ServerConfig> {
    let content = fs::read_to_string(path)?;
    let config: ServerConfig = serde_json::from_str(&content)?;
    Ok(config)
}
