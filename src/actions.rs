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

#[cfg(target_os = "windows")]
async fn run_win_blocking<F, T>(label: &str, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| anyhow::anyhow!("Windows {} task failed: {}", label, e))?
}

fn resolve_step_coords(
    item: &serde_json::Value,
    default_coord_space: &str,
    x_key: &str,
    y_key: &str,
) -> Result<(f64, f64)> {
    let raw_x = item.get(x_key).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let raw_y = item.get(y_key).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let space = item
        .get("coord_space")
        .and_then(|v| v.as_str())
        .unwrap_or(default_coord_space);
    let vbounds = crate::screen::current_virtual_bounds();
    let last = crate::screen::get_last_screen_transform();
    crate::screen::map_cua_coords(raw_x, raw_y, space, item, vbounds, last)
}

fn parse_scroll_deltas(item: &serde_json::Value) -> (i32, i32) {
    let mut dx = item
        .get("dx")
        .or_else(|| item.get("delta_x"))
        .or_else(|| item.get("scroll_x"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    let mut dy = item
        .get("dy")
        .or_else(|| item.get("delta_y"))
        .or_else(|| item.get("scroll_y"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    if dx == 0 && dy == 0 {
        if let Some(dir) = item.get("direction").and_then(|v| v.as_str()) {
            let amount = item.get("amount").and_then(|v| v.as_i64()).unwrap_or(3).max(1) as i32;
            match dir.trim().to_ascii_lowercase().as_str() {
                "up" => dy = amount,
                "down" => dy = -amount,
                "left" => dx = -amount,
                "right" => dx = amount,
                _ => {}
            }
        }
    }
    (dx, dy)
}

#[allow(unused_variables)]
async fn execute_cua_step(
    sub_action: &str,
    item: &serde_json::Value,
    default_coord_space: &str,
    pointer: Option<&Arc<DiromPointer>>,
) -> Result<serde_json::Value> {
    match sub_action {
        "click" | "double_click" | "triple_click" | "right_click" | "middle_click" => {
            let (x, y) = resolve_step_coords(item, default_coord_space, "x", "y")?;
            let btn = match sub_action {
                "right_click" => "right",
                "middle_click" => "middle",
                _ => item.get("button").and_then(|v| v.as_str()).unwrap_or("left"),
            };
            let clicks: u8 = match sub_action {
                "double_click" => 2,
                "triple_click" => 3,
                _ => item
                    .get("clicks")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1)
                    .clamp(1, 5) as u8,
            };
            let duration = item
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(120);

            #[cfg(target_os = "windows")]
            {
                let ix = x.round() as i32;
                let iy = y.round() as i32;
                let btn_owned = btn.to_string();
                run_win_blocking("click", move || {
                    crate::pointer::windows_click(ix, iy, &btn_owned, clicks)
                })
                .await?;
                Ok(json!({
                    "handled": true,
                    "action": sub_action,
                    "method": "windows_sendinput_click",
                    "x": ix,
                    "y": iy,
                    "button": btn,
                    "clicks": clicks,
                }))
            }
            #[cfg(not(target_os = "windows"))]
            {
                if let Some(ptr) = pointer {
                    let ptr_c = Arc::clone(ptr);
                    let btn_owned = btn.to_string();
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        ptr_c.move_to(x, y, duration)?;
                        ptr_c.click_count(&btn_owned, clicks)?;
                        Ok(())
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Pointer click task failed: {}", e))??;
                    Ok(json!({
                        "handled": true,
                        "action": sub_action,
                        "method": "uinput_bezier",
                        "x": x,
                        "y": y,
                        "button": btn,
                        "clicks": clicks,
                    }))
                } else {
                    let ix = x.round() as i32;
                    let iy = y.round() as i32;
                    let click_code = match btn {
                        "right" => "0xC1",
                        "middle" => "0xC2",
                        _ => "0xC0",
                    };
                    let _ = Command::new("ydotool")
                        .args(["mousemove", "--absolute", &ix.to_string(), &iy.to_string()])
                        .status();
                    for _ in 0..clicks {
                        let _ = Command::new("ydotool").args(["click", click_code]).status();
                    }
                    Ok(json!({
                        "handled": true,
                        "action": sub_action,
                        "method": "ydotool_fallback",
                        "x": ix,
                        "y": iy,
                        "button": btn,
                        "clicks": clicks,
                    }))
                }
            }
        }

        "move" => {
            let (x, y) = resolve_step_coords(item, default_coord_space, "x", "y")?;
            let duration = item
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(180);

            #[cfg(target_os = "windows")]
            {
                let ix = x.round() as i32;
                let iy = y.round() as i32;
                run_win_blocking("move", move || crate::pointer::windows_move_to(ix, iy)).await?;
                Ok(json!({
                    "handled": true,
                    "action": "move",
                    "method": "windows_sendinput_move",
                    "x": ix,
                    "y": iy,
                }))
            }
            #[cfg(not(target_os = "windows"))]
            {
                if let Some(ptr) = pointer {
                    let ptr_c = Arc::clone(ptr);
                    tokio::task::spawn_blocking(move || ptr_c.move_to(x, y, duration))
                        .await
                        .map_err(|e| anyhow::anyhow!("Pointer move task failed: {}", e))??;
                    Ok(json!({
                        "handled": true,
                        "action": "move",
                        "method": "uinput_bezier_move",
                        "x": x,
                        "y": y,
                    }))
                } else {
                    let ix = x.round() as i32;
                    let iy = y.round() as i32;
                    let _ = Command::new("ydotool")
                        .args(["mousemove", "--absolute", &ix.to_string(), &iy.to_string()])
                        .status();
                    Ok(json!({
                        "handled": true,
                        "action": "move",
                        "method": "ydotool_fallback",
                        "x": ix,
                        "y": iy,
                    }))
                }
            }
        }

        "mouse_down" | "mouse_up" => {
            let down = sub_action == "mouse_down";
            let has_coords = item.get("x").is_some() || item.get("y").is_some();
            let opt_coords = if has_coords {
                Some(resolve_step_coords(item, default_coord_space, "x", "y")?)
            } else {
                None
            };
            let btn = item.get("button").and_then(|v| v.as_str()).unwrap_or("left");

            #[cfg(target_os = "windows")]
            {
                let opt_x = opt_coords.map(|(x, _)| x.round() as i32);
                let opt_y = opt_coords.map(|(_, y)| y.round() as i32);
                let btn_owned = btn.to_string();
                run_win_blocking(sub_action, move || {
                    crate::pointer::windows_mouse_button(opt_x, opt_y, &btn_owned, down)
                })
                .await?;
                Ok(json!({
                    "handled": true,
                    "action": sub_action,
                    "method": "windows_sendinput_button",
                    "button": btn,
                    "x": opt_x,
                    "y": opt_y,
                }))
            }
            #[cfg(not(target_os = "windows"))]
            {
                if let Some(ptr) = pointer {
                    let ptr_c = Arc::clone(ptr);
                    let btn_owned = btn.to_string();
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        if let Some((x, y)) = opt_coords {
                            ptr_c.move_to(x, y, 80)?;
                        }
                        ptr_c.mouse_button(&btn_owned, down)?;
                        Ok(())
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Pointer button task failed: {}", e))??;
                }
                Ok(json!({
                    "handled": true,
                    "action": sub_action,
                    "button": btn,
                }))
            }
        }

        "drag" => {
            let from_x_key = if item.get("from_x").is_some() {
                "from_x"
            } else if item.get("start_x").is_some() {
                "start_x"
            } else {
                "x"
            };
            let from_y_key = if item.get("from_y").is_some() {
                "from_y"
            } else if item.get("start_y").is_some() {
                "start_y"
            } else {
                "y"
            };
            let to_x_key = if item.get("to_x").is_some() {
                "to_x"
            } else {
                "end_x"
            };
            let to_y_key = if item.get("to_y").is_some() {
                "to_y"
            } else {
                "end_y"
            };
            if item.get(to_x_key).is_none() || item.get(to_y_key).is_none() {
                return Err(anyhow::anyhow!(
                    "Missing drag destination parameters: to_x / to_y"
                ));
            }
            let (fx, fy) = resolve_step_coords(item, default_coord_space, from_x_key, from_y_key)?;
            let (tx, ty) = resolve_step_coords(item, default_coord_space, to_x_key, to_y_key)?;
            let btn = item.get("button").and_then(|v| v.as_str()).unwrap_or("left");
            let duration = item
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(280);

            #[cfg(target_os = "windows")]
            {
                let ifx = fx.round() as i32;
                let ify = fy.round() as i32;
                let itx = tx.round() as i32;
                let ity = ty.round() as i32;
                let btn_owned = btn.to_string();
                run_win_blocking("drag", move || {
                    crate::pointer::windows_drag(ifx, ify, itx, ity, &btn_owned, duration)
                })
                .await?;
                Ok(json!({
                    "handled": true,
                    "action": "drag",
                    "method": "windows_sendinput_drag",
                    "from_x": ifx,
                    "from_y": ify,
                    "to_x": itx,
                    "to_y": ity,
                    "button": btn,
                }))
            }
            #[cfg(not(target_os = "windows"))]
            {
                if let Some(ptr) = pointer {
                    let ptr_c = Arc::clone(ptr);
                    let btn_owned = btn.to_string();
                    tokio::task::spawn_blocking(move || {
                        ptr_c.drag(fx, fy, tx, ty, &btn_owned, duration)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Pointer drag task failed: {}", e))??;
                }
                Ok(json!({
                    "handled": true,
                    "action": "drag",
                    "method": "uinput_bezier_drag",
                    "from_x": fx,
                    "from_y": fy,
                    "to_x": tx,
                    "to_y": ty,
                    "button": btn,
                }))
            }
        }

        "scroll" => {
            let has_coords = item.get("x").is_some() || item.get("y").is_some();
            let opt_coords = if has_coords {
                Some(resolve_step_coords(item, default_coord_space, "x", "y")?)
            } else {
                None
            };
            let (dx, dy) = parse_scroll_deltas(item);

            #[cfg(target_os = "windows")]
            {
                let opt_x = opt_coords.map(|(x, _)| x.round() as i32);
                let opt_y = opt_coords.map(|(_, y)| y.round() as i32);
                run_win_blocking("scroll", move || {
                    crate::pointer::windows_scroll(opt_x, opt_y, dx, dy)
                })
                .await?;
                Ok(json!({
                    "handled": true,
                    "action": "scroll",
                    "method": "windows_sendinput_scroll",
                    "dx": dx,
                    "dy": dy,
                    "x": opt_x,
                    "y": opt_y,
                }))
            }
            #[cfg(not(target_os = "windows"))]
            {
                if let Some(ptr) = pointer {
                    let ptr_c = Arc::clone(ptr);
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        if let Some((x, y)) = opt_coords {
                            ptr_c.move_to(x, y, 80)?;
                        }
                        ptr_c.scroll(dx, dy)?;
                        Ok(())
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("Pointer scroll task failed: {}", e))??;
                }
                Ok(json!({
                    "handled": true,
                    "action": "scroll",
                    "method": "uinput_scroll",
                    "dx": dx,
                    "dy": dy,
                }))
            }
        }

        "key" => {
            let key = item
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing key"))?;
            let repeat = item
                .get("repeat")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .clamp(1, 50);

            #[cfg(target_os = "windows")]
            {
                let key_owned = key.to_string();
                run_win_blocking("key", move || -> Result<()> {
                    for _ in 0..repeat {
                        crate::keys::send_key(&key_owned)?;
                    }
                    Ok(())
                })
                .await?;
                Ok(json!({
                    "handled": true,
                    "action": "key",
                    "method": "windows_sendinput_key",
                    "key": key,
                    "repeat": repeat,
                }))
            }
            #[cfg(not(target_os = "windows"))]
            {
                let key_owned = key.to_string();
                tokio::task::spawn_blocking(move || {
                    for _ in 0..repeat {
                        if key_owned.to_lowercase() == "super" || key_owned.starts_with("niri:") {
                            let niri_cmd = key_owned.trim_start_matches("niri:");
                            let _ = Command::new("niri").args(["msg", "action", niri_cmd]).status();
                        } else {
                            let _ = Command::new("ydotool").args(["key", &key_owned]).status();
                        }
                    }
                })
                .await
                .map_err(|e| anyhow::anyhow!("Linux key task failed: {}", e))?;
                Ok(json!({
                    "handled": true,
                    "action": "key",
                    "method": if key.to_lowercase() == "super" || key.starts_with("niri:") { "niri_msg" } else { "ydotool" },
                    "key": key,
                    "repeat": repeat,
                }))
            }
        }

        "type" => {
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            #[cfg(target_os = "windows")]
            {
                let text_owned = text.to_string();
                run_win_blocking("type", move || crate::keys::send_text(&text_owned)).await?;
                Ok(json!({
                    "handled": true,
                    "action": "type",
                    "method": "windows_sendinput_type",
                    "text": text,
                }))
            }
            #[cfg(not(target_os = "windows"))]
            {
                let text_owned = text.to_string();
                tokio::task::spawn_blocking(move || {
                    let _ = Command::new("ydotool").args(["type", &text_owned]).status();
                })
                .await
                .map_err(|e| anyhow::anyhow!("Linux type task failed: {}", e))?;
                Ok(json!({
                    "handled": true,
                    "action": "type",
                    "text": text,
                }))
            }
        }

        "wait" | "sleep" => {
            let ms = item
                .get("ms")
                .or_else(|| item.get("duration_ms"))
                .and_then(|v| v.as_u64())
                .unwrap_or(250)
                .min(30_000);
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok(json!({
                "handled": true,
                "action": "wait",
                "ms": ms,
            }))
        }

        "screen" | "screenshot" => {
            let captured = crate::screen::capture_screen(item).await?;
            Ok(json!({
                "handled": true,
                "action": "screen",
                "screen": captured.to_json(),
            }))
        }

        _ => Ok(json!({ "handled": false, "error": format!("Unknown subaction '{}'", sub_action) })),
    }
}

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
            let (h_found, h_port, h_origin, h_state, h_version) = (
                h.found,
                h.port,
                h.api_origin.clone(),
                h.state.clone(),
                h.code_version.clone(),
            );
            #[cfg(not(feature = "hermes"))]
            let (h_found, h_port, h_origin, h_state, h_version) =
                (false, None::<u16>, None::<String>, None::<String>, None::<String>);

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
                "hermes_api_origin": h_origin,
                "hermes_state": h_state,
                "hermes_version": h_version,
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
            let captured = crate::screen::capture_screen(&params).await?;
            Ok(captured.to_json())
        }

        "display_control" => {
            let default_coord_space = params
                .get("coord_space")
                .and_then(|v| v.as_str())
                .unwrap_or("physical");

            let mut response = if let Some(actions_list) = params.get("actions").and_then(|v| v.as_array()) {
                let default_delay = params
                    .get("default_delay_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50);
                let mut executed = Vec::with_capacity(actions_list.len());

                for item in actions_list {
                    let sub_action = item
                        .get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or("click");
                    let delay = item
                        .get("delay_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(default_delay);

                    let item_res =
                        execute_cua_step(sub_action, item, default_coord_space, pointer).await?;
                    executed.push(item_res);

                    if delay > 0 && sub_action != "wait" && sub_action != "sleep" {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                }

                json!({
                    "handled": true,
                    "batch": true,
                    "executed_count": executed.len(),
                    "items": executed,
                })
            } else {
                let sub_action = params
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("click");
                execute_cua_step(sub_action, &params, default_coord_space, pointer).await?
            };

            if let Some(cap_cfg) = params.get("capture_after") {
                let should_capture = match cap_cfg {
                    serde_json::Value::Bool(b) => *b,
                    serde_json::Value::Object(_) => true,
                    _ => false,
                };
                if should_capture {
                    let delay_ms = cap_cfg
                        .get("delay_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(120)
                        .min(10_000);
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    let screen_params = if cap_cfg.is_object() {
                        cap_cfg.clone()
                    } else {
                        json!({})
                    };
                    let captured = crate::screen::capture_screen(&screen_params).await?;
                    if let Some(obj) = response.as_object_mut() {
                        obj.insert("screen".to_string(), captured.to_json());
                    }
                }
            }

            Ok(response)
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

        "fs.stat" => crate::files::fs_stat(&params).await,

        "fs.download" => crate::files::fs_download(&params).await,

        "fs.upload" => crate::files::fs_upload(&params).await,

        "fs.fetch" => crate::files::fs_fetch(&params).await,

        // ── 6. Hermes Desktop Sibling Synapse ───────────────────────
        #[cfg(feature = "hermes")]
        "hermes.sessions" => {
            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Home directory not found"))?;
            let db_path = home.join(".hermes/state.db");
            if !db_path.exists() {
                return Err(anyhow::anyhow!("Hermes state.db not found at {:?}", db_path));
            }

            let conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;

            let mut stmt = conn.prepare(
                "SELECT id, source, title, started_at, ended_at, message_count, cwd, model 
                 FROM sessions 
                 ORDER BY started_at DESC 
                 LIMIT ?1"
            )?;

            let rows = stmt.query_map([limit], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "source": row.get::<_, Option<String>>(1)?,
                    "title": row.get::<_, Option<String>>(2)?,
                    "started_at": row.get::<_, Option<f64>>(3)?,
                    "ended_at": row.get::<_, Option<f64>>(4)?,
                    "message_count": row.get::<_, Option<i64>>(5)?,
                    "cwd": row.get::<_, Option<String>>(6)?,
                    "model": row.get::<_, Option<String>>(7)?,
                }))
            })?;

            let mut sessions = Vec::new();
            for r in rows {
                sessions.push(r?);
            }

            Ok(json!({ "sessions": sessions, "total": sessions.len() }))
        }

        #[cfg(feature = "hermes")]
        "hermes.messages" => {
            let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Home directory not found"))?;
            let db_path = home.join(".hermes/state.db");
            if !db_path.exists() {
                return Err(anyhow::anyhow!("Hermes state.db not found at {:?}", db_path));
            }

            let conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;

            // Resolve target session_id
            let target_sid = match params.get("session_id").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => {
                    let mut stmt = conn.prepare("SELECT session_id FROM messages ORDER BY id DESC LIMIT 1")?;
                    let mut rows = stmt.query([])?;
                    if let Some(row) = rows.next()? {
                        row.get::<_, String>(0)?
                    } else {
                        return Err(anyhow::anyhow!("No active Hermes sessions found in state.db"));
                    }
                }
            };

            let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            let mut stmt = conn.prepare(
                "SELECT id, role, content, timestamp, tool_name 
                 FROM messages 
                 WHERE session_id = ?1 
                 ORDER BY id DESC 
                 LIMIT ?2"
            )?;

            let rows = stmt.query_map(rusqlite::params![target_sid, limit], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "role": row.get::<_, Option<String>>(1)?,
                    "content": row.get::<_, Option<String>>(2)?,
                    "timestamp": row.get::<_, Option<f64>>(3)?,
                    "tool_name": row.get::<_, Option<String>>(4)?,
                }))
            })?;

            let mut messages = Vec::new();
            for r in rows {
                messages.push(r?);
            }
            messages.reverse(); // Chronological order

            Ok(json!({ "session_id": target_sid, "messages": messages, "count": messages.len() }))
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
    // Hermes 0.23+: the gateway no longer accepts `?token=` auth. Every dial mints a one-use
    // session ticket over the private control socket and offers it as a subprotocol.
    let h = discover_hermes();
    if !h.found {
        return Err(anyhow::anyhow!("Hermes gateway not discovered on host (no control socket answer)"));
    }
    let api_origin = h
        .api_origin
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Hermes gateway has no api_origin"))?;
    let (profile_id, instance_id) = match (h.profile_id.clone(), h.instance_id.clone()) {
        (Some(p), Some(i)) => (p, i),
        _ => return Err(anyhow::anyhow!(
            "Hermes gateway identity incomplete (profile_id/instance_id missing): a ticket cannot be minted"
        )),
    };

    let mint = |profile: String, instance: String| async move {
        tokio::task::spawn_blocking(move || crate::hermes::mint_ticket(&profile, &instance))
            .await
            .ok()
            .flatten()
    };

    let mut resolved_sid = target_session.map(|s| s.to_string());
    let mut live_sid: Option<String> = None;

    // A stored session is resumed on the same socket that later carries the turn.
    let dial = |ticket: String, origin: String| async move {
        let (url, protocols) = crate::hermes::gateway_ws_target(&origin, &ticket);
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut req = url.into_client_request()?;
        req.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            protocols.parse().map_err(|_| anyhow::anyhow!("invalid subprotocol header"))?,
        );
        let (stream, _resp) = connect_async(req).await?;
        Ok::<_, anyhow::Error>(stream)
    };

    let ticket = mint(profile_id.clone(), instance_id.clone())
        .await
        .ok_or_else(|| anyhow::anyhow!("Hermes control socket refused to mint a session ticket"))?;
    let mut ws_stream = dial(ticket, api_origin.clone()).await?;

    // 1. No stored session named: pick the newest stored session over the socket.
    if resolved_sid.is_none() {
        let list_rpc = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "session.list",
            "params": { "scope": "stored", "limit": 10 }
        });
        ws_stream.send(Message::Text(list_rpc.to_string().into())).await?;
        let list_timeout = tokio::time::sleep(Duration::from_millis(5000));
        tokio::pin!(list_timeout);
        loop {
            tokio::select! {
                _ = &mut list_timeout => { break; }
                msg = ws_stream.next() => {
                    if let Some(Ok(Message::Text(text))) = msg {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                            if parsed.get("id") == Some(&json!(9)) {
                                if let Some(first) = parsed
                                    .get("result")
                                    .and_then(|r| r.get("sessions"))
                                    .and_then(|s| s.as_array())
                                    .and_then(|a| a.first())
                                {
                                    if let Some(id) = first.get("id").and_then(|i| i.as_str()) {
                                        resolved_sid = Some(id.to_string());
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. If stored session specified, send session.resume and WAIT for response to get in-memory runtime session_id
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

    // 3. Submit the turn. `submission_id` is mandatory (it is the admission's idempotency key);
    // `surface` is only accepted for 'hud'/'voice-live', so a plain device turn omits it entirely.
    let turn_sid = live_sid.clone().or_else(|| resolved_sid.clone());
    let submission_id = uuid_like();
    let submit_rpc = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "prompt.submit",
        "params": {
            "session_id": turn_sid,
            "text": format!("[Dirom Reqwest]\n{}", prompt),
            "submission_id": submission_id
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
                            let result = parsed.get("result").cloned().unwrap_or(json!({}));
                            return Ok(json!({
                                "ok": true,
                                "status": result.get("status").and_then(|s| s.as_str()).unwrap_or("submitted"),
                                "admission_id": result.get("admission_id"),
                                "sequence": result.get("sequence"),
                                "submission_id": submission_id,
                                "session_id": turn_sid,
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
        "session_id": turn_sid,
        "stored_session_id": resolved_sid,
        "submission_id": submission_id,
        "status": "queued_to_hermes_no_ack"
    }))
}

/// A 32-hex-char submission id: the admission's idempotency key (any opaque string works,
/// but it must be present — Hermes refuses `prompt.submit` without one).
#[cfg(feature = "hermes")]
fn uuid_like() -> String {
    let a = rand::random::<u64>();
    let b = rand::random::<u64>();
    format!("{:016x}{:016x}", a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scroll_directions_and_deltas() {
        assert_eq!(parse_scroll_deltas(&json!({ "direction": "up", "amount": 5 })), (0, 5));
        assert_eq!(parse_scroll_deltas(&json!({ "direction": "down", "amount": 4 })), (0, -4));
        assert_eq!(parse_scroll_deltas(&json!({ "direction": "left", "amount": 2 })), (-2, 0));
        assert_eq!(parse_scroll_deltas(&json!({ "dx": 3, "dy": -7 })), (3, -7));
    }

    #[tokio::test]
    async fn executes_batch_wait_and_fs_actions() {
        let cfg = DdbConfig::default();
        let batch_res = execute_action(
            "display_control",
            json!({
                "actions": [
                    { "action": "wait", "ms": 5 },
                    { "action": "sleep", "duration_ms": 5 }
                ],
                "default_delay_ms": 0
            }),
            &cfg,
            None,
        )
        .await
        .unwrap();
        assert_eq!(batch_res["handled"], true);
        assert_eq!(batch_res["batch"], true);
        assert_eq!(batch_res["executed_count"], 2);

        let drag_err = execute_action(
            "display_control",
            json!({ "action": "drag", "x": 10, "y": 10 }),
            &cfg,
            None,
        )
        .await
        .unwrap_err();
        assert!(drag_err.to_string().contains("to_x / to_y"));
    }
}
