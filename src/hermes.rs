use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// Discovered Hermes gateway, as described by the owner's own control socket.
///
/// Since Hermes 0.23 the gateway refuses `?token=` URLs and HTTP `X-Hermes-Session-Token`
/// header auth: every WebSocket client must mint a one-use session ticket over the private
/// control socket (`~/.hermes/gateway.sock`) and offer it as the
/// `hermes-gateway-ticket.<ticket>` subprotocol beside `hermes-gateway-v1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermesInstance {
    pub found: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub token: Option<String>,
    pub api_origin: Option<String>,
    pub instance_id: Option<String>,
    pub profile_id: Option<String>,
    pub state: Option<String>,
    pub code_version: Option<String>,
    pub runtime_protocol: Option<u64>,
}

impl HermesInstance {
    pub fn not_found() -> Self {
        Self {
            found: false,
            pid: None,
            port: None,
            token: None,
            api_origin: None,
            instance_id: None,
            profile_id: None,
            state: None,
            code_version: None,
            runtime_protocol: None,
        }
    }
}

#[cfg(target_os = "linux")]
fn hermes_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HERMES_HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    dirs::home_dir().map(|h| h.join(".hermes"))
}

/// Path of the owner's private control socket (`gateway.sock`, or the `gateway.sock.path` pointer).
#[cfg(target_os = "linux")]
pub fn control_socket_path() -> Option<PathBuf> {
    let home = hermes_home()?;
    let direct = home.join("gateway.sock");
    if direct.exists() {
        return Some(direct);
    }
    let pointer = home.join("gateway.sock.path");
    let target = fs::read_to_string(pointer).ok()?;
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    Some(PathBuf::from(target))
}

/// One request/response exchange on the control socket. Returns the parsed JSON answer.
#[cfg(target_os = "linux")]
fn control_query(request: serde_json::Value) -> Option<serde_json::Value> {
    use std::os::unix::net::UnixStream;
    let path = control_socket_path()?;
    let stream = UnixStream::connect(&path).ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;

    let mut writer = stream.try_clone().ok()?;
    let mut payload = serde_json::to_vec(&request).ok()?;
    payload.push(b'\n');
    writer.write_all(&payload).ok()?;
    writer.flush().ok()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if line.trim().is_empty() {
        return None;
    }
    serde_json::from_str(line.trim()).ok()
}

/// `identify` answer from the owner: api_origin, instance_id, profile, state, capabilities.
#[cfg(target_os = "linux")]
pub fn gateway_identify() -> Option<serde_json::Value> {
    let answer = control_query(serde_json::json!({"protocol": 1, "verb": "identify", "id": 1}))?;
    if answer.get("ok") != Some(&serde_json::json!(true)) {
        return None;
    }
    answer.get("result").cloned()
}

/// Mint a one-use session ticket valid ~30s for the WebSocket upgrade.
#[cfg(target_os = "linux")]
pub fn mint_ticket(profile_id: &str, instance_id: &str) -> Option<String> {
    let request = serde_json::json!({
        "protocol": 1,
        "id": 1,
        "verb": "session-ticket",
        "params": {
            "profile_id": profile_id,
            "instance_id": instance_id,
            "purpose": "interactive"
        }
    });
    let answer = control_query(request)?;
    answer
        .get("result")
        .and_then(|r| r.get("ticket"))
        .and_then(|t| t.as_str())
        .map(|t| t.to_string())
}

/// `(ws_url, subprotocols)` for dialing the owner's canonical WebSocket with a session ticket.
#[cfg(target_os = "linux")]
pub fn gateway_ws_target(api_origin: &str, ticket: &str) -> (String, String) {
    let base = api_origin
        .replace("https:", "wss:")
        .replace("http:", "ws:");
    let url = format!("{}/api/ws", base.trim_end_matches('/'));
    let protocols = format!(
        "hermes-gateway-v1, hermes-gateway-ticket.{}",
        ticket
    );
    (url, protocols)
}

/// Linux discovery: control socket `identify` first, then the legacy state file / /proc scan.
#[cfg(target_os = "linux")]
pub fn discover_hermes() -> HermesInstance {
    if let Some(id) = gateway_identify() {
        let api_origin = id.get("api_origin").and_then(|v| v.as_str()).map(String::from);
        let port = api_origin
            .as_deref()
            .and_then(|o| o.rsplit(':').next())
            .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok());
        let profile_id = id
            .get("profile_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                id.get("served_profiles")
                    .and_then(|s| s.as_array())
                    .and_then(|a| a.first())
                    .and_then(|p| p.get("profile_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
        return HermesInstance {
            found: true,
            pid: id.get("pid").and_then(|p| p.as_u64()).map(|p| p as u32),
            port,
            token: None,
            api_origin,
            instance_id: id.get("instance_id").and_then(|v| v.as_str()).map(String::from),
            profile_id,
            state: id.get("state").and_then(|v| v.as_str()).map(String::from),
            code_version: id.get("code_version").and_then(|v| v.as_str()).map(String::from),
            runtime_protocol: id.get("runtime_protocol").and_then(|v| v.as_u64()),
        };
    }

    // Fallback: state file + /proc (older gateways that published no control socket answer).
    if let Some(home) = hermes_home() {
        let state_file = home.join("gateway_state.json");
        let pid_file = home.join("gateway.pid");

        if let Ok(state_str) = fs::read_to_string(&state_file) {
            if let Ok(state) = serde_json::from_str::<serde_json::Value>(&state_str) {
                let pid = state
                    .get("pid")
                    .and_then(|p| p.as_u64())
                    .map(|p| p as u32)
                    .or_else(|| {
                        fs::read_to_string(&pid_file).ok().and_then(|s| {
                            serde_json::from_str::<serde_json::Value>(&s)
                                .ok()
                                .and_then(|v| v.get("pid").and_then(|p| p.as_u64()).map(|p| p as u32))
                        })
                    });

                if let Some(p) = pid {
                    if PathBuf::from(format!("/proc/{}", p)).exists() {
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

                        return HermesInstance {
                            found: true,
                            pid: Some(p),
                            port,
                            token: None,
                            api_origin: port.map(|pt| format!("http://127.0.0.1:{}", pt)),
                            instance_id: None,
                            profile_id: None,
                            state: state
                                .get("gateway_state")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            code_version: state
                                .get("code_version")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            runtime_protocol: None,
                        };
                    }
                }
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
fn resolve_listen_port(pid: u32) -> Option<u16> {
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

    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = fs::read_to_string(table) {
            for line in content.lines().skip(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() < 10 || cols[3] != "0A" {
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
    }

    None
}
