use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermesInstance {
    pub found: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub token: Option<String>,
}

impl HermesInstance {
    pub fn not_found() -> Self {
        Self {
            found: false,
            pid: None,
            port: None,
            token: None,
        }
    }
}

/// Linux discovery via ~/.hermes/gateway_state.json or /proc
#[cfg(target_os = "linux")]
pub fn discover_hermes() -> HermesInstance {
    // 1. Fast path: check ~/.hermes/gateway_state.json and gateway.pid
    if let Some(home) = dirs::home_dir() {
        let hermes_dir = home.join(".hermes");
        let state_file = hermes_dir.join("gateway_state.json");
        let pid_file = hermes_dir.join("gateway.pid");

        if let Ok(state_str) = fs::read_to_string(&state_file) {
            if let Ok(state) = serde_json::from_str::<serde_json::Value>(&state_str) {
                let pid = state.get("pid").and_then(|p| p.as_u64()).map(|p| p as u32)
                    .or_else(|| {
                        fs::read_to_string(&pid_file).ok().and_then(|s| {
                            serde_json::from_str::<serde_json::Value>(&s).ok()
                                .and_then(|v| v.get("pid").and_then(|p| p.as_u64()).map(|p| p as u32))
                        })
                    });

                if let Some(p) = pid {
                    // Check if PID is alive
                    if PathBuf::from(format!("/proc/{}", p)).exists() {
                        // Resolve port from listener_base or resolve_listen_port
                        let mut port = None;
                        if let Some(platforms) = state.get("platforms").and_then(|pl| pl.as_object()) {
                            for (_name, plat) in platforms {
                                if let Some(base) = plat.get("listener_base").and_then(|b| b.as_str()) {
                                    if let Some(p_str) = base.rsplit(':').next() {
                                        if let Ok(p_num) = p_str.trim_end_matches('/').parse::<u16>() {
                                            port = Some(p_num);
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if port.is_none() {
                            port = resolve_listen_port(p);
                        }

                        let token = state.get("auth_token")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| read_env_var(p, "HERMES_DASHBOARD_SESSION_TOKEN"))
                            .or_else(|| Some("unai-local-token".to_string()));

                        return HermesInstance {
                            found: true,
                            pid: Some(p),
                            port,
                            token,
                        };
                    }
                }
            }
        }
    }

    // 2. Fallback: scan /proc
    let proc_dir = match fs::read_dir("/proc") {
        Ok(dir) => dir,
        Err(_) => return HermesInstance::not_found(),
    };

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let pid: u32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let cmdline_path = entry.path().join("cmdline");
        if let Ok(cmd_bytes) = fs::read(&cmdline_path) {
            let cmdline = String::from_utf8_lossy(&cmd_bytes).replace('\0', " ");
            let lc = cmdline.to_lowercase();
            if lc.contains("hermes") && (lc.contains("gateway") || lc.contains("serve") || lc.contains("desktop") || lc.contains("hermes-agent")) {
                let token = read_env_var(pid, "HERMES_DASHBOARD_SESSION_TOKEN")
                    .or_else(|| Some("unai-local-token".to_string()));
                let port = resolve_listen_port(pid);
                return HermesInstance {
                    found: true,
                    pid: Some(pid),
                    port,
                    token,
                };
            }
        }
    }

    HermesInstance::not_found()
}

#[cfg(not(target_os = "linux"))]
pub fn discover_hermes() -> HermesInstance {
    HermesInstance::not_found()
}

#[cfg(target_os = "linux")]
fn read_env_var(pid: u32, target_key: &str) -> Option<String> {
    let environ_path = format!("/proc/{}/environ", pid);
    if let Ok(bytes) = fs::read(environ_path) {
        for entry in bytes.split(|&b| b == 0) {
            let s = String::from_utf8_lossy(entry);
            if let Some((k, v)) = s.split_once('=') {
                if k == target_key {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn resolve_listen_port(pid: u32) -> Option<u16> {
    // 1. Check socket inodes via /proc/<pid>/fd/
    let fd_dir = match fs::read_dir(format!("/proc/{}/fd", pid)) {
        Ok(d) => d,
        Err(_) => return None,
    };

    let mut inodes = std::collections::HashSet::new();
    for entry in fd_dir.flatten() {
        if let Ok(target) = fs::read_link(entry.path()) {
            let ts = target.to_string_lossy();
            if ts.starts_with("socket:[") && ts.ends_with(']') {
                let ino_str = &ts[8..ts.len() - 1];
                if let Ok(ino) = ino_str.parse::<u64>() {
                    inodes.insert(ino);
                }
            }
        }
    }

    if inodes.is_empty() {
        return None;
    }

    // 2. Scan /proc/net/tcp
    if let Ok(tcp_content) = fs::read_to_string("/proc/net/tcp") {
        for line in tcp_content.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 10 {
                continue;
            }
            let state = cols[3];
            if state != "0A" {
                // Not LISTEN
                continue;
            }
            if let Ok(inode) = cols[9].parse::<u64>() {
                if inodes.contains(&inode) {
                    if let Some((_, port_hex)) = cols[1].split_once(':') {
                        if let Ok(port) = u16::from_str_radix(port_hex, 16) {
                            if port != 8790 {
                                return Some(port);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}
