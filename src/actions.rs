use crate::config::DdbConfig;
#[cfg(feature = "hermes")]
use crate::hermes::discover_hermes;
use crate::pointer::DiromPointer;
use anyhow::Result;
#[cfg(feature = "hermes")]
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
#[cfg(feature = "hermes")]
use tokio_tungstenite::connect_async;
#[cfg(feature = "hermes")]
use tokio_tungstenite::tungstenite::Message;

#[allow(unused_variables)]
pub async fn execute_action(
    action: &str,
    params: serde_json::Value,
    config: &DdbConfig,
    pointer: Option<&Arc<DiromPointer>>,
) -> Result<serde_json::Value> {
    match action {
        // ── 1. Telemetry & Host Diagnostics ─────────────────────────
        "status" => {
            let mut sys = sysinfo::System::new_all();
            sys.refresh_all();
            #[cfg(feature = "hermes")]
            let h = discover_hermes();
            #[cfg(not(feature = "hermes"))]
            let h = json!({ "found": false, "note": "Hermes disabled in public build" });
            Ok(json!({
                "ok": true,
                "host": {
                    "name": config.host_name,
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                    "freemem": sys.available_memory(),
                    "totalmem": sys.total_memory(),
                    "cpus": sys.cpus().len(),
                },
                "hermes": h,
                "tunnel": {
                    "type": config.tunnel_type,
                    "url": config.tunnel_url,
                }
            }))
        }

        "diagnostics" => {
            let mut sys = sysinfo::System::new_all();
            sys.refresh_all();
            #[cfg(feature = "hermes")]
            let h = discover_hermes();
            #[cfg(not(feature = "hermes"))]
            let _h = json!({ "found": false, "port": null });
            let uptime = sysinfo::System::uptime();

            #[cfg(feature = "hermes")]
            let (h_found, h_port) = (h.found, h.port);
            #[cfg(not(feature = "hermes"))]
            let (h_found, h_port) = (false, None::<u16>);

            Ok(json!({
                "ok": true,
                "uptime": uptime,
                "load_average": {
                    "one": sysinfo::System::load_average().one,
                    "five": sysinfo::System::load_average().five,
                    "fifteen": sysinfo::System::load_average().fifteen,
                },
                "memory_used_mb": (sys.total_memory() - sys.available_memory()) / 1024 / 1024,
                "memory_total_mb": sys.total_memory() / 1024 / 1024,
                "hermes_discovered": h_found,
                "hermes_port": h_port,
            }))
        }

        "restart" => {
            let component = params
                .get("component")
                .and_then(|v| v.as_str())
                .unwrap_or("bridge");

            match component {
                "supervisor" | "all" => {
                    tokio::spawn(async {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        let _ = Command::new("systemctl")
                            .args(["--user", "restart", "ddb-supervisor"])
                            .spawn();
                    });
                    Ok(json!({ "ok": true, "restarting": "ddb-supervisor" }))
                }
                _ => Ok(json!({ "ok": true, "message": "Component active in monolith" })),
            }
        }

        // ── 2. System Command Execution ─────────────────────────────
        "exec" => {
            let cmd = params
                .get("command")
                .or_else(|| params.get("cmd"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing parameter: command"))?;

            #[cfg(target_os = "windows")]
            let mut command = {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(cmd);
                c
            };
            #[cfg(not(target_os = "windows"))]
            let mut command = {
                let mut c = Command::new("sh");
                c.arg("-c").arg(cmd);
                c
            };

            if let Some(cwd) = params.get("cwd").and_then(|v| v.as_str()) {
                command.current_dir(cwd);
            }

            let output = command.output()?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Ok(json!({
                "exit_code": output.status.code().unwrap_or(-1),
                "stdout": stdout,
                "stderr": stderr,
            }))
        }

        // ── 3. Screen & Display Control ─────────────────────────────
        "screen" => {
            #[cfg(target_os = "windows")]
            {
                let ps_script = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$screen = [System.Windows.Forms.Screen]::PrimaryScreen
$bounds = $screen.Bounds
$bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
$ms = New-Object System.IO.MemoryStream
$bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()
[Convert]::ToBase64String($ms.ToArray())
"#;
                let output = Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", ps_script])
                    .output()?;
                if !output.status.success() {
                    let err_msg = String::from_utf8_lossy(&output.stderr);
                    return Err(anyhow::anyhow!("Windows PowerShell screenshot failed: {}", err_msg.trim()));
                }
                let b64 = String::from_utf8_lossy(&output.stdout).replace("\r", "").replace("\n", "").trim().to_string();
                Ok(json!({ "image_base64": b64, "format": "png" }))
            }

            #[cfg(not(target_os = "windows"))]
            {
                let mut cmd = Command::new("grim");
                if std::env::var("WAYLAND_DISPLAY").is_err() {
                    // Детектим сокет wayland в /run/user/<uid> или фоллбекаем на wayland-1
                    let wayland_display = if std::path::Path::new("/run/user/1000/wayland-1").exists() {
                        "wayland-1"
                    } else if std::path::Path::new("/run/user/1000/wayland-0").exists() {
                        "wayland-0"
                    } else {
                        "wayland-1"
                    };
                    cmd.env("WAYLAND_DISPLAY", wayland_display);
                }
                if std::env::var("XDG_RUNTIME_DIR").is_err() {
                    let uid = dirs::data_dir()
                        .and_then(|p| p.to_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    let uid_str = if uid.contains("1000") { "1000" } else { "1000" };
                    cmd.env("XDG_RUNTIME_DIR", format!("/run/user/{}", uid_str));
                }

                let output = cmd.arg("-").output()?;
                if !output.status.success() {
                    let err_msg = String::from_utf8_lossy(&output.stderr);
                    return Err(anyhow::anyhow!("grim screenshot failed: {}", err_msg.trim()));
                }
                let b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &output.stdout,
                );
                Ok(json!({ "image_base64": b64, "format": "png" }))
            }
        }

        "display_control" => {
            // Поддержка очереди действий (Actions Batch Queue)
            if let Some(actions_list) = params.get("actions").and_then(|v| v.as_array()) {
                let default_delay = params.get("default_delay_ms").and_then(|v| v.as_u64()).unwrap_or(50);
                let mut executed = Vec::new();

                for item in actions_list {
                    let sub_action = item.get("action").and_then(|v| v.as_str()).unwrap_or("click");
                    let delay = item.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(default_delay);

                    match sub_action {
                        "key" => {
                            if let Some(key) = item.get("key").and_then(|v| v.as_str()) {
                                #[cfg(target_os = "windows")]
                                {
                                    let ps_cmd = format!("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{}')", key);
                                    let _ = Command::new("powershell")
                                        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                                        .status();
                                }
                                #[cfg(not(target_os = "windows"))]
                                {
                                    if key.to_lowercase() == "super" || key.starts_with("niri:") {
                                        let niri_cmd = key.trim_start_matches("niri:");
                                        let _ = Command::new("niri").args(["msg", "action", niri_cmd]).status();
                                    } else {
                                        let _ = Command::new("ydotool").args(["key", key]).status();
                                    }
                                }
                                executed.push(json!({ "action": "key", "key": key }));
                            }
                        }
                        "click" => {
                            let x = item.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
                            let y = item.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
                            #[cfg(target_os = "windows")]
                            {
                                let ps_cmd = format!(r#"Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(int flags, int dx, int dy, int cButtons, int extraInfo); [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);' -Name Mouse -Namespace Win32; [Win32.Mouse]::SetCursorPos({}, {}); [Win32.Mouse]::mouse_event(0x02, 0, 0, 0, 0); Start-Sleep -Milliseconds 40; [Win32.Mouse]::mouse_event(0x04, 0, 0, 0, 0)"#, x, y);
                                let _ = Command::new("powershell")
                                    .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                                    .status();
                            }
                            #[cfg(not(target_os = "windows"))]
                            {
                                let _ = Command::new("ydotool")
                                    .args(["mousemove", "--absolute", &x.to_string(), &y.to_string()])
                                    .status();
                                let _ = Command::new("ydotool").args(["click", "0xC0"]).status();
                            }
                            executed.push(json!({ "action": "click", "x": x, "y": y }));
                        }
                        "type" => {
                            let text = item.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                            #[cfg(target_os = "windows")]
                            {
                                let escaped = text.replace("'", "''");
                                let ps_cmd = format!("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{}')", escaped);
                                let _ = Command::new("powershell")
                                    .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                                    .status();
                            }
                            #[cfg(not(target_os = "windows"))]
                            {
                                let _ = Command::new("ydotool").args(["type", text]).status();
                            }
                            executed.push(json!({ "action": "type", "text": text }));
                        }
                        _ => {}
                    }

                    if delay > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(delay));
                    }
                }
                return Ok(json!({ "handled": true, "batch": true, "executed_count": executed.len(), "items": executed }));
            }

            let sub_action = params
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("click");

            match sub_action {
                "key" => {
                    let key = params
                        .get("key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("Missing key"))?;

                    #[cfg(target_os = "windows")]
                    {
                        let ps_cmd = format!("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{}')", key);
                        let _ = Command::new("powershell")
                            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                            .spawn();
                        Ok(json!({ "handled": true, "method": "windows_sendkeys", "key": key }))
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        if key.to_lowercase() == "super" || key.starts_with("niri:") {
                            let niri_cmd = key.trim_start_matches("niri:");
                            let _ = Command::new("niri").args(["msg", "action", niri_cmd]).spawn();
                            Ok(json!({ "handled": true, "method": "niri_msg", "key": key }))
                        } else {
                            let _ = Command::new("ydotool").args(["key", key]).spawn();
                            Ok(json!({ "handled": true, "method": "ydotool", "key": key }))
                        }
                    }
                }
                "click" => {
                    let x = params.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let btn = params.get("button").and_then(|v| v.as_str()).unwrap_or("left");
                    let duration = params.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(200);

                    #[cfg(target_os = "windows")]
                    {
                        let (down_flag, up_flag) = match btn {
                            "right" => (0x08, 0x10),
                            "middle" => (0x20, 0x40),
                            _ => (0x02, 0x04),
                        };
                        let ps_cmd = format!(r#"Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(int flags, int dx, int dy, int cButtons, int extraInfo); [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);' -Name Mouse -Namespace Win32; [Win32.Mouse]::SetCursorPos({}, {}); [Win32.Mouse]::mouse_event({}, 0, 0, 0, 0); Start-Sleep -Milliseconds 40; [Win32.Mouse]::mouse_event({}, 0, 0, 0, 0)"#, x as i64, y as i64, down_flag, up_flag);
                        let _ = Command::new("powershell")
                            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                            .spawn();
                        Ok(json!({ "handled": true, "method": "windows_user32_click", "x": x, "y": y, "button": btn }))
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        if let Some(ptr) = pointer {
                            // Smooth human-like Bezier move + Click!
                            ptr.move_to(x, y, duration)?;
                            ptr.click(btn)?;
                            Ok(json!({ "handled": true, "method": "uinput_bezier", "x": x, "y": y, "button": btn }))
                        } else {
                            // Fallback ydotool
                            let _ = Command::new("ydotool")
                                .args(["mousemove", "--absolute", &x.to_string(), &y.to_string()])
                                .spawn();
                            let _ = Command::new("ydotool").args(["click", "0xC0"]).spawn();
                            Ok(json!({ "handled": true, "method": "ydotool_fallback", "x": x, "y": y }))
                        }
                    }
                }
                "move" => {
                    let x = params.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let duration = params.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(250);

                    #[cfg(target_os = "windows")]
                    {
                        let ps_cmd = format!(r#"Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);' -Name Mouse -Namespace Win32; [Win32.Mouse]::SetCursorPos({}, {});"#, x as i64, y as i64);
                        let _ = Command::new("powershell")
                            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                            .spawn();
                        Ok(json!({ "handled": true, "method": "windows_user32_move", "x": x, "y": y }))
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        if let Some(ptr) = pointer {
                            ptr.move_to(x, y, duration)?;
                            Ok(json!({ "handled": true, "method": "uinput_bezier_move", "x": x, "y": y }))
                        } else {
                            let _ = Command::new("ydotool")
                                .args(["mousemove", "--absolute", &x.to_string(), &y.to_string()])
                                .spawn();
                            Ok(json!({ "handled": true, "method": "ydotool_move", "x": x, "y": y }))
                        }
                    }
                }
                "type" => {
                    let text = params
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();

                    #[cfg(target_os = "windows")]
                    {
                        let escaped = text.replace("'", "''");
                        let ps_cmd = format!("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{}')", escaped);
                        let _ = Command::new("powershell")
                            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                            .spawn();
                        Ok(json!({ "handled": true, "method": "windows_sendkeys", "text": text }))
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        let _ = Command::new("ydotool").args(["type", text]).spawn();
                        Ok(json!({ "handled": true, "action": "type", "text": text }))
                    }
                }
                _ => Ok(json!({ "handled": false, "error": "Unknown subaction" })),
            }
        }

        // ── 4. Sound & Audio Playback ───────────────────────────────
        "audio.play" => {
            let sample = params
                .get("sample")
                .or_else(|| params.get("sample_or_url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing sample or url"))?;

            #[cfg(target_os = "windows")]
            {
                let ps_cmd = format!(r#"Add-Type -AssemblyName System.Media; $p = New-Object System.Media.SoundPlayer('{}'); $p.PlaySync()"#, sample);
                let _ = Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                    .spawn();
            }
            #[cfg(not(target_os = "windows"))]
            {
                if sample.ends_with(".wav") || sample.ends_with(".ogg") {
                    let _ = Command::new("paplay").arg(sample).spawn();
                } else {
                    let _ = Command::new("ffplay")
                        .args(["-nodisp", "-autoexit", sample])
                        .spawn();
                }
            }

            Ok(json!({ "playing": true, "sample": sample }))
        }

        "sound_steal" => {
            let wakeword = params.get("wakeword").and_then(|v| v.as_bool()).unwrap_or(true);
            let mouth = params.get("mouth").and_then(|v| v.as_bool()).unwrap_or(false);
            let get_events = params.get("get_events").and_then(|v| v.as_bool()).unwrap_or(true);

            Ok(json!({
                "ok": true,
                "status": "configured",
                "wakeword": wakeword,
                "mouth": mouth,
                "get_events": get_events
            }))
        }

        // ── 5. File System (fs.*) ───────────────────────────────────
        "fs.read" => {
            let path = params
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing parameter: path"))?;
            let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize;

            let content = std::fs::read_to_string(path)?;
            let lines: Vec<&str> = content.lines().collect();
            let total_lines = lines.len();

            let start = if offset > 0 { offset - 1 } else { 0 };
            let paged: Vec<String> = lines
                .into_iter()
                .skip(start)
                .take(limit)
                .map(|s| s.to_string())
                .collect();

            Ok(json!({
                "content": paged.join("\n"),
                "total_lines": total_lines,
                "offset": offset,
                "limit": limit
            }))
        }

        "fs.write" => {
            let path = params
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing parameter: path"))?;
            let content = params
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing parameter: content"))?;
            std::fs::write(path, content)?;
            Ok(json!({ "written": true, "path": path }))
        }

        "fs.list" => {
            let path = params
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(path)? {
                if let Ok(e) = entry {
                    let file_name = e.file_name().to_string_lossy().to_string();
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    entries.push(json!({
                        "name": file_name,
                        "is_dir": is_dir,
                        "size": size
                    }));
                }
            }
            Ok(json!({ "entries": entries }))
        }

        // ── 6. Hermes Desktop Sibling Synapse ───────────────────────
        #[cfg(feature = "hermes")]
        "hermes.sessions" => {
            let h = discover_hermes();
            if !h.found || h.port.is_none() {
                return Err(anyhow::anyhow!("Hermes not discovered on host"));
            }
            let port = h.port.unwrap();
            let client = reqwest::Client::new();
            let mut req = client.get(format!("http://127.0.0.1:{}/api/sessions?limit=50", port));
            if let Some(tok) = &h.token {
                req = req.header("X-Hermes-Session-Token", tok);
            }
            let resp = req.send().await?.json::<serde_json::Value>().await?;
            Ok(resp)
        }

        #[cfg(feature = "hermes")]
        "hermes.messages" => {
            let h = discover_hermes();
            if !h.found || h.port.is_none() {
                return Err(anyhow::anyhow!("Hermes not discovered on host"));
            }
            let port = h.port.unwrap();
            let client = reqwest::Client::new();

            // Если session_id не передан — берем активную или последнюю сессию
            let mut sid = params.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string());
            if sid.is_none() {
                let mut req = client.get(format!("http://127.0.0.1:{}/api/sessions?limit=10", port));
                if let Some(tok) = &h.token {
                    req = req.header("X-Hermes-Session-Token", tok);
                }
                if let Ok(resp) = req.send().await {
                    if let Ok(v) = resp.json::<serde_json::Value>().await {
                        if let Some(sessions) = v.get("sessions").and_then(|s| s.as_array()) {
                            if let Some(first) = sessions
                                .iter()
                                .find(|s| s.get("is_active") == Some(&json!(true)))
                                .or_else(|| sessions.first())
                            {
                                if let Some(id) = first.get("id").and_then(|i| i.as_str()) {
                                    sid = Some(id.to_string());
                                }
                            }
                        }
                    }
                }
            }

            let session_id = sid.ok_or_else(|| anyhow::anyhow!("No active Hermes session found"))?;
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);
            let mut req = client.get(format!("http://127.0.0.1:{}/api/sessions/{}/messages?limit={}", port, session_id, limit));
            if let Some(tok) = &h.token {
                req = req.header("X-Hermes-Session-Token", tok);
            }
            let resp = req.send().await?.json::<serde_json::Value>().await?;
            Ok(resp)
        }

        #[cfg(feature = "hermes")]
        "hermes.send" => {
            let prompt = params
                .get("prompt")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing parameter: prompt"))?;

            let session_id_arg = params.get("session_id").and_then(|v| v.as_str());

            send_to_hermes(prompt, session_id_arg).await
        }

        _ => Err(anyhow::anyhow!("Action '{}' is not implemented", action)),
    }
}

#[cfg(feature = "hermes")]
async fn send_to_hermes(prompt: &str, target_session: Option<&str>) -> Result<serde_json::Value> {
    let h = discover_hermes();
    if !h.found || h.port.is_none() {
        return Err(anyhow::anyhow!("Hermes not discovered on host"));
    }
    let port = h.port.unwrap();

    let mut resolved_sid = target_session.map(|s| s.to_string());
    if resolved_sid.is_none() {
        let client = reqwest::Client::new();
        let mut req = client.get(format!("http://127.0.0.1:{}/api/sessions?limit=10", port));
        if let Some(tok) = &h.token {
            req = req.header("X-Hermes-Session-Token", tok);
        }
        if let Ok(resp) = req.send().await {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                if let Some(sessions) = v.get("sessions").and_then(|s| s.as_array()) {
                    if let Some(first) = sessions
                        .iter()
                        .find(|s| s.get("is_active") == Some(&json!(true)))
                        .or_else(|| sessions.first())
                    {
                        if let Some(id) = first.get("id").and_then(|i| i.as_str()) {
                            resolved_sid = Some(id.to_string());
                        }
                    }
                }
            }
        }
    }

    let ws_url = match &h.token {
        Some(t) => format!("ws://127.0.0.1:{}/api/ws?token={}", port, urlencoding(t)),
        None => format!("ws://127.0.0.1:{}/api/ws", port),
    };

    let (mut ws_stream, _) = connect_async(&ws_url).await?;

    let mut live_sid = resolved_sid.clone();

    // 1. If stored session specified, send session.resume and WAIT for response to get in-memory runtime session_id
    if let Some(sid) = &resolved_sid {
        let resume_rpc = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session.resume",
            "params": {
                "session_id": sid,
                "source": "desktop",
                "omit_messages": true
            }
        });
        ws_stream.send(Message::Text(resume_rpc.to_string().into())).await?;

        // Wait for session.resume result
        let resume_timeout = tokio::time::sleep(Duration::from_millis(3000));
        tokio::pin!(resume_timeout);

        loop {
            tokio::select! {
                _ = &mut resume_timeout => {
                    break;
                }
                msg = ws_stream.next() => {
                    if let Some(Ok(Message::Text(text))) = msg {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                            if parsed.get("id") == Some(&json!(1)) {
                                if let Some(res_sid) = parsed.get("result").and_then(|r| r.get("session_id")).and_then(|s| s.as_str()) {
                                    live_sid = Some(res_sid.to_string());
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Submit prompt with resolved live_sid
    let submit_rpc = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "prompt.submit",
        "params": {
            "session_id": live_sid,
            "text": format!("[Dirom Reqwest]\n{}", prompt)
        }
    });

    ws_stream.send(Message::Text(submit_rpc.to_string().into())).await?;

    // Wait up to 5s for prompt.submit acknowledgement
    let timeout_fut = tokio::time::sleep(Duration::from_millis(5000));
    tokio::pin!(timeout_fut);

    loop {
        tokio::select! {
            _ = &mut timeout_fut => {
                break;
            }
            msg = ws_stream.next() => {
                if let Some(Ok(Message::Text(text))) = msg {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                        if parsed.get("id") == Some(&json!(2)) {
                            if let Some(err) = parsed.get("error") {
                                return Err(anyhow::anyhow!("Hermes prompt.submit error: {}", err));
                            }
                            let status = parsed.get("result")
                                .and_then(|r| r.get("status"))
                                .and_then(|s| s.as_str())
                                .unwrap_or("submitted");
                            return Ok(json!({
                                "ok": true,
                                "status": status,
                                "session_id": live_sid,
                                "stored_session_id": resolved_sid,
                            }));
                        }
                    }
                }
            }
        }
    }

    Ok(json!({
        "ok": true,
        "session_id": live_sid,
        "stored_session_id": resolved_sid,
        "status": "queued_to_hermes"
    }))
}

#[allow(dead_code)]
fn urlencoding(s: &str) -> String {
    let mut encoded = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{:02X}", b));
        }
    }
    encoded
}
