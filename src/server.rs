use crate::actions::execute_action;
use crate::config::DdbConfig;
#[cfg(feature = "hermes")]
use crate::hermes::discover_hermes;
use crate::pointer::DiromPointer;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::TryStreamExt;
use serde::Deserialize;
use serde_json::json;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::RwLock;
use tokio_util::io::ReaderStream;

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

#[derive(Deserialize)]
pub struct DownloadQuery {
    pub path: String,
    pub offset: Option<u64>,
    pub length: Option<u64>,
    pub hash: Option<bool>,
}

#[derive(Deserialize)]
pub struct UploadQuery {
    pub path: Option<String>,
    pub create_dirs: Option<bool>,
    pub append: Option<bool>,
    pub offset: Option<u64>,
    pub sha256: Option<String>,
    pub expected_sha256: Option<String>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/ping", get(handle_ping))
        .route("/status", get(handle_status))
        .route("/exec", post(handle_exec))
        .route("/action", post(handle_action))
        .route("/fs/download", get(handle_fs_download))
        .route(
            "/fs/upload",
            post(handle_fs_upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/logs", get(handle_logs))
        .with_state(state)
}

async fn handle_ping() -> impl IntoResponse {
    Json(json!({ "ok": true, "service": "ddb-rust", "version": "1.0.0" }))
}

async fn handle_status(State(state): State<AppState>) -> impl IntoResponse {
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
    State(state): State<AppState>,
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

    let cmd_str = payload.command.or(payload.cmd).unwrap_or_default();

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
    State(state): State<AppState>,
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

async fn handle_fs_download(
    State(state): State<AppState>,
    Query(query): Query<DownloadQuery>,
) -> impl IntoResponse {
    {
        let cfg = state.config.read().await;
        if !cfg.permissions.fs.read {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "ok": false,
                    "error": "FORBIDDEN",
                    "message": "fs.read permission is disabled in config"
                })),
            )
                .into_response();
        }
    }

    let path = match crate::files::resolve_path(&query.path) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) => {
            let status = if e.kind() == std::io::ErrorKind::NotFound {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            return (status, Json(json!({ "ok": false, "error": e.to_string() }))).into_response();
        }
    };

    if !meta.is_file() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "Target path is not a regular file" })),
        )
            .into_response();
    }

    let total_size = meta.len();
    let offset = query.offset.unwrap_or(0);
    if offset > total_size {
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            Json(json!({ "ok": false, "error": "Requested offset exceeds file size" })),
        )
            .into_response();
    }

    let available = total_size - offset;
    let read_len = query.length.map(|l| l.min(available)).unwrap_or(available);

    let sha_header = if query.hash == Some(true) {
        crate::files::sha256_file_hex(&path).await.ok()
    } else {
        None
    };

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if offset > 0 {
        if let Err(e) = file.seek(SeekFrom::Start(offset)).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": e.to_string() })),
            )
                .into_response();
        }
    }

    let stream = ReaderStream::new(file.take(read_len));
    let body = Body::from_stream(stream);

    let safe_name: String = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_else(|| "download.bin".into())
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
        .collect();

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(v) = HeaderValue::from_str(&read_len.to_string()) {
        headers.insert(header::CONTENT_LENGTH, v);
    }
    if let Ok(v) = HeaderValue::from_str(&total_size.to_string()) {
        headers.insert("X-File-Size", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", safe_name)) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    if let Some(digest) = sha_header {
        if let Ok(v) = HeaderValue::from_str(&digest) {
            headers.insert("X-File-Sha256", v);
        }
    }

    let status = if query.offset.is_some() || query.length.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };

    (status, headers, body).into_response()
}

fn parse_bool_str(val: &str) -> Option<bool> {
    match val.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

async fn handle_fs_upload(
    State(state): State<AppState>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    req: Request,
) -> impl IntoResponse {
    {
        let cfg = state.config.read().await;
        if !cfg.permissions.fs.write {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "ok": false,
                    "error": "FORBIDDEN",
                    "message": "fs.write permission is disabled in config"
                })),
            );
        }
    }

    let is_multipart = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_ascii_lowercase().contains("multipart/form-data"))
        .unwrap_or(false);

    let mut target_path = query.path;
    let mut create_dirs = query.create_dirs.unwrap_or(true);
    let mut append = query.append.unwrap_or(false);
    let mut offset = query.offset;
    let mut expected_sha = query.expected_sha256.or(query.sha256);

    if is_multipart {
        let mut multipart = match Multipart::from_request(req, &state).await {
            Ok(mp) => mp,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "ok": false, "error": format!("Invalid multipart body: {}", e) })),
                );
            }
        };

        let mut staged_tmp: Option<(PathBuf, crate::files::StreamWriteOutcome)> = None;
        let mut direct_outcome: Option<crate::files::StreamWriteOutcome> = None;

        loop {
            let field = match multipart.next_field().await {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    if let Some((tmp, _)) = staged_tmp.take() {
                        let _ = tokio::fs::remove_file(tmp).await;
                    }
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "ok": false, "error": format!("Multipart read error: {}", e) })),
                    );
                }
            };

            let name = field.name().unwrap_or_default().to_string();
            match name.as_str() {
                "path" => {
                    if let Ok(txt) = field.text().await {
                        target_path = Some(txt);
                    }
                }
                "create_dirs" => {
                    if let Ok(txt) = field.text().await {
                        if let Some(b) = parse_bool_str(&txt) {
                            create_dirs = b;
                        }
                    }
                }
                "append" => {
                    if let Ok(txt) = field.text().await {
                        if let Some(b) = parse_bool_str(&txt) {
                            append = b;
                        }
                    }
                }
                "offset" => {
                    if let Ok(txt) = field.text().await {
                        offset = txt.trim().parse::<u64>().ok();
                    }
                }
                "sha256" | "expected_sha256" => {
                    if let Ok(txt) = field.text().await {
                        expected_sha = Some(txt);
                    }
                }
                "file" | "data" => {
                    if let Some(ref raw_p) = target_path {
                        let dest = match crate::files::resolve_path(raw_p) {
                            Ok(p) => p,
                            Err(e) => {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    Json(json!({ "ok": false, "error": e.to_string() })),
                                );
                            }
                        };
                        match crate::files::write_stream_to_path(
                            &dest,
                            field,
                            create_dirs,
                            append,
                            offset,
                            expected_sha.as_deref(),
                        )
                        .await
                        {
                            Ok(outcome) => direct_outcome = Some(outcome),
                            Err(e) => {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    Json(json!({ "ok": false, "error": e.to_string() })),
                                );
                            }
                        }
                    } else {
                        let stage_path = std::env::temp_dir()
                            .join(format!("ddb_mp_stage_{}.tmp", rand::random::<u64>()));
                        match crate::files::write_stream_to_path(
                            &stage_path,
                            field,
                            true,
                            false,
                            None,
                            None,
                        )
                        .await
                        {
                            Ok(outcome) => staged_tmp = Some((stage_path, outcome)),
                            Err(e) => {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    Json(json!({ "ok": false, "error": e.to_string() })),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(outcome) = direct_outcome {
            if let Err(e) = crate::files::verify_sha256(
                &outcome.sha256,
                expected_sha.as_deref(),
                "multipart upload",
            ) {
                if !append && offset.is_none() {
                    let _ = tokio::fs::remove_file(&outcome.path).await;
                }
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "ok": false, "error": e.to_string() })),
                );
            }
            return (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "path": outcome.path,
                    "bytes_written": outcome.bytes_written,
                    "total_size": outcome.total_size,
                    "sha256": outcome.sha256,
                })),
            );
        }

        if let Some((stage_path, stage_outcome)) = staged_tmp {
            if let Err(e) = crate::files::verify_sha256(
                &stage_outcome.sha256,
                expected_sha.as_deref(),
                "multipart upload",
            ) {
                let _ = tokio::fs::remove_file(&stage_path).await;
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "ok": false, "error": e.to_string() })),
                );
            }

            let raw_p = match target_path {
                Some(p) => p,
                None => {
                    let _ = tokio::fs::remove_file(&stage_path).await;
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "ok": false, "error": "Missing multipart/query parameter: path" })),
                    );
                }
            };
            let dest = match crate::files::resolve_path(&raw_p) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tokio::fs::remove_file(&stage_path).await;
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "ok": false, "error": e.to_string() })),
                    );
                }
            };

            let move_res: anyhow::Result<crate::files::StreamWriteOutcome> = async {
                let staged_file = tokio::fs::File::open(&stage_path).await?;
                let stream = ReaderStream::new(staged_file).map_err(|e| e.to_string());
                crate::files::write_stream_to_path(&dest, stream, create_dirs, append, offset, None)
                    .await
            }
            .await;
            let _ = tokio::fs::remove_file(&stage_path).await;

            return match move_res {
                Ok(outcome) => (
                    StatusCode::OK,
                    Json(json!({
                        "ok": true,
                        "path": outcome.path,
                        "bytes_written": outcome.bytes_written,
                        "total_size": outcome.total_size,
                        "sha256": outcome.sha256,
                    })),
                ),
                Err(e) => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "ok": false, "error": e.to_string() })),
                ),
            };
        }

        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "Missing multipart 'file' field" })),
        );
    }

    let raw_p = match target_path {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": "Missing query parameter: path" })),
            );
        }
    };
    let dest = match crate::files::resolve_path(&raw_p) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e.to_string() })),
            );
        }
    };

    match crate::files::write_stream_to_path(
        &dest,
        req.into_body().into_data_stream(),
        create_dirs,
        append,
        offset,
        expected_sha.as_deref(),
    )
    .await
    {
        Ok(outcome) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "path": outcome.path,
                "bytes_written": outcome.bytes_written,
                "total_size": outcome.total_size,
                "sha256": outcome.sha256,
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": e.to_string() })),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn http_upload_and_download_streaming_roundtrip() {
        let mut cfg = DdbConfig::default();
        cfg.permissions.fs.read = true;
        cfg.permissions.fs.write = true;
        let state = AppState {
            config: Arc::new(RwLock::new(cfg)),
            pointer: None,
            uptime_start: Instant::now(),
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, create_router(state)).await;
        });

        let dir = std::env::temp_dir().join(format!("ddb_http_fs_{}", rand::random::<u64>()));
        let dest = dir.join("sub").join("stream.bin");
        let payload: Vec<u8> = (0..65537).map(|i| (i % 253) as u8).collect();
        let expected_sha = crate::files::sha256_hex(&payload);

        let client = reqwest::Client::new();
        let upload_url = format!("http://{}/fs/upload", addr);
        let resp = client
            .post(&upload_url)
            .query(&[
                ("path", dest.to_string_lossy().as_ref()),
                ("create_dirs", "true"),
                ("expected_sha256", expected_sha.as_str()),
            ])
            .body(payload.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let up_json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(up_json["ok"], true);
        assert_eq!(up_json["bytes_written"], payload.len() as u64);
        assert_eq!(up_json["sha256"], expected_sha);

        // Full download with X-File-Sha256 header
        let download_url = format!("http://{}/fs/download", addr);
        let dl_resp = client
            .get(&download_url)
            .query(&[("path", dest.to_string_lossy().as_ref()), ("hash", "true")])
            .send()
            .await
            .unwrap();
        assert_eq!(dl_resp.status(), reqwest::StatusCode::OK);
        assert_eq!(
            dl_resp.headers().get("X-File-Sha256").unwrap().to_str().unwrap(),
            expected_sha
        );
        let downloaded = dl_resp.bytes().await.unwrap();
        assert_eq!(downloaded.as_ref(), payload.as_slice());

        // Partial range download
        let range_resp = client
            .get(&download_url)
            .query(&[
                ("path", dest.to_string_lossy().as_ref()),
                ("offset", "100"),
                ("length", "512"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(range_resp.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        let partial = range_resp.bytes().await.unwrap();
        assert_eq!(partial.as_ref(), &payload[100..612]);

        server_task.abort();
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
