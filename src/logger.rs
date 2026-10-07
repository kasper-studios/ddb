use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Cloud,
    Tunnel,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
}

const MAX_LOG_ENTRIES: usize = 120;

static LOGS: LazyLock<Mutex<VecDeque<LogEntry>>> =
    LazyLock::new(|| Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)));
static IS_TUI_RUNNING: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));

pub fn set_tui_running(running: bool) {
    if let Ok(mut g) = IS_TUI_RUNNING.lock() {
        *g = running;
    }
}

pub fn is_tui_running() -> bool {
    IS_TUI_RUNNING.lock().map(|g| *g).unwrap_or(false)
}

fn get_timestamp() -> String {
    let now = SystemTime::now();
    let duration = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, mins, s)
}

pub fn log(level: LogLevel, msg: impl Into<String>) {
    let msg_str = msg.into();
    let entry = LogEntry {
        timestamp: get_timestamp(),
        level: level.clone(),
        message: msg_str.clone(),
    };

    if let Ok(mut buffer) = LOGS.lock() {
        if buffer.len() >= MAX_LOG_ENTRIES {
            buffer.pop_front();
        }
        buffer.push_back(entry);
    }

    // Если TUI НЕ запущен (фоновый режим --daemon), пишем в консоль
    if !is_tui_running() {
        match level {
            LogLevel::Error => eprintln!("{}", msg_str),
            _ => println!("{}", msg_str),
        }
    }
}

pub fn get_recent_logs(limit: usize) -> Vec<LogEntry> {
    if let Ok(buffer) = LOGS.lock() {
        buffer.iter().rev().take(limit).rev().cloned().collect()
    } else {
        Vec::new()
    }
}
