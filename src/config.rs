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
            return match action {
                "fs.read" | "fs.download" | "fs.stat" => p.fs.read,
                "fs.write" | "fs.upload" | "fs.fetch" => p.fs.write,
                "fs.list" => p.fs.list,
                _ => false,
            };
        }
        if action == "display_control" {
            if !p.display_control.enabled {
                return false;
            }
            let wants_capture_after = match params.get("capture_after") {
                Some(serde_json::Value::Bool(b)) => *b,
                Some(serde_json::Value::Object(_)) => true,
                _ => false,
            };
            if wants_capture_after && !p.screen {
                return false;
            }
            if let Some(actions) = params.get("actions").and_then(|v| v.as_array()) {
                return actions.iter().all(|item| {
                    let sub = item
                        .get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or("click");
                    is_display_subaction_permitted(p, sub)
                });
            }
            if let Some(sub) = params.get("action").and_then(|v| v.as_str()) {
                return is_display_subaction_permitted(p, sub);
            }
            return true;
        }

        // Default allowed for benign status/info actions
        true
    }
}

fn is_display_subaction_permitted(p: &PermissionsConfig, sub: &str) -> bool {
    match sub {
        "click" | "double_click" | "triple_click" | "right_click" | "middle_click"
        | "move" | "mouse_down" | "mouse_up" | "drag" | "scroll" => {
            p.display_control.allow_click
        }
        "type" => p.display_control.allow_type,
        "key" => p.display_control.allow_keys,
        "screen" | "screenshot" => p.screen,
        "wait" | "sleep" => true,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enforces_fs_and_batch_display_permissions() {
        let mut cfg = DdbConfig::default();
        cfg.permissions.fs.read = false;
        cfg.permissions.fs.write = true;
        assert!(!cfg.is_action_permitted("fs.download", &json!({})));
        assert!(!cfg.is_action_permitted("fs.stat", &json!({})));
        assert!(cfg.is_action_permitted("fs.upload", &json!({})));
        assert!(cfg.is_action_permitted("fs.fetch", &json!({})));

        cfg.permissions.display_control.allow_click = false;
        assert!(!cfg.is_action_permitted(
            "display_control",
            &json!({ "actions": [{ "action": "key", "key": "a" }, { "action": "scroll", "dy": -3 }] })
        ));
        assert!(cfg.is_action_permitted(
            "display_control",
            &json!({ "actions": [{ "action": "key", "key": "a" }, { "action": "wait", "ms": 50 }] })
        ));

        cfg.permissions.screen = false;
        assert!(!cfg.is_action_permitted(
            "display_control",
            &json!({ "action": "key", "key": "enter", "capture_after": true })
        ));
    }
}
