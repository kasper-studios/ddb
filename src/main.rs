mod actions;
mod cloud;
mod config;
#[cfg(feature = "hermes")]
mod hermes;
mod pointer;
mod server;
mod tui;
mod tunnel;

use clap::Parser;
use config::load_config;
use pointer::DiromPointer;
use server::{create_router, AppState};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[derive(Parser, Debug)]
#[command(author, version, about = "Dirom Device Bridge (DDB) — Native Host Agent")]
struct Cli {
    /// Run as background daemon (no interactive TUI)
    #[arg(long)]
    daemon: bool,

    /// Subcommand: install systemd service
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Install ddb as systemd --user service
    Install,
    /// Run background daemon
    Run,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::Install) = cli.command {
        return install_systemd();
    }

    let is_daemon = cli.daemon || matches!(cli.command, Some(Commands::Run));

    // Shared state
    let config = Arc::new(RwLock::new(load_config()));
    let pointer = DiromPointer::new().ok().map(Arc::new);
    let app_state = AppState {
        config: Arc::clone(&config),
        pointer,
        uptime_start: Instant::now(),
    };

    // Shutdown cancellation token
    let cancel_token = CancellationToken::new();
    let server_token = cancel_token.clone();

    // Spawn Kasper Cloud API Client loop (Zero-Config Magic)
    let cloud_config = Arc::clone(&config);
    let cloud_pointer = app_state.pointer.clone();
    let cloud_token = cancel_token.clone();
    tokio::spawn(async move {
        cloud::run_cloud_loop(cloud_config, cloud_pointer, Instant::now(), cloud_token).await;
    });

    // Spawn Tunnel Manager (Cloudflare / Ngrok)
    let tunnel_config = Arc::clone(&config);
    let tunnel_token = cancel_token.clone();
    tokio::spawn(async move {
        tunnel::run_tunnel_manager(tunnel_config, tunnel_token).await;
    });

    // Spawn Axum HTTP server on 0.0.0.0:8790 with graceful shutdown (if not already running)
    let router = create_router(app_state);
    let addr = SocketAddr::from(([0, 0, 0, 0], 8790));

    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                let _ = axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        server_token.cancelled().await;
                    })
                    .await;
            }
            Err(e) => {
                // Если порт уже занят (например, ddb-supervisor уже крутится в systemd),
                // TUI-клиент не должен паниковать и ломать экран!
                if is_daemon {
                    eprintln!("Ошибка: порт {} уже занят: {}", addr, e);
                }
            }
        }
    });

    if is_daemon {
        println!("DDB Native Daemon running on {}", addr);
        tokio::signal::ctrl_c().await?;
        println!("Shutting down DDB Daemon gracefully...");
        cancel_token.cancel();
    } else {
        // Run interactive Ratatui TUI
        tui::run_tui()?;
        cancel_token.cancel();
    }

    Ok(())
}

fn install_systemd() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy();

    let service_content = format!(
        r#"[Unit]
Description=DDB Native Rust Daemon (Dirom Device Bridge)
After=network.target

[Service]
Type=simple
ExecStart={} --daemon
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
"#,
        exe_str
    );

    if let Some(mut dir) = dirs::config_dir() {
        dir.push("systemd");
        dir.push("user");
        std::fs::create_dir_all(&dir)?;
        dir.push("ddb-supervisor.service");
        std::fs::write(&dir, service_content)?;
        println!("✓ Служба записана в: {}", dir.display());

        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "ddb-supervisor"])
            .status();

        println!("✓ Служба ddb-supervisor успешно установлена и запущена в systemd --user!");
    } else {
        println!("Не удалось определить путь ~/.config/systemd/user");
    }

    Ok(())
}
