use anyhow::Result;
use base64::Engine;
use futures_util::Stream;
use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub const DEFAULT_CHUNK_SIZE: usize = 512 * 1024;
pub const MAX_CHUNK_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct StreamWriteOutcome {
    pub path: String,
    pub bytes_written: u64,
    pub total_size: u64,
    pub sha256: String,
}

/// Expands a leading `~/` or `~\` to the user's home directory.
pub fn resolve_path(raw: &str) -> Result<PathBuf> {
    if raw.is_empty() {
        return Err(anyhow::anyhow!("Missing or empty path"));
    }
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return Ok(home);
        }
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return Ok(home.join(rest));
        }
    }
    Ok(PathBuf::from(raw))
}

pub async fn ensure_parent_dirs(path: &Path, create_dirs: bool) -> Result<()> {
    if create_dirs {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    bytes_to_hex(&digest)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

pub async fn sha256_file_hex(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(bytes_to_hex(&hasher.finalize()))
}

pub fn verify_sha256(actual: &str, expected: Option<&str>, context: &str) -> Result<()> {
    if let Some(exp) = expected.map(str::trim).filter(|s| !s.is_empty()) {
        if !actual.eq_ignore_ascii_case(exp) {
            return Err(anyhow::anyhow!(
                "SHA-256 mismatch ({}): expected {}, got {}",
                context,
                exp,
                actual
            ));
        }
    }
    Ok(())
}

fn temp_sibling_path(dest: &Path, tag: &str) -> PathBuf {
    let file_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let nonce: u64 = rand::random();
    let tmp_name = format!(".{}.{}.{}.tmp", file_name, tag, nonce);
    match dest.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(tmp_name),
        _ => PathBuf::from(tmp_name),
    }
}

/// Streams byte chunks into `dest` either atomically (overwrite mode) or at `offset` / `append`.
pub async fn write_stream_to_path<S, B, E>(
    dest: &Path,
    mut stream: S,
    create_dirs: bool,
    append: bool,
    offset: Option<u64>,
    expected_sha256: Option<&str>,
) -> Result<StreamWriteOutcome>
where
    S: Stream<Item = std::result::Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    ensure_parent_dirs(dest, create_dirs).await?;

    let is_atomic_overwrite = !append && offset.is_none();
    let mut hasher = Sha256::new();
    let mut bytes_written: u64 = 0;

    if is_atomic_overwrite {
        let tmp_path = temp_sibling_path(dest, "ddb_upload");
        let write_res: Result<()> = async {
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res.map_err(|e| anyhow::anyhow!("Stream read error: {}", e))?;
                let slice = chunk.as_ref();
                if !slice.is_empty() {
                    file.write_all(slice).await?;
                    hasher.update(slice);
                    bytes_written += slice.len() as u64;
                }
            }
            file.flush().await?;
            Ok(())
        }
        .await;

        if let Err(e) = write_res {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(e);
        }

        let stream_sha = bytes_to_hex(&hasher.finalize());
        if let Err(e) = verify_sha256(&stream_sha, expected_sha256, "upload stream") {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(e);
        }

        if let Err(rename_err) = tokio::fs::rename(&tmp_path, dest).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(anyhow::anyhow!(
                "Failed to move uploaded file into place '{}': {}",
                dest.display(),
                rename_err
            ));
        }

        return Ok(StreamWriteOutcome {
            path: dest.to_string_lossy().to_string(),
            bytes_written,
            total_size: bytes_written,
            sha256: stream_sha,
        });
    }

    let mut opts = tokio::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(false);
    if append && offset.is_none() {
        opts.append(true);
    }

    let mut file = opts.open(dest).await?;
    if let Some(off) = offset {
        file.seek(SeekFrom::Start(off)).await?;
    }

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| anyhow::anyhow!("Stream read error: {}", e))?;
        let slice = chunk.as_ref();
        if !slice.is_empty() {
            file.write_all(slice).await?;
            hasher.update(slice);
            bytes_written += slice.len() as u64;
        }
    }
    file.flush().await?;

    let stream_sha = bytes_to_hex(&hasher.finalize());
    verify_sha256(&stream_sha, expected_sha256, "upload chunk")?;
    let total_size = tokio::fs::metadata(dest).await?.len();

    Ok(StreamWriteOutcome {
        path: dest.to_string_lossy().to_string(),
        bytes_written,
        total_size,
        sha256: stream_sha,
    })
}

/// `fs.stat`: inspects file metadata and optional SHA-256 digest.
pub async fn fs_stat(params: &serde_json::Value) -> Result<serde_json::Value> {
    let raw_path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing parameter: path"))?;
    let path = resolve_path(raw_path)?;

    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({
                "ok": true,
                "exists": false,
                "path": path.to_string_lossy(),
            }));
        }
        Err(e) => return Err(e.into()),
    };

    let is_file = meta.is_file();
    let is_dir = meta.is_dir();
    let size = meta.len();
    let modified_unix = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    let compute_hash = match params
        .get("include_sha256")
        .or_else(|| params.get("sha256"))
        .and_then(|v| v.as_bool())
    {
        Some(explicit) => explicit && is_file,
        None => is_file && size <= 16 * 1024 * 1024,
    };

    let sha256 = if compute_hash {
        Some(sha256_file_hex(&path).await?)
    } else {
        None
    };

    Ok(json!({
        "ok": true,
        "exists": true,
        "path": path.to_string_lossy(),
        "is_file": is_file,
        "is_dir": is_dir,
        "size": size,
        "modified_unix": modified_unix,
        "sha256": sha256,
    }))
}

/// `fs.download`: reads a file (or byte chunk) as Base64 with SHA-256 metadata.
pub async fn fs_download(params: &serde_json::Value) -> Result<serde_json::Value> {
    let raw_path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing parameter: path"))?;
    let path = resolve_path(raw_path)?;

    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
    let requested_chunk = params
        .get("chunk_size")
        .or_else(|| params.get("limit_bytes"))
        .or_else(|| params.get("length"))
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_CHUNK_SIZE as u64);
    let chunk_size = (requested_chunk as usize).clamp(1, MAX_CHUNK_SIZE);
    let include_file_sha256 = params
        .get("include_file_sha256")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let meta = tokio::fs::metadata(&path).await?;
    if !meta.is_file() {
        return Err(anyhow::anyhow!("Target path is not a regular file: {}", path.display()));
    }
    let total_size = meta.len();

    if offset >= total_size {
        let file_sha256 = if include_file_sha256 {
            Some(sha256_file_hex(&path).await?)
        } else {
            None
        };
        return Ok(json!({
            "ok": true,
            "path": path.to_string_lossy(),
            "offset": offset,
            "bytes_read": 0,
            "total_size": total_size,
            "next_offset": total_size,
            "eof": true,
            "base64": "",
            "chunk_sha256": sha256_hex(&[]),
            "file_sha256": file_sha256,
        }));
    }

    let mut file = tokio::fs::File::open(&path).await?;
    file.seek(SeekFrom::Start(offset)).await?;

    let to_read = ((total_size - offset) as usize).min(chunk_size);
    let mut buf = vec![0u8; to_read];
    let mut filled = 0usize;
    while filled < to_read {
        let n = file.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);

    let next_offset = offset + filled as u64;
    let eof = next_offset >= total_size || filled == 0;
    let chunk_sha256 = sha256_hex(&buf);
    let file_sha256 = if offset == 0 && eof {
        Some(chunk_sha256.clone())
    } else if include_file_sha256 {
        Some(sha256_file_hex(&path).await?)
    } else {
        None
    };

    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);

    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy(),
        "offset": offset,
        "bytes_read": filled,
        "total_size": total_size,
        "next_offset": next_offset,
        "eof": eof,
        "base64": b64,
        "chunk_sha256": chunk_sha256,
        "file_sha256": file_sha256,
    }))
}

/// `fs.upload`: decodes Base64 payload and writes (or appends/patches at offset) to disk,
/// optionally creating parent directories and verifying chunk/file SHA-256 digests.
pub async fn fs_upload(params: &serde_json::Value) -> Result<serde_json::Value> {
    let raw_path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing parameter: path"))?;
    let path = resolve_path(raw_path)?;

    let b64_str = params
        .get("base64")
        .or_else(|| params.get("data_base64"))
        .or_else(|| params.get("chunk_base64"))
        .or_else(|| params.get("content_base64"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing parameter: base64"))?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_str.trim())
        .map_err(|e| anyhow::anyhow!("Invalid base64 payload: {}", e))?;

    let offset = params.get("offset").and_then(|v| v.as_u64());
    let append = params.get("append").and_then(|v| v.as_bool()).unwrap_or(false);
    let truncate = params
        .get("truncate")
        .and_then(|v| v.as_bool())
        .unwrap_or(!append && offset.unwrap_or(0) == 0);
    let create_dirs = params
        .get("create_dirs")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let finalize = params
        .get("finalize")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let generic_sha = params
        .get("expected_sha256")
        .or_else(|| params.get("sha256"))
        .and_then(|v| v.as_str());
    let expected_chunk_sha = params
        .get("expected_chunk_sha256")
        .and_then(|v| v.as_str())
        .or_else(|| if !finalize { generic_sha } else { None });
    let expected_file_sha = params
        .get("expected_file_sha256")
        .and_then(|v| v.as_str())
        .or_else(|| if finalize || truncate { generic_sha } else { None });

    let chunk_sha256 = sha256_hex(&bytes);
    verify_sha256(&chunk_sha256, expected_chunk_sha, "upload chunk")?;

    ensure_parent_dirs(&path, create_dirs).await?;

    let is_single_shot = truncate && !append && offset.unwrap_or(0) == 0;
    if is_single_shot {
        let tmp_path = temp_sibling_path(&path, "ddb_upload");
        let write_res: Result<()> = async {
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            file.write_all(&bytes).await?;
            file.flush().await?;
            Ok(())
        }
        .await;

        if let Err(e) = write_res {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(e);
        }

        verify_sha256(&chunk_sha256, expected_file_sha, "uploaded file")?;

        if let Err(rename_err) = tokio::fs::rename(&tmp_path, &path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(anyhow::anyhow!(
                "Failed to replace '{}': {}",
                path.display(),
                rename_err
            ));
        }

        return Ok(json!({
            "ok": true,
            "path": path.to_string_lossy(),
            "offset": 0,
            "bytes_written": bytes.len(),
            "total_size": bytes.len(),
            "chunk_sha256": chunk_sha256,
            "file_sha256": chunk_sha256,
        }));
    }

    let mut opts = tokio::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(false);
    if append && offset.is_none() {
        opts.append(true);
    }
    let mut file = opts.open(&path).await?;
    if let Some(off) = offset {
        file.seek(SeekFrom::Start(off)).await?;
    }
    file.write_all(&bytes).await?;
    file.flush().await?;

    let total_size = tokio::fs::metadata(&path).await?.len();
    let file_sha256 = if finalize || expected_file_sha.is_some() {
        let digest = sha256_file_hex(&path).await?;
        verify_sha256(&digest, expected_file_sha, "finalized file")?;
        Some(digest)
    } else {
        None
    };

    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy(),
        "offset": offset,
        "bytes_written": bytes.len(),
        "total_size": total_size,
        "chunk_sha256": chunk_sha256,
        "file_sha256": file_sha256,
    }))
}

/// `fs.fetch`: downloads a remote HTTP/HTTPS URL directly onto the device disk via streaming,
/// with optional custom headers, `create_dirs`, and SHA-256 verification.
pub async fn fs_fetch(params: &serde_json::Value) -> Result<serde_json::Value> {
    let url = params
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing parameter: url"))?;

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow::anyhow!(
            "Unsupported URL scheme for fs.fetch (only http:// and https:// allowed)"
        ));
    }

    let raw_path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing parameter: path"))?;
    let path = resolve_path(raw_path)?;

    let create_dirs = params
        .get("create_dirs")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let expected_sha256 = params
        .get("expected_sha256")
        .or_else(|| params.get("sha256"))
        .and_then(|v| v.as_str());
    let timeout_secs = params
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .clamp(1, 3600);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .user_agent("DDB-FileTransfer/1.0")
        .build()?;

    let mut req = client.get(url);
    if let Some(headers) = params.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers {
            if let Some(val_str) = v.as_str() {
                req = req.header(k.as_str(), val_str);
            }
        }
    }

    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("fs.fetch HTTP error {} for {}", status, url));
    }

    let outcome = write_stream_to_path(
        &path,
        resp.bytes_stream(),
        create_dirs,
        false,
        None,
        expected_sha256,
    )
    .await?;

    Ok(json!({
        "ok": true,
        "url": url,
        "status": status.as_u16(),
        "path": outcome.path,
        "bytes_written": outcome.bytes_written,
        "total_size": outcome.total_size,
        "sha256": outcome.sha256,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chunked_upload_and_download_roundtrip_with_sha256() {
        let dir = std::env::temp_dir().join(format!("ddb_files_test_{}", rand::random::<u64>()));
        let file_path = dir.join("nested").join("payload.bin");
        let payload: Vec<u8> = (0..10_000).map(|i| (i % 251) as u8).collect();
        let expected_sha = sha256_hex(&payload);

        let chunk1 = base64::engine::general_purpose::STANDARD.encode(&payload[..4096]);
        let chunk2 = base64::engine::general_purpose::STANDARD.encode(&payload[4096..]);

        let up1 = fs_upload(&json!({
            "path": file_path.to_string_lossy(),
            "base64": chunk1,
            "offset": 0,
            "create_dirs": true,
        }))
        .await
        .unwrap();
        assert_eq!(up1["bytes_written"], 4096);

        let up2 = fs_upload(&json!({
            "path": file_path.to_string_lossy(),
            "base64": chunk2,
            "offset": 4096,
            "finalize": true,
            "expected_file_sha256": expected_sha,
        }))
        .await
        .unwrap();
        assert_eq!(up2["total_size"], payload.len() as u64);
        assert_eq!(up2["file_sha256"], expected_sha);

        let dl1 = fs_download(&json!({
            "path": file_path.to_string_lossy(),
            "offset": 0,
            "chunk_size": 6000,
        }))
        .await
        .unwrap();
        assert_eq!(dl1["eof"], false);
        assert_eq!(dl1["bytes_read"], 6000);

        let dl2 = fs_download(&json!({
            "path": file_path.to_string_lossy(),
            "offset": 6000,
            "chunk_size": 6000,
            "include_file_sha256": true,
        }))
        .await
        .unwrap();
        assert_eq!(dl2["eof"], true);
        assert_eq!(dl2["file_sha256"], expected_sha);

        let mut reassembled = base64::engine::general_purpose::STANDARD
            .decode(dl1["base64"].as_str().unwrap())
            .unwrap();
        reassembled.extend(
            base64::engine::general_purpose::STANDARD
                .decode(dl2["base64"].as_str().unwrap())
                .unwrap(),
        );
        assert_eq!(reassembled, payload);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn upload_rejects_bad_sha256() {
        let dir = std::env::temp_dir().join(format!("ddb_files_badsha_{}", rand::random::<u64>()));
        let file_path = dir.join("test.bin");
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello world");

        let err = fs_upload(&json!({
            "path": file_path.to_string_lossy(),
            "base64": b64,
            "expected_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("SHA-256 mismatch"));
        assert!(!file_path.exists());
    }
}
