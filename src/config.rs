use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdbConfig {
    pub host_name: String,
    pub api_key: String,
    pub cloud_url: String,
    pub tunnel_type: String,
    pub tunnel_token: String,
    pub tunnel_url: String,
    #[serde(default)]
    pub streamer_mode: bool,
    pub permissions: PermissionsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsConfig {
    pub display_control: DisplayControlPerms,
    pub screen: bool,
    pub audio: bool,
    pub fs: FsPerms,
    pub exec: bool,
    pub hermes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayControlPerms {
    pub enabled: bool,
    pub allow_click: bool,
    pub allow_type: bool,
    pub allow_keys: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsPerms {
    pub read: bool,
    pub write: bool,
    pub list: bool,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            display_control: DisplayControlPerms {
                enabled: true,
                allow_click: true,
                allow_type: true,
                allow_keys: true,
            },
            screen: true,
            audio: true,
            fs: FsPerms {
                read: true,
                write: false,
                list: true,
            },
            exec: true,
            hermes: true,
        }
    }
}

impl Default for DdbConfig {
    fn default() -> Self {
        let hostname = match std::env::var("HOSTNAME") {
            Ok(h) => h,
            Err(_) => "ponosik".to_string(),
        };
        let api_key = format!("ddb_{:x}", rand::random::<u128>());
        Self {
            host_name: hostname,
            api_key,
            cloud_url: "wss://diromstream.kasperstudios.xyz/api/v1/bridge".to_string(),
            tunnel_type: "lan".to_string(),
            tunnel_token: String::new(),
            tunnel_url: String::new(),
            streamer_mode: false,
            permissions: PermissionsConfig::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Some(mut p) = dirs::config_dir() {
        p.push("ddb");
        p.push("config.json");
        return p;
    }
    PathBuf::from("config.json")
}

pub fn load_config() -> DdbConfig {
    let path = config_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(mut cfg) = serde_json::from_str::<DdbConfig>(&content) {
                // Авто-миграция старого несуществующего домена api.kasperstudios.xyz
                if cfg.cloud_url.contains("api.kasperstudios.xyz") || cfg.cloud_url.is_empty() {
                    cfg.cloud_url = "wss://diromstream.kasperstudios.xyz/api/v1/bridge".to_string();
                    let _ = save_config(&cfg);
                }
                return cfg;
            }
        }
    }

    let default_cfg = DdbConfig::default();
    let _ = save_config(&default_cfg);
    default_cfg
}

pub fn save_config(cfg: &DdbConfig) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(cfg)?;
    fs::write(path, json)?;
    Ok(())
}

impl DdbConfig {
    pub fn is_action_permitted(&self, action: &str, params: &serde_json::Value) -> bool {
        let p = &self.permissions;
        if action == "exec" {
            return p.exec;
        }
        if action == "screen" {
            return p.screen;
        }
        if action.starts_with("hermes.") {
            return p.hermes;
        }
        if action.starts_with("fs.") {
            if action == "fs.read" {
                return p.fs.read;
            }
            if action == "fs.write" {
                return p.fs.write;
            }
            if action == "fs.list" {
                return p.fs.list;
            }
            return false;
        }
        if action == "display_control" {
            if !p.display_control.enabled {
                return false;
            }
            if let Some(sub) = params.get("action").and_then(|v| v.as_str()) {
                match sub {
                    "click" => return p.display_control.allow_click,
                    "type" => return p.display_control.allow_type,
                    "key" => return p.display_control.allow_keys,
                    _ => return true,
                }
            }
            return true;
        }

        // Default allowed for benign status/info actions
        true
    }
}
