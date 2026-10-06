use crate::config::DdbConfig;
use crate::hermes::discover_hermes;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::process::Command;
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub async fn execute_action(
    action: &str,
    params: serde_json::Value,
    config: &DdbConfig,
) -> Result<serde_json::Value> {
    match action {
        // ── 1. Telemetry & Host Diagnostics ─────────────────────────
        "status" => {
            let mut sys = sysinfo::System::new_all();
            sys.refresh_all();
            let h = discover_hermes();
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
            let h = discover_hermes();
            let uptime = sysinfo::System::uptime();

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
                "hermes_discovered": h.found,
                "hermes_port": h.port,
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

            let mut command = Command::new("sh");
            command.arg("-c").arg(cmd);

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
            let output = Command::new("grim").arg("-").output()?;
            if !output.status.success() {
                return Err(anyhow::anyhow!("grim screenshot failed"));
            }
            let b64 = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &output.stdout,
            );
            Ok(json!({ "image_base64": b64, "format": "png" }))
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
                                if key.to_lowercase() == "super" || key.starts_with("niri:") {
                                    let niri_cmd = key.trim_start_matches("niri:");
                                    let _ = Command::new("niri").args(["msg", "action", niri_cmd]).status();
                                } else {
                                    let _ = Command::new("ydotool").args(["key", key]).status();
                                }
                                executed.push(json!({ "action": "key", "key": key }));
                            }
                        }
                        "click" => {
                            let x = item.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
                            let y = item.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
                            let _ = Command::new("ydotool")
                                .args(["mousemove", "--absolute", &x.to_string(), &y.to_string()])
                                .status();
                            let _ = Command::new("ydotool").args(["click", "0xC0"]).status();
                            executed.push(json!({ "action": "click", "x": x, "y": y }));
                        }
                        "type" => {
                            let text = item.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                            let _ = Command::new("ydotool").args(["type", text]).status();
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

                    if key.to_lowercase() == "super" || key.starts_with("niri:") {
                        let niri_cmd = key.trim_start_matches("niri:");
                        let _ = Command::new("niri").args(["msg", "action", niri_cmd]).spawn();
                        Ok(json!({ "handled": true, "method": "niri_msg", "key": key }))
                    } else {
                        let _ = Command::new("ydotool").args(["key", key]).spawn();
                        Ok(json!({ "handled": true, "method": "ydotool", "key": key }))
                    }
                }
                "click" => {
                    let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
                    let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
                    let _ = Command::new("ydotool")
                        .args(["mousemove", "--absolute", &x.to_string(), &y.to_string()])
                        .spawn();
                    let _ = Command::new("ydotool").args(["click", "0xC0"]).spawn();
                    Ok(json!({ "handled": true, "action": "click", "x": x, "y": y }))
                }
                "type" => {
                    let text = params
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let _ = Command::new("ydotool").args(["type", text]).spawn();
                    Ok(json!({ "handled": true, "action": "type", "text": text }))
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

            if sample.ends_with(".wav") || sample.ends_with(".ogg") {
                let _ = Command::new("paplay").arg(sample).spawn();
            } else {
                let _ = Command::new("ffplay")
                    .args(["-nodisp", "-autoexit", sample])
                    .spawn();
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
