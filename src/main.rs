mod actions;
mod config;
#[cfg(feature = "hermes")]
mod hermes;
mod pointer;
mod server;
mod tui;

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

    // Spawn Axum HTTP server on 0.0.0.0:8790 with graceful shutdown
    let router = create_router(app_state);
    let addr = SocketAddr::from(([0, 0, 0, 0], 8790));

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                server_token.cancelled().await;
            })
            .await
            .unwrap();
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
