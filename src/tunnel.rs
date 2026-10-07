use crate::config::{save_config, DdbConfig};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub async fn run_tunnel_manager(
    config_lock: Arc<RwLock<DdbConfig>>,
    cancel_token: CancellationToken,
) {
    let mut current_child: Option<Child> = None;
    let mut last_type = String::new();
    let mut last_token = String::new();

    while !cancel_token.is_cancelled() {
        let (tunnel_type, tunnel_token, _tunnel_url) = {
            let cfg = config_lock.read().await;
            (cfg.tunnel_type.clone(), cfg.tunnel_token.clone(), cfg.tunnel_url.clone())
        };

        // Если режим или токен изменились — перезапускаем дочерний процесс
        if tunnel_type != last_type || tunnel_token != last_token {
            if let Some(mut child) = current_child.take() {
                let _ = child.kill().await;
                println!("[TUNNEL] Предыдущий туннельный процесс остановлен.");
            }

            last_type = tunnel_type.clone();
            last_token = tunnel_token.clone();

            match tunnel_type.as_str() {
                "cloudflare" => {
                    println!("[TUNNEL] Запуск Cloudflare Tunnel...");
                    let mut cmd = Command::new("cloudflared");

                    if !tunnel_token.trim().is_empty() {
                        cmd.args(["tunnel", "run", "--token", tunnel_token.trim()]);
                    } else {
                        // Quick Tunnel mode (автоматический https://*.trycloudflare.com)
                        cmd.args(["tunnel", "--url", "http://127.0.0.1:8790"]);
                    }

                    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

                    match cmd.spawn() {
                        Ok(mut child) => {
                            let stderr = child.stderr.take();
                            let cfg_ref = Arc::clone(&config_lock);

                            // Чтение логов cloudflared для автоопределения URL
                            if let Some(err_stream) = stderr {
                                tokio::spawn(async move {
                                    let mut reader = BufReader::new(err_stream).lines();
                                    while let Ok(Some(line)) = reader.next_line().await {
                                        if line.contains("trycloudflare.com") {
                                            for part in line.split_whitespace() {
                                                if part.starts_with("https://") && part.contains("trycloudflare.com") {
                                                    let url = part.trim().to_string();
                                                    println!("[TUNNEL] Cloudflare Tunnel URL обнаружен: {}", url);
                                                    let mut cfg = cfg_ref.write().await;
                                                    cfg.tunnel_url = url;
                                                    let _ = save_config(&cfg);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                });
                            }

                            current_child = Some(child);
                        }
                        Err(e) => {
                            eprintln!("[TUNNEL] Не удалось запустить cloudflared (установлен ли он?): {}", e);
                        }
                    }
                }

                "ngrok" => {
                    println!("[TUNNEL] Запуск Ngrok Tunnel...");
                    if !tunnel_token.trim().is_empty() {
                        let _ = Command::new("ngrok")
                            .args(["config", "add-authtoken", tunnel_token.trim()])
                            .status()
                            .await;
                    }

                    let mut cmd = Command::new("ngrok");
                    cmd.args(["http", "8790"]);
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());

                    match cmd.spawn() {
                        Ok(child) => {
                            current_child = Some(child);
                            let cfg_ref = Arc::clone(&config_lock);

                            // Опрос ngrok local API на :4040 для авто-извлечения public_url
                            tokio::spawn(async move {
                                for _ in 0..15 {
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                    if let Ok(resp) = reqwest::get("http://127.0.0.1:4040/api/tunnels").await {
                                        if let Ok(val) = resp.json::<serde_json::Value>().await {
                                            if let Some(tunnels) = val.get("tunnels").and_then(|v| v.as_array()) {
                                                for t in tunnels {
                                                    if let Some(p_url) = t.get("public_url").and_then(|v| v.as_str()) {
                                                        if p_url.starts_with("https://") {
                                                            println!("[TUNNEL] Ngrok URL обнаружен: {}", p_url);
                                                            let mut cfg = cfg_ref.write().await;
                                                            cfg.tunnel_url = p_url.to_string();
                                                            let _ = save_config(&cfg);
                                                            return;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("[TUNNEL] Не удалось запустить ngrok (установлен ли он?): {}", e);
                        }
                    }
                }

                _ => {
                    // "lan", "cloud", "custom" — сторонний процесс не требуется
                }
            }
        }

        // Проверяем живы ли запущенные туннели
        if let Some(ref mut child) = current_child {
            if let Ok(Some(status)) = child.try_wait() {
                println!("[TUNNEL] Туннельный процесс завершился со статусом: {}", status);
                current_child = None;
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            _ = cancel_token.cancelled() => {
                if let Some(mut child) = current_child.take() {
                    let _ = child.kill().await;
                }
                break;
            }
        }
    }
}
