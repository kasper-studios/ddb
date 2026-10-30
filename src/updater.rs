use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

pub static UPDATE_AVAILABLE: AtomicBool = AtomicBool::new(false);
static LATEST_VERSION: RwLock<String> = RwLock::new(String::new());
static IS_UPDATING: AtomicBool = AtomicBool::new(false);
static UPDATE_STATUS: RwLock<Option<String>> = RwLock::new(None);

pub fn get_latest_version() -> String {
    LATEST_VERSION.read().unwrap().clone()
}

pub fn get_update_status() -> Option<String> {
    UPDATE_STATUS.read().unwrap().clone()
}

pub fn set_update_status(status: Option<String>) {
    *UPDATE_STATUS.write().unwrap() = status;
}

pub fn is_updating() -> bool {
    IS_UPDATING.load(Ordering::Relaxed)
}

/// Проверяет наличие свежего релиза на GitHub без использования api.github.com
/// (чтобы не упираться в лимиты 60 req/h и 403 Forbidden).
pub async fn check_for_updates() -> Result<Option<String>> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("DDB-Updater/1.0")
        .build()?;

    let url = "https://github.com/kasper-studios/ddb/releases/latest";
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    // GitHub отдаёт 302 с Location: /kasper-studios/ddb/releases/tag/vX.Y.Z
    let tag = if resp.status().is_redirection() {
        if let Some(loc) = resp.headers().get("location") {
            let loc_str = loc.to_str().unwrap_or("");
            loc_str.split("/tag/").nth(1).map(|s| s.to_string())
        } else {
            None
        }
    } else {
        None
    };

    if let Some(tag_ver) = tag {
        let clean_ver = tag_ver.trim_start_matches('v').trim();
        let current_ver = env!("CARGO_PKG_VERSION").trim();

        if is_newer(clean_ver, current_ver) {
            *LATEST_VERSION.write().unwrap() = tag_ver.clone();
            UPDATE_AVAILABLE.store(true, Ordering::Relaxed);
            return Ok(Some(tag_ver));
        }
    }

    Ok(None)
}

fn is_newer(remote: &str, current: &str) -> bool {
    let parse_parts = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    };

    let r_parts = parse_parts(remote);
    let c_parts = parse_parts(current);

    for (r, c) in r_parts.iter().zip(c_parts.iter()) {
        if r > c {
            return true;
        }
        if r < c {
            return false;
        }
    }
    r_parts.len() > c_parts.len()
}

/// Скачивает свежий бинарник и заменяет текущий запущенный exe
pub async fn perform_update() -> Result<()> {
    if IS_UPDATING.swap(true, Ordering::Relaxed) {
        return Ok(());
    }

    set_update_status(Some("Стягиваем свежий поносик с GitHub...".into()));

    let tag = get_latest_version();
    if tag.is_empty() {
        set_update_status(Some("Ошибка: версия не определена".into()));
        IS_UPDATING.store(false, Ordering::Relaxed);
        return Err(anyhow::anyhow!("No update tag"));
    }

    let os = std::env::consts::OS;
    let asset_name = if os == "windows" {
        "ddb-windows-x64.exe"
    } else if cfg!(feature = "hermes") {
        "ddb-linux-kasper-hermes"
    } else {
        "ddb-linux-x64"
    };

    let download_url = format!(
        "https://github.com/kasper-studios/ddb/releases/download/{}/{}",
        tag, asset_name
    );

    let client = reqwest::Client::builder()
        .user_agent("DDB-Updater/1.0")
        .build()?;

    set_update_status(Some(format!("Качаем {}...", asset_name)));
    let resp = client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        let err_msg = format!("GitHub ответил статусом: {}", resp.status());
        set_update_status(Some(err_msg.clone()));
        IS_UPDATING.store(false, Ordering::Relaxed);
        return Err(anyhow::anyhow!(err_msg));
    }

    let bytes = resp.bytes().await?;
    let current_exe = std::env::current_exe()?;

    set_update_status(Some("Заменяем бинарник на хосте...".into()));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tmp_path = current_exe.with_extension("tmp_update");
        std::fs::write(&tmp_path, &bytes)?;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp_path, &current_exe)?;
    }

    #[cfg(windows)]
    {
        let old_exe = current_exe.with_extension("old_exe");
        let _ = std::fs::remove_file(&old_exe);
        if let Err(e) = std::fs::rename(&current_exe, &old_exe) {
            set_update_status(Some(format!("Не удалось переименовать старый exe: {}", e)));
            IS_UPDATING.store(false, Ordering::Relaxed);
            return Err(e.into());
        }
        if let Err(e) = std::fs::write(&current_exe, &bytes) {
            let _ = std::fs::rename(&old_exe, &current_exe);
            set_update_status(Some(format!("Ошибка записи нового exe: {}", e)));
            IS_UPDATING.store(false, Ordering::Relaxed);
            return Err(e.into());
        }
    }

    UPDATE_AVAILABLE.store(false, Ordering::Relaxed);
    IS_UPDATING.store(false, Ordering::Relaxed);
    set_update_status(Some("✓ Обновка успешно установлена! Перезапустите DDB.".into()));

    Ok(())
}
