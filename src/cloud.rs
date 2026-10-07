use crate::actions::execute_action;
use crate::config::DdbConfig;
use crate::pointer::DiromPointer;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudStatus {
    pub state: String, // "disabled", "connecting", "connected", "error"
    pub url: String,
    pub last_error: Option<String>,
    pub last_connected_at: Option<u64>,
    pub messages_received: u64,
    pub messages_sent: u64,
}

static CLOUD_CONNECTED: AtomicBool = AtomicBool::new(false);
static GLOBAL_STATUS: std::sync::OnceLock<Arc<RwLock<CloudStatus>>> = std::sync::OnceLock::new();

pub fn get_cloud_status_handle() -> Arc<RwLock<CloudStatus>> {
    GLOBAL_STATUS
        .get_or_init(|| {
            Arc::new(RwLock::new(CloudStatus {
                state: "disabled".to_string(),
                url: String::new(),
                last_error: None,
                last_connected_at: None,
                messages_received: 0,
                messages_sent: 0,
            }))
        })
        .clone()
}

pub fn is_cloud_connected() -> bool {
    CLOUD_CONNECTED.load(Ordering::Relaxed)
}

pub async fn run_cloud_loop(
    config_lock: Arc<RwLock<DdbConfig>>,
    pointer: Option<Arc<DiromPointer>>,
    uptime_start: Instant,
    cancel_token: CancellationToken,
) {
    let status_handle = get_cloud_status_handle();

    while !cancel_token.is_cancelled() {
        let (tunnel_type, cloud_url, api_key, host_name, permissions) = {
            let cfg = config_lock.read().await;
            (
                cfg.tunnel_type.clone(),
                cfg.cloud_url.clone(),
                cfg.api_key.clone(),
                cfg.host_name.clone(),
                cfg.permissions.clone(),
            )
        };

        if tunnel_type != "cloud" {
            CLOUD_CONNECTED.store(false, Ordering::Relaxed);
            {
                let mut st = status_handle.write().await;
                st.state = "disabled".to_string();
                st.url = cloud_url;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                _ = cancel_token.cancelled() => break,
            }
            continue;
        }

        // Подготовка WebSocket URL
        let mut target_url = cloud_url.trim().to_string();
        if target_url.is_empty() || target_url.contains("api.kasperstudios.xyz") {
            target_url = "wss://diromstream.kasperstudios.xyz/api/v1/bridge".to_string();
        }
        if target_url.starts_with("http://") {
            target_url = target_url.replacen("http://", "ws://", 1);
        } else if target_url.starts_with("https://") {
            target_url = target_url.replacen("https://", "wss://", 1);
        } else if !target_url.starts_with("ws://") && !target_url.starts_with("wss://") {
            target_url = format!("wss://{}", target_url);
        }

        let parsed_url = match Url::parse(&target_url) {
            Ok(mut u) => {
                u.query_pairs_mut()
                    .append_pair("token", &api_key)
                    .append_pair("device", &host_name);
                u
            }
            Err(e) => {
                let err_str = format!("Invalid cloud URL '{}': {}", target_url, e);
                {
                    let mut st = status_handle.write().await;
                    st.state = "error".to_string();
                    st.last_error = Some(err_str.clone());
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(4)) => {}
                    _ = cancel_token.cancelled() => break,
                }
                continue;
            }
        };

        {
            let mut st = status_handle.write().await;
            st.state = "connecting".to_string();
            st.url = parsed_url.to_string();
            st.last_error = None;
        }

        // Построение WebSocket Handshake Request
        let mut request = match parsed_url.as_str().into_client_request() {
            Ok(req) => req,
            Err(e) => {
                let err_str = format!("Handshake request error: {}", e);
                {
                    let mut st = status_handle.write().await;
                    st.state = "error".to_string();
                    st.last_error = Some(err_str);
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(4)) => {}
                    _ = cancel_token.cancelled() => break,
                }
                continue;
            }
        };

        if let Ok(val) = format!("Bearer {}", api_key).parse() {
            request.headers_mut().insert("Authorization", val);
        }
        if let Ok(val) = host_name.parse() {
            request.headers_mut().insert("X-DDB-Host", val);
        }
        let ua = format!(
            "DDB-Native-Agent/1.0.0 ({}; {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        if let Ok(val) = ua.parse() {
            request.headers_mut().insert("User-Agent", val);
        }

        // Попытка подключения
        match connect_async(request).await {
            Ok((ws_stream, _resp)) => {
                CLOUD_CONNECTED.store(true, Ordering::Relaxed);
                let now_ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                {
                    let mut st = status_handle.write().await;
                    st.state = "connected".to_string();
                    st.last_connected_at = Some(now_ts);
                    st.last_error = None;
                }

                let log_msg = format!("[CLOUD] ✓ Connected to {} successfully! Device registered.", target_url);
                crate::logger::log(crate::logger::LogLevel::Cloud, &log_msg);

                let (mut ws_sender, mut ws_receiver) = ws_stream.split();

                // ── 1. Рукопожатие (Handshake / Registration) ─────────────
                let handshake = json!({
                    "type": "handshake",
                    "version": "1.0.0",
                    "device": host_name,
                    "token": api_key,
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                    "capabilities": [
                        "exec",
                        "screen",
                        "display_control",
                        "status",
                        "diagnostics",
                        "restart",
                        "fs.read",
                        "fs.write",
                        "fs.list",
                        "hermes.sessions",
                        "hermes.send"
                    ],
                    "permissions": permissions
                });

                if let Err(e) = ws_sender
                    .send(Message::Text(handshake.to_string().into()))
                    .await
                {
                    crate::logger::log(crate::logger::LogLevel::Error, format!("[CLOUD] Handshake send error: {}", e));
                    CLOUD_CONNECTED.store(false, Ordering::Relaxed);
                    continue;
                }

                {
                    let mut st = status_handle.write().await;
                    st.messages_sent += 1;
                }

                // ── 2. Двусторонний RPC-мост и Heartbeat ─────────────────
                let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(15));
                heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                loop {
                    tokio::select! {
                        _ = cancel_token.cancelled() => {
                            let _ = ws_sender.close().await;
                            break;
                        }

                        // Периодический Heartbeat
                        _ = heartbeat_interval.tick() => {
                            let mut sys = sysinfo::System::new_all();
                            sys.refresh_all();
                            let uptime = uptime_start.elapsed().as_secs();
                            let hb_msg = json!({
                                "type": "heartbeat",
                                "device": host_name,
                                "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                                "status": {
                                    "uptime": uptime,
                                    "freemem": sys.available_memory(),
                                    "totalmem": sys.total_memory(),
                                    "cpus": sys.cpus().len(),
                                    "os": std::env::consts::OS,
                                    "arch": std::env::consts::ARCH,
                                }
                            });

                            if let Err(e) = ws_sender.send(Message::Text(hb_msg.to_string().into())).await {
                                crate::logger::log(crate::logger::LogLevel::Error, format!("[CLOUD] Heartbeat send failed: {}", e));
                                break;
                            }
                            {
                                let mut st = status_handle.write().await;
                                st.messages_sent += 1;
                            }
                        }

                        // Приём сообщений из сокета
                        msg_opt = ws_receiver.next() => {
                            let msg = match msg_opt {
                                Some(Ok(m)) => m,
                                Some(Err(e)) => {
                                    crate::logger::log(crate::logger::LogLevel::Error, format!("[CLOUD] WebSocket read error: {}", e));
                                    break;
                                }
                                None => {
                                    crate::logger::log(crate::logger::LogLevel::Warn, "[CLOUD] WebSocket closed by remote server");
                                    break;
                                }
                            };

                            match msg {
                                Message::Text(txt) => {
                                    {
                                        let mut st = status_handle.write().await;
                                        st.messages_received += 1;
                                    }

                                    let parsed: serde_json::Value = match serde_json::from_str(&txt) {
                                        Ok(v) => v,
                                        Err(_) => continue,
                                    };

                                    // Проверка пинга
                                    if parsed.get("type").and_then(|v| v.as_str()) == Some("ping") {
                                        let pong = json!({ "type": "pong" });
                                        let _ = ws_sender.send(Message::Text(pong.to_string().into())).await;
                                        continue;
                                    }

                                    // Обработка RPC-запроса
                                    let req_id = parsed.get("id").cloned();
                                    let action = parsed.get("action")
                                        .or_else(|| parsed.get("method"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());

                                    if let Some(action_name) = action {
                                        let params = parsed.get("params").cloned().unwrap_or_else(|| json!({}));

                                        let current_cfg = config_lock.read().await.clone();

                                        let response_payload = if !current_cfg.is_action_permitted(&action_name, &params) {
                                            json!({
                                                "id": req_id,
                                                "ok": false,
                                                "action": action_name,
                                                "error": "FORBIDDEN",
                                                "message": format!("Action '{}' is forbidden by permissions matrix on host", action_name)
                                            })
                                        } else {
                                            match execute_action(&action_name, params, &current_cfg, pointer.as_ref()).await {
                                                Ok(result) => json!({
                                                    "id": req_id,
                                                    "ok": true,
                                                    "action": action_name,
                                                    "result": result
                                                }),
                                                Err(e) => json!({
                                                    "id": req_id,
                                                    "ok": false,
                                                    "action": action_name,
                                                    "error": e.to_string()
                                                }),
                                            }
                                        };

                                        if let Err(e) = ws_sender.send(Message::Text(response_payload.to_string().into())).await {
                                            crate::logger::log(crate::logger::LogLevel::Error, format!("[CLOUD] Failed to send RPC response: {}", e));
                                            break;
                                        }

                                        {
                                            let mut st = status_handle.write().await;
                                            st.messages_sent += 1;
                                        }
                                    }
                                }
                                Message::Ping(p) => {
                                    let _ = ws_sender.send(Message::Pong(p)).await;
                                }
                                Message::Close(_) => {
                                    crate::logger::log(crate::logger::LogLevel::Warn, "[CLOUD] Server sent close frame");
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }

                CLOUD_CONNECTED.store(false, Ordering::Relaxed);
                {
                    let mut st = status_handle.write().await;
                    st.state = "error".to_string();
                    st.last_error = Some("Connection closed".to_string());
                }
            }
            Err(e) => {
                CLOUD_CONNECTED.store(false, Ordering::Relaxed);
                let err_str = format!("Connection failed: {}", e);
                crate::logger::log(crate::logger::LogLevel::Error, format!("[CLOUD] ⚠️ Failed to connect to {}: {}", target_url, e));
                {
                    let mut st = status_handle.write().await;
                    st.state = "error".to_string();
                    st.last_error = Some(err_str);
                }
            }
        }

        // Задержка перед повторным переподключением
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            _ = cancel_token.cancelled() => break,
        }
    }

    CLOUD_CONNECTED.store(false, Ordering::Relaxed);
    {
        let mut st = status_handle.write().await;
        st.state = "disabled".to_string();
    }
}
