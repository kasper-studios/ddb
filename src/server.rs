use crate::actions::execute_action;
use crate::config::DdbConfig;
#[cfg(feature = "hermes")]
use crate::hermes::discover_hermes;
use crate::pointer::DiromPointer;
use axum::{
    extract::Json,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<DdbConfig>>,
    pub pointer: Option<Arc<DiromPointer>>,
    pub uptime_start: std::time::Instant,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct ExecPayload {
    pub command: Option<String>,
    pub cmd: Option<String>,
    pub timeout: Option<u64>,
}

#[derive(Deserialize)]
pub struct ActionPayload {
    pub action: String,
    pub params: Option<serde_json::Value>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/ping", get(handle_ping))
        .route("/status", get(handle_status))
        .route("/exec", post(handle_exec))
        .route("/action", post(handle_action))
        .route("/logs", get(handle_logs))
        .with_state(state)
}

async fn handle_ping() -> impl IntoResponse {
    Json(json!({ "ok": true, "service": "ddb-rust", "version": "1.0.0" }))
}

async fn handle_status(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();

    let uptime = state.uptime_start.elapsed().as_secs();

    #[cfg(feature = "hermes")]
    let hermes_val = json!(discover_hermes());
    #[cfg(not(feature = "hermes"))]
    let hermes_val = json!({ "found": false, "note": "hermes feature disabled in this build" });

    let cfg = state.config.read().await;

    Json(json!({
        "ok": true,
        "service": "ddb",
        "version": "1.0.0",
        "uptime": uptime,
        "host": {
            "name": cfg.host_name,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "freemem": sys.available_memory(),
            "totalmem": sys.total_memory(),
            "cpus": sys.cpus().len(),
        },
        "hermes": hermes_val,
        "tunnel": {
            "type": cfg.tunnel_type,
            "url": cfg.tunnel_url,
        }
    }))
}

async fn handle_exec(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(payload): Json<ExecPayload>,
) -> impl IntoResponse {
    let cfg = state.config.read().await;
    if !cfg.permissions.exec {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "ok": false,
                "error": "FORBIDDEN",
                "message": "Exec permission is disabled in config"
            })),
        );
    }

    let cmd_str = payload
        .command
        .or(payload.cmd)
        .unwrap_or_default();

    if cmd_str.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "MISSING_COMMAND" })),
        );
    }

    #[cfg(target_os = "windows")]
    let cmd_result = std::process::Command::new("cmd")
        .args(["/C", &cmd_str])
        .output();

    #[cfg(not(target_os = "windows"))]
    let cmd_result = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .output();

    match cmd_result {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            (
                StatusCode::OK,
                Json(json!({
                    "ok": out.status.success(),
                    "exit_code": out.status.code().unwrap_or(-1),
                    "stdout": stdout,
                    "stderr": stderr
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e.to_string() })),
        ),
    }
}

async fn handle_action(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(payload): Json<ActionPayload>,
) -> impl IntoResponse {
    let cfg = state.config.read().await;
    let params = payload.params.unwrap_or_else(|| json!({}));

    if !cfg.is_action_permitted(&payload.action, &params) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "ok": false,
                "error": "FORBIDDEN",
                "message": format!("Action '{}' is forbidden by permissions matrix", payload.action)
            })),
        );
    }

    match execute_action(&payload.action, params, &cfg, state.pointer.as_ref()).await {
        Ok(res) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "action": payload.action,
                "result": res
            })),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "action": payload.action,
                "error": err.to_string()
            })),
        ),
    }
}

async fn handle_logs() -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "lines": [
            "DDB Rust Native Daemon v1.0.0 initialized",
            "Listening on 0.0.0.0:8790"
        ]
    }))
}
