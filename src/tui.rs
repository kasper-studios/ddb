use crate::config::{load_config, save_config, DdbConfig};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Paragraph,
    },
    Terminal,
};
use std::io;
use std::net::UdpSocket;
use std::time::Duration;

#[derive(Clone, PartialEq, Eq)]
enum Screen {
    MainMenu,
    Permissions,
    Tunnels,
    Feedback,
    InputPrompt {
        title: String,
        prompt: String,
        target_field: InputField,
        buffer: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputField {
    CloudKey,
    CloudUrl,
    CloudflareToken,
    NgrokToken,
    CustomTunnelUrl,
    HostName,
}

struct AppState {
    screen: Screen,
    selected_index: usize,
    cfg: DdbConfig,
    notice_msg: Option<(String, Color)>,
    supervisor_active: bool,
    autostart_enabled: bool,
    lan_ip: String,
}

pub fn run_tui() -> anyhow::Result<()> {
    crate::logger::set_tui_running(true);
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = tui_loop(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    crate::logger::set_tui_running(false);

    res
}

fn censor_log_message(msg: &str) -> String {
    let mut out = msg.to_string();
    if let Some(idx) = out.find("ks_ddb_") {
        let rest = &out[idx..];
        let end = rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '}').unwrap_or(rest.len());
        out.replace_range(idx..idx + end, "ks_ddb_****************");
    }
    if let Some(idx) = out.find("ddb_") {
        let rest = &out[idx..];
        let end = rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '}').unwrap_or(rest.len());
        out.replace_range(idx..idx + end, "ddb_****************");
    }
    out
}

fn get_local_ip() -> String {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return "127.0.0.1".into(),
    };
    if socket.connect("8.8.8.8:80").is_ok() {
        if let Ok(addr) = socket.local_addr() {
            return addr.ip().to_string();
        }
    }
    "127.0.0.1".into()
}

fn check_daemon_online() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 8790)),
        Duration::from_millis(300),
    )
    .is_ok()
}

fn tui_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let mut app = AppState {
        screen: Screen::MainMenu,
        selected_index: 0,
        cfg: load_config(),
        notice_msg: None,
        supervisor_active: check_daemon_online(),
        autostart_enabled: check_autostart_enabled(),
        lan_ip: get_local_ip(),
    };

    let mut last_check = std::time::Instant::now();

    loop {
        if last_check.elapsed() > Duration::from_secs(2) {
            app.supervisor_active = check_daemon_online();
            app.autostart_enabled = check_autostart_enabled();
            last_check = std::time::Instant::now();
        }

        terminal.draw(|f| {
            let size = f.area();

            // Background frame
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(5),  // Header Banner with Logo & Config path
                    Constraint::Length(5),  // Device Quick Info / Registration string
                    Constraint::Min(10),    // Active Menu / Settings
                    Constraint::Length(6),  // Live Event Log Box
                    Constraint::Length(1),  // Footer Navigation Bar
                ])
                .split(size);

            // ── 1. HEADER BANNER ─────────────────────────────────────
            let cfg_path_str = crate::config::config_path().to_string_lossy().to_string();
            let header_text = vec![
                Line::from(vec![
                    Span::styled("⚡ DDB (Dirom Device Bridge) ", Style::default().fg(Color::Rgb(168, 85, 247)).add_modifier(Modifier::BOLD)),
                    Span::styled("v3.0.0-RUST ", Style::default().fg(Color::Rgb(216, 180, 254))),
                    Span::styled("[Инструмент Настройки Хоста]", Style::default().fg(Color::Yellow)),
                    Span::raw("  │  "),
                    Span::styled("Конфиг: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&cfg_path_str, Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::raw("Суверенный агент устройств Цитадели | "),
                    Span::styled("Архитектура: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH), Style::default().fg(Color::Cyan)),
                    Span::raw(" | "),
                    Span::styled("Статус демона: ", Style::default().fg(Color::DarkGray)),
                    if app.supervisor_active {
                        Span::styled("● АКТИВЕН (:8790)", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                    } else {
                        Span::styled("○ ОФФЛАЙН", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    },
                    Span::raw(" | "),
                    if app.cfg.streamer_mode {
                        Span::styled("🛡️ STREAMER MODE [ON]", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD))
                    } else {
                        Span::styled("STREAMER MODE [OFF] (нажмите 's')", Style::default().fg(Color::DarkGray))
                    },
                ]),
            ];
            let header = Paragraph::new(header_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(147, 51, 234)))
                    .title(Span::styled(" Главный Терминал DDB ", Style::default().fg(Color::Rgb(216, 180, 254)).add_modifier(Modifier::BOLD)))
            );
            f.render_widget(header, chunks[0]);

            // ── 2. REGISTRATION & CONNECTION BAR ─────────────────────
            let display_ip = if app.cfg.streamer_mode {
                "***.***.***.***".to_string()
            } else {
                app.lan_ip.clone()
            };

            let display_api_key = if app.cfg.streamer_mode {
                "************************".to_string()
            } else {
                app.cfg.api_key.clone()
            };

            let target_url = if !app.cfg.tunnel_url.is_empty() {
                if app.cfg.streamer_mode {
                    "https://***.trycloudflare.com".to_string()
                } else {
                    app.cfg.tunnel_url.clone()
                }
            } else if app.cfg.tunnel_type == "lan" {
                format!("http://{}:8790", display_ip)
            } else {
                format!("http://{}:8790", display_ip)
            };

            let reg_cmd = format!("device.reg(name='{}', url='{}', key='{}')", app.cfg.host_name, target_url, display_api_key);

            let (status_title, reg_lines) = if app.cfg.tunnel_type == "cloud" {
                let is_connected = crate::cloud::is_cloud_connected();
                let cloud_badge = if is_connected {
                    Span::styled("● ПОДКЛЮЧЕНО К ОБЛАКУ (Zero-Config Active)", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled("◌ ПОДКЛЮЧЕНИЕ К ОБЛАКУ...", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                };

                let lines = vec![
                    Line::from(vec![
                        Span::styled("Хост: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&app.cfg.host_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled("  │  Режим: ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Kasper Cloud API", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::styled("  │  Статус: ", Style::default().fg(Color::DarkGray)),
                        cloud_badge,
                        Span::styled("  │  API Key: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(display_api_key, Style::default().fg(Color::Rgb(168, 85, 247))),
                    ]),
                    Line::from(vec![
                        Span::styled("Шлюз: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(if app.cfg.cloud_url.is_empty() { "wss://diromstream.kasperstudios.xyz/api/v1/bridge" } else { &app.cfg.cloud_url }, Style::default().fg(Color::Cyan)),
                        Span::styled("  │  Диром: ", Style::default().fg(Color::DarkGray)),
                        Span::styled("🪄 Zero-Config активен! Устройство автоматически регистрируется в Цитадели.", Style::default().fg(Color::Green)),
                    ]),
                ];
                (" Kasper Cloud API (Zero-Config) ", lines)
            } else {
                let lines = vec![
                    Line::from(vec![
                        Span::styled("Хост: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&app.cfg.host_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled("  │  Связь: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&app.cfg.tunnel_type, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::styled("  │  Адрес: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&target_url, Style::default().fg(Color::Yellow)),
                        Span::styled("  │  API Key: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(display_api_key, Style::default().fg(Color::Rgb(168, 85, 247))),
                    ]),
                    Line::from(vec![
                        Span::styled("Скопируйте для Дирома: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::styled(&reg_cmd, Style::default().fg(Color::Yellow)),
                    ]),
                ];
                (" Параметры Подключения в Диром ", lines)
            };

            let reg_widget = Paragraph::new(reg_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(75, 85, 99)))
                    .title(Span::styled(status_title, Style::default().fg(Color::Cyan)))
            );
            f.render_widget(reg_widget, chunks[1]);

            // ── 3. MAIN WORKSPACE / INTERACTIVE MENU ─────────────────
            match &app.screen {
                Screen::MainMenu => render_main_menu(f, chunks[2], &app),
                Screen::Permissions => render_permissions_menu(f, chunks[2], &app),
                Screen::Tunnels => render_tunnels_menu(f, chunks[2], &app),
                Screen::Feedback => render_feedback_menu(f, chunks[2], &app),
                Screen::InputPrompt { title, prompt, buffer, .. } => {
                    render_input_prompt(f, chunks[2], title, prompt, buffer);
                }
            }

            // ── 4. EVENT LOGS BOX (NEW) ──────────────────────────────
            let logs = crate::logger::get_recent_logs(5);
            let log_lines: Vec<Line> = if logs.is_empty() {
                vec![Line::from(Span::styled("  Ожидание сетевых событий...", Style::default().fg(Color::DarkGray)))]
            } else {
                logs.into_iter().map(|entry| {
                    let level_style = match entry.level {
                        crate::logger::LogLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        crate::logger::LogLevel::Warn => Style::default().fg(Color::Yellow),
                        crate::logger::LogLevel::Cloud => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                        crate::logger::LogLevel::Tunnel => Style::default().fg(Color::Magenta),
                        crate::logger::LogLevel::Info => Style::default().fg(Color::White),
                    };

                    let safe_msg = if app.cfg.streamer_mode {
                        censor_log_message(&entry.message)
                    } else {
                        entry.message
                    };

                    Line::from(vec![
                        Span::styled(format!(" [{}] ", entry.timestamp), Style::default().fg(Color::DarkGray)),
                        Span::styled(safe_msg, level_style),
                    ])
                }).collect()
            };

            let log_widget = Paragraph::new(log_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(75, 85, 99)))
                    .title(Span::styled(" 📜 Журнал Событий Хоста (Live Logs) ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            );
            f.render_widget(log_widget, chunks[3]);

            // ── 5. FOOTER COMMAND BAR ────────────────────────────────
            let footer_line = match &app.screen {
                Screen::MainMenu => Line::from(vec![
                    Span::styled(" [↑ / ↓] ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(" Навигация  "),
                    Span::styled(" [Enter / 1-6] ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(" Выбрать  "),
                    Span::styled(" [s] ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(" Streamer Mode  "),
                    Span::styled(" [q / Esc] ", Style::default().fg(Color::Black).bg(Color::Rgb(244, 63, 94)).add_modifier(Modifier::BOLD)),
                    Span::raw(" Выход  "),
                ]),
                Screen::Tunnels => Line::from(vec![
                    Span::styled(" [↑ / ↓] ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(" Навигация  "),
                    Span::styled(" [Enter / 1-5] ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(" Выбрать  "),
                    Span::styled(" [e] ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(" Изменить токен/URL  "),
                    Span::styled(" [h] ", Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD)),
                    Span::raw(" Имя хоста  "),
                    Span::styled(" [s] ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(" Streamer Mode  "),
                    Span::styled(" [Esc] ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw(" Назад  "),
                ]),
                Screen::InputPrompt { .. } => Line::from(vec![
                    Span::styled(" [Enter] ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(" Сохранить  "),
                    Span::styled(" [Backspace] ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(" Стереть  "),
                    Span::styled(" [Esc] ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw(" Отмена  "),
                ]),
                _ => Line::from(vec![
                    Span::styled(" [↑ / ↓] ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(" Навигация  "),
                    Span::styled(" [Enter / Пробел] ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(" Переключить  "),
                    Span::styled(" [s] ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(" Streamer Mode  "),
                    Span::styled(" [Esc / 0] ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(" Назад в главное меню  "),
                ]),
            };
            let footer = Paragraph::new(footer_line).style(Style::default().bg(Color::Rgb(17, 24, 39)));
            f.render_widget(footer, chunks[4]);

            // Pop-up Notice (if any)
            if let Some((msg, color)) = &app.notice_msg {
                render_popup_notice(f, size, msg, *color);
            }
        })?;

        // ── EVENT HANDLING ───────────────────────────────────────────
        if event::poll(Duration::from_millis(150))? {
            if let Event::Key(key) = event::read()? {
                // Игнорируем события Release на Windows во избежание дублирования клавиш
                if key.kind != event::KeyEventKind::Press {
                    continue;
                }

                // Clear notice on key press
                if app.notice_msg.is_some() {
                    app.notice_msg = None;
                    continue;
                }

                // Хоткей Streamer Mode (работает везде, кроме ввода текста)
                if key.code == KeyCode::Char('s') {
                    if !matches!(app.screen, Screen::InputPrompt { .. }) {
                        app.cfg.streamer_mode = !app.cfg.streamer_mode;
                        let _ = save_config(&app.cfg);
                        let msg = if app.cfg.streamer_mode {
                            "🛡️ STREAMER MODE ВКЛЮЧЕН! (Ключи и IP скрыты звёздочками)".to_string()
                        } else {
                            "STREAMER MODE ВЫКЛЮЧЕН (Ключи и IP видны)".to_string()
                        };
                        app.notice_msg = Some((msg, Color::Yellow));
                        continue;
                    }
                }

                match &mut app.screen {
                    Screen::MainMenu => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Up => {
                            if app.selected_index > 0 {
                                app.selected_index -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if app.selected_index < 5 {
                                app.selected_index += 1;
                            }
                        }
                        KeyCode::Char('1') => open_menu_item(&mut app, 0),
                        KeyCode::Char('2') => open_menu_item(&mut app, 1),
                        KeyCode::Char('3') => open_menu_item(&mut app, 2),
                        KeyCode::Char('4') => open_menu_item(&mut app, 3),
                        KeyCode::Char('5') => open_menu_item(&mut app, 4),
                        KeyCode::Char('6') => open_menu_item(&mut app, 5),
                        KeyCode::Enter => {
                            let idx = app.selected_index;
                            open_menu_item(&mut app, idx);
                        }
                        _ => {}
                    },

                    Screen::Permissions => match key.code {
                        KeyCode::Esc | KeyCode::Char('0') => {
                            app.screen = Screen::MainMenu;
                            app.selected_index = 0;
                        }
                        KeyCode::Up => {
                            if app.selected_index > 0 {
                                app.selected_index -= 1;
                            }
                        }
                        KeyCode::Down => {
                            let max_idx = if cfg!(feature = "hermes") { 7 } else { 6 };
                            if app.selected_index < max_idx {
                                app.selected_index += 1;
                            }
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            let idx = app.selected_index;
                            toggle_permission(&mut app, idx);
                        }
                        KeyCode::Char('1') => toggle_permission(&mut app, 0),
                        KeyCode::Char('2') => toggle_permission(&mut app, 1),
                        KeyCode::Char('3') => toggle_permission(&mut app, 2),
                        KeyCode::Char('4') => toggle_permission(&mut app, 3),
                        KeyCode::Char('5') => toggle_permission(&mut app, 4),
                        KeyCode::Char('6') => toggle_permission(&mut app, 5),
                        KeyCode::Char('7') => toggle_permission(&mut app, 6),
                        KeyCode::Char('8') => {
                            if cfg!(feature = "hermes") {
                                toggle_permission(&mut app, 7);
                            }
                        }
                        _ => {}
                    },

                    Screen::Tunnels => match key.code {
                        KeyCode::Esc | KeyCode::Char('0') => {
                            app.screen = Screen::MainMenu;
                            app.selected_index = 1;
                        }
                        KeyCode::Up => {
                            if app.selected_index > 0 {
                                app.selected_index -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if app.selected_index < 4 {
                                app.selected_index += 1;
                            }
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            let idx = app.selected_index;
                            select_tunnel(&mut app, idx);
                        }
                        KeyCode::Char('e') | KeyCode::Char('t') => {
                            let idx = app.selected_index;
                            open_tunnel_input(&mut app, idx);
                        }
                        KeyCode::Char('h') => {
                            app.screen = Screen::InputPrompt {
                                title: "Имя устройства (Hostname)".to_string(),
                                prompt: "Введите имя этого компьютера в Дироме (например, ferox-pc):".to_string(),
                                target_field: InputField::HostName,
                                buffer: app.cfg.host_name.clone(),
                            };
                        }
                        KeyCode::Char('u') => {
                            app.screen = Screen::InputPrompt {
                                title: "Kasper Cloud WSS URL".to_string(),
                                prompt: "Введите WSS URL облачного шлюза (по умолчанию: wss://api.kasperstudios.xyz/ddb/ws):".to_string(),
                                target_field: InputField::CloudUrl,
                                buffer: app.cfg.cloud_url.clone(),
                            };
                        }
                        KeyCode::Char('1') => select_tunnel(&mut app, 0),
                        KeyCode::Char('2') => select_tunnel(&mut app, 1),
                        KeyCode::Char('3') => select_tunnel(&mut app, 2),
                        KeyCode::Char('4') => select_tunnel(&mut app, 3),
                        KeyCode::Char('5') => select_tunnel(&mut app, 4),
                        _ => {}
                    },

                    Screen::InputPrompt { target_field, buffer, .. } => match key.code {
                        KeyCode::Esc => {
                            app.screen = Screen::Tunnels;
                        }
                        KeyCode::Backspace => {
                            buffer.pop();
                        }
                        KeyCode::Char(c) => {
                            buffer.push(c);
                        }
                        KeyCode::Enter => {
                            let val = buffer.trim().to_string();
                            match target_field {
                                InputField::CloudKey => {
                                    if !val.is_empty() {
                                        app.cfg.api_key = val;
                                        app.notice_msg = Some(("API-ключ обновлен!".into(), Color::Green));
                                    }
                                }
                                InputField::CloudUrl => {
                                    if !val.is_empty() {
                                        app.cfg.cloud_url = val;
                                        app.notice_msg = Some(("Cloud WSS URL обновлен!".into(), Color::Green));
                                    }
                                }
                                InputField::CloudflareToken => {
                                    app.cfg.tunnel_token = val.clone();
                                    app.notice_msg = Some(("Токен Cloudflare сохранен!".into(), Color::Green));
                                }
                                InputField::NgrokToken => {
                                    app.cfg.tunnel_token = val.clone();
                                    app.notice_msg = Some(("Токен Ngrok сохранен!".into(), Color::Green));
                                }
                                InputField::CustomTunnelUrl => {
                                    app.cfg.tunnel_url = val.clone();
                                    app.cfg.tunnel_type = "custom".to_string();
                                    app.notice_msg = Some(("Публичный URL сохранен!".into(), Color::Green));
                                }
                                InputField::HostName => {
                                    if !val.is_empty() {
                                        app.cfg.host_name = val;
                                        app.notice_msg = Some(("Имя устройства сохранено!".into(), Color::Green));
                                    }
                                }
                            }
                            let _ = save_config(&app.cfg);
                            app.screen = Screen::Tunnels;
                        }
                        _ => {}
                    },

                    Screen::Feedback => match key.code {
                        KeyCode::Esc | KeyCode::Char('0') => {
                            app.screen = Screen::MainMenu;
                            app.selected_index = 4;
                        }
                        KeyCode::Char('1') => {
                            let _ = std::process::Command::new("xdg-open")
                                .arg("https://discord.gg/kasper")
                                .spawn();
                            app.notice_msg = Some(("Открываю Discord...".into(), Color::Cyan));
                        }
                        KeyCode::Char('2') => {
                            let _ = std::process::Command::new("xdg-open")
                                .arg("https://github.com/kasper-studios/ddb/issues")
                                .spawn();
                            app.notice_msg = Some(("Открываю GitHub Issues...".into(), Color::Cyan));
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    Ok(())
}

fn open_menu_item(app: &mut AppState, idx: usize) {
    match idx {
        0 => {
            app.screen = Screen::Permissions;
            app.selected_index = 0;
        }
        1 => {
            app.screen = Screen::Tunnels;
            app.selected_index = match app.cfg.tunnel_type.as_str() {
                "lan" => 0,
                "cloud" => 1,
                "cloudflare" => 2,
                "ngrok" => 3,
                "custom" => 4,
                _ => 0,
            };
        }
        2 => {
            // Интерактивный TOGGLE автозагрузки
            toggle_autostart(app);
        }
        3 => {
            // Перезапуск службы / воркера
            restart_daemon(app);
        }
        4 => {
            app.screen = Screen::Feedback;
            app.selected_index = 0;
        }
        5 => {
            // Остановка службы / воркера
            stop_daemon(app);
        }
        _ => {}
    }
}

fn toggle_permission(app: &mut AppState, idx: usize) {
    let p = &mut app.cfg.permissions;
    #[cfg(feature = "hermes")]
    match idx {
        0 => p.exec = !p.exec,
        1 => p.hermes = !p.hermes,
        2 => p.screen = !p.screen,
        3 => p.fs.read = !p.fs.read,
        4 => p.fs.write = !p.fs.write,
        5 => p.fs.list = !p.fs.list,
        6 => p.display_control.allow_click = !p.display_control.allow_click,
        7 => p.display_control.allow_keys = !p.display_control.allow_keys,
        _ => {}
    }
    #[cfg(not(feature = "hermes"))]
    match idx {
        0 => p.exec = !p.exec,
        1 => p.screen = !p.screen,
        2 => p.fs.read = !p.fs.read,
        3 => p.fs.write = !p.fs.write,
        4 => p.fs.list = !p.fs.list,
        5 => p.display_control.allow_click = !p.display_control.allow_click,
        6 => p.display_control.allow_keys = !p.display_control.allow_keys,
        _ => {}
    }
    let _ = save_config(&app.cfg);
    app.notice_msg = Some(("Настройки прав сохранены в config.json".into(), Color::Green));
}

fn select_tunnel(app: &mut AppState, idx: usize) {
    match idx {
        0 => app.cfg.tunnel_type = "lan".to_string(),
        1 => {
            app.cfg.tunnel_type = "cloud".to_string();
            if app.cfg.cloud_url.is_empty() {
                app.cfg.cloud_url = "wss://diromstream.kasperstudios.xyz/api/v1/bridge".to_string();
            }
        }
        2 => app.cfg.tunnel_type = "cloudflare".to_string(),
        3 => app.cfg.tunnel_type = "ngrok".to_string(),
        4 => app.cfg.tunnel_type = "custom".to_string(),
        _ => {}
    }
    let _ = save_config(&app.cfg);
    app.notice_msg = Some((format!("Тип туннеля переключен на: '{}'", app.cfg.tunnel_type), Color::Green));
}

fn open_tunnel_input(app: &mut AppState, idx: usize) {
    match idx {
        0 => {
            app.notice_msg = Some(("Локальная сеть использует авто-определённый локальный IP".into(), Color::Yellow));
        }
        1 => {
            app.screen = Screen::InputPrompt {
                title: "Kasper Cloud API".to_string(),
                prompt: "Введите Cloud API ключ (выдан Создателем или Диромом):".to_string(),
                target_field: InputField::CloudKey,
                buffer: app.cfg.api_key.clone(),
            };
        }
        2 => {
            app.screen = Screen::InputPrompt {
                title: "Cloudflare Tunnel".to_string(),
                prompt: "Введите токен Cloudflare туннеля (или оставьте пустым для Quick Tunnel):".to_string(),
                target_field: InputField::CloudflareToken,
                buffer: app.cfg.tunnel_token.clone(),
            };
        }
        3 => {
            app.screen = Screen::InputPrompt {
                title: "Ngrok Tunnel".to_string(),
                prompt: "Введите ваш Ngrok Authtoken:".to_string(),
                target_field: InputField::NgrokToken,
                buffer: app.cfg.tunnel_token.clone(),
            };
        }
        4 => {
            app.screen = Screen::InputPrompt {
                title: "Пользовательский URL Туннеля".to_string(),
                prompt: "Введите публичный URL (например: https://xxx.trycloudflare.com):".to_string(),
                target_field: InputField::CustomTunnelUrl,
                buffer: app.cfg.tunnel_url.clone(),
            };
        }
        _ => {}
    }
}

fn check_autostart_enabled() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let output = std::process::Command::new("reg")
            .args(["query", r#"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"#, "/v", "DDB"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(out) = output {
            return out.status.success();
        }
        false
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = std::process::Command::new("systemctl")
            .args(["--user", "is-enabled", "ddb-supervisor"])
            .output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            return text.trim() == "enabled";
        }
        false
    }
}

fn toggle_autostart(app: &mut AppState) {
    let current = check_autostart_enabled();
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if current {
            let _ = std::process::Command::new("reg")
                .args(["delete", r#"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"#, "/v", "DDB", "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
            app.autostart_enabled = check_autostart_enabled();
            app.notice_msg = Some(("Автозагрузка DDB на Windows выключена.".into(), Color::Yellow));
            crate::logger::log(crate::logger::LogLevel::Info, "[AUTOSTART] Автозагрузка DDB в реестре Windows отключена.");
        } else {
            if let Ok(exe) = std::env::current_exe() {
                let cmd_str = format!("\"{}\" --daemon", exe.to_string_lossy());
                let _ = std::process::Command::new("reg")
                    .args(["add", r#"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"#, "/v", "DDB", "/t", "REG_SZ", "/d", &cmd_str, "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
            }
            app.autostart_enabled = check_autostart_enabled();
            app.notice_msg = Some(("Автозагрузка DDB в реестр Windows [ВКЛЮЧЕНА]!".into(), Color::Green));
            crate::logger::log(crate::logger::LogLevel::Info, "[AUTOSTART] Автозагрузка DDB в реестр Windows успешно добавлена.");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if current {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "ddb-supervisor"])
                .status();
            app.autostart_enabled = check_autostart_enabled();
            app.supervisor_active = check_daemon_online();
            app.notice_msg = Some(("Автозагрузка ddb-supervisor выключена.".into(), Color::Yellow));
            crate::logger::log(crate::logger::LogLevel::Info, "[AUTOSTART] Автозагрузка ddb-supervisor отключена в systemd.");
        } else {
            run_systemd_install(app);
            app.autostart_enabled = check_autostart_enabled();
            crate::logger::log(crate::logger::LogLevel::Info, "[AUTOSTART] Автозагрузка ddb-supervisor включена в systemd.");
        }
    }
}

fn restart_daemon(app: &mut AppState) {
    #[cfg(target_os = "windows")]
    {
        stop_windows_daemon();
        if let Ok(exe) = std::env::current_exe() {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let _ = std::process::Command::new(exe)
                .arg("--daemon")
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
        std::thread::sleep(Duration::from_millis(600));
        app.supervisor_active = check_daemon_online();
        app.notice_msg = Some(("Фоновый воркер DDB на Windows успешно перезапущен!".into(), Color::Green));
        crate::logger::log(crate::logger::LogLevel::Info, "[DAEMON] Фоновый воркер DDB на Windows перезапущен.");
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "restart", "ddb-supervisor"])
            .status();
        app.supervisor_active = check_daemon_online();
        app.notice_msg = Some(("Служба ddb-supervisor успешно перезапущена!".into(), Color::Green));
        crate::logger::log(crate::logger::LogLevel::Info, "[DAEMON] Служба ddb-supervisor перезапущена через systemctl.");
    }
}

fn stop_daemon(app: &mut AppState) {
    #[cfg(target_os = "windows")]
    {
        stop_windows_daemon();
        std::thread::sleep(Duration::from_millis(400));
        app.supervisor_active = check_daemon_online();
        app.notice_msg = Some(("Фоновые процессы DDB на Windows остановлены.".into(), Color::Yellow));
        crate::logger::log(crate::logger::LogLevel::Warn, "[DAEMON] Фоновые процессы DDB на Windows остановлены.");
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "stop", "ddb-supervisor"])
            .status();
        app.supervisor_active = check_daemon_online();
        app.notice_msg = Some(("Служба ddb-supervisor остановлена.".into(), Color::Yellow));
        crate::logger::log(crate::logger::LogLevel::Warn, "[DAEMON] Служба ddb-supervisor остановлена через systemctl.");
    }
}

#[cfg(target_os = "windows")]
fn stop_windows_daemon() {
    let my_pid = std::process::id();
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    for exe_name in &["ddb.exe", "ddb-windows-x64.exe"] {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", exe_name, "/FI", &format!("PID ne {}", my_pid)])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
}

#[cfg(not(target_os = "windows"))]
fn run_systemd_install(app: &mut AppState) {
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy();
        let service_content = format!(
            "[Unit]\nDescription=DDB Native Rust Daemon (Dirom Device Bridge)\nAfter=network.target\n\n[Service]\nType=simple\nExecStart={} --daemon\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
            exe_str
        );

        if let Some(mut dir) = dirs::config_dir() {
            dir.push("systemd");
            dir.push("user");
            let _ = std::fs::create_dir_all(&dir);
            dir.push("ddb-supervisor.service");
            if std::fs::write(&dir, service_content).is_ok() {
                let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
                let _ = std::process::Command::new("systemctl").args(["--user", "enable", "--now", "ddb-supervisor"]).status();
                app.supervisor_active = check_daemon_online();
                app.notice_msg = Some(("Служба ddb-supervisor успешно установлена и запущена в systemd --user!".into(), Color::Green));
                return;
            }
        }
    }
    app.notice_msg = Some(("Ошибка при записи systemd службы".into(), Color::Red));
}

// ── RENDER MENUS ─────────────────────────────────────────────────────

fn render_main_menu(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let (auto_badge, auto_desc) = if app.autostart_enabled {
        (
            Span::styled(" [ВКЛ] ● ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
            if cfg!(target_os = "windows") {
                "Автозапуск в реестре Windows (HKCU/Run) активен [Нажмите Enter для выключения]"
            } else {
                "Служба systemd --user активна [Нажмите Enter для выключения]"
            }
        )
    } else {
        (
            Span::styled(" [ВЫКЛ] ○ ", Style::default().fg(Color::White).bg(Color::Rgb(55, 65, 81))),
            if cfg!(target_os = "windows") {
                "Добавить DDB в автозапуск Windows (HKCU/Run) [Нажмите Enter для включения]"
            } else {
                "Включить службу ddb-supervisor в systemd [Нажмите Enter для включения]"
            }
        )
    };

    let (restart_title, stop_title) = if cfg!(target_os = "windows") {
        ("Перезапустить процесс DDB (Windows Worker)", "Остановить процессы DDB (Windows Worker)")
    } else {
        ("Перезапустить службу DDB Supervisor (systemd)", "Остановить службу DDB Supervisor (systemd)")
    };

    let items = vec![
        make_menu_item("1", "Настройка прав доступа (Permissions Matrix)", "Включение и выключение шелла, мыши, экрана, файлов, Гермеса", None, app.selected_index == 0),
        make_menu_item("2", "Сетевой режим и туннели связи", "Переключение между P2P LAN, Cloudflare, Ngrok, Custom URL и Kasper Cloud", None, app.selected_index == 1),
        make_menu_item("3", "Автозагрузка системы", auto_desc, Some(auto_badge), app.selected_index == 2),
        make_menu_item("4", restart_title, "Быстрый рестарт фонового демона/воркера для применения правок", None, app.selected_index == 3),
        make_menu_item("5", "Обратная связь / Сообщить о баге", "Переход в Discord сообщество или GitHub Issues", None, app.selected_index == 4),
        make_menu_item("6", stop_title, "Мягкая остановка фонового демона/воркера", None, app.selected_index == 5),
    ];

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(147, 51, 234)))
            .title(Span::styled(" Меню Настройки Агента Хоста ", Style::default().fg(Color::Rgb(216, 180, 254)).add_modifier(Modifier::BOLD)))
    );
    f.render_widget(list, area);
}

fn make_menu_item<'a>(num: &'a str, title: &'a str, desc: &'a str, badge: Option<Span<'a>>, selected: bool) -> ListItem<'a> {
    let (prefix, style) = if selected {
        (" ❯ ", Style::default().fg(Color::Rgb(244, 63, 94)).add_modifier(Modifier::BOLD))
    } else {
        ("   ", Style::default().fg(Color::White))
    };

    let mut line_spans = vec![
        Span::styled(prefix, style),
        Span::styled(format!("[{}] ", num), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(title, style),
    ];

    if let Some(b) = badge {
        line_spans.push(Span::raw(" "));
        line_spans.push(b);
    }

    ListItem::new(vec![
        Line::from(line_spans),
        Line::from(vec![
            Span::raw("       "),
            Span::styled(desc.to_string(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ])
}

fn render_permissions_menu(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let p = &app.cfg.permissions;
    #[cfg(feature = "hermes")]
    let items = vec![
        make_perm_item("1", "Выполнение шелл-команд (exec)", "Запуск bash/sh/cmd команд на хосте", p.exec, app.selected_index == 0),
        make_perm_item("2", "Hermes Desktop A2A Synapse", "A2A связь с Гермесом по WebSockets", p.hermes, app.selected_index == 1),
        make_perm_item("3", "Захват экрана (screen)", "Скриншоты Niri Wayland / DirectX Windows", p.screen, app.selected_index == 2),
        make_perm_item("4", "Файловая система: Чтение (fs.read)", "Чтение файлов с диска хоста", p.fs.read, app.selected_index == 3),
        make_perm_item("5", "Файловая система: Запись (fs.write)", "Создание и перезапись файлов на диске", p.fs.write, app.selected_index == 4),
        make_perm_item("6", "Файловая система: Листинг (fs.list)", "Просмотр содержимого директорий", p.fs.list, app.selected_index == 5),
        make_perm_item("7", "Display Control: Мышь и Клик", "Клики и перемещения курсора Безье", p.display_control.allow_click, app.selected_index == 6),
        make_perm_item("8", "Display Control: Клавиатура и Ввод", "Ввод текста и хоткеи", p.display_control.allow_keys, app.selected_index == 7),
    ];
    #[cfg(not(feature = "hermes"))]
    let items = vec![
        make_perm_item("1", "Выполнение шелл-команд (exec)", "Запуск bash/sh/cmd команд на хосте", p.exec, app.selected_index == 0),
        make_perm_item("2", "Захват экрана (screen)", "Скриншоты Niri Wayland / DirectX Windows", p.screen, app.selected_index == 1),
        make_perm_item("3", "Файловая система: Чтение (fs.read)", "Чтение файлов с диска хоста", p.fs.read, app.selected_index == 2),
        make_perm_item("4", "Файловая система: Запись (fs.write)", "Создание и перезапись файлов на диске", p.fs.write, app.selected_index == 3),
        make_perm_item("5", "Файловая система: Листинг (fs.list)", "Просмотр содержимого директорий", p.fs.list, app.selected_index == 4),
        make_perm_item("6", "Display Control: Мышь и Клик", "Клики и перемещения курсора Безье", p.display_control.allow_click, app.selected_index == 5),
        make_perm_item("7", "Display Control: Клавиатура и Ввод", "Ввод текста и хоткеи", p.display_control.allow_keys, app.selected_index == 6),
    ];

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(147, 51, 234)))
            .title(Span::styled(" [Настройка Прав Доступа] Нажмите [Enter] или [Пробел] для переключения ", Style::default().fg(Color::Rgb(216, 180, 254)).add_modifier(Modifier::BOLD)))
    );
    f.render_widget(list, area);
}

fn make_perm_item<'a>(num: &'a str, title: &'a str, desc: &'a str, enabled: bool, selected: bool) -> ListItem<'a> {
    let (status_text, status_color) = if enabled {
        ("[✓ ВКЛЮЧЕНО]", Color::Green)
    } else {
        ("[✗ ВЫКЛЮЧЕНО]", Color::Red)
    };

    let pointer = if selected { " ❯ " } else { "   " };
    let row_style = if selected { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White) };

    ListItem::new(vec![
        Line::from(vec![
            Span::styled(pointer, Style::default().fg(Color::Rgb(244, 63, 94)).add_modifier(Modifier::BOLD)),
            Span::styled(format!("[{}] ", num), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(title, row_style),
            Span::raw("  "),
            Span::styled(status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw("       "),
            Span::styled(desc, Style::default().fg(Color::DarkGray)),
        ]),
    ])
}

fn render_tunnels_menu(f: &mut ratatui::Frame, area: Rect, app: &AppState) {
    let current = app.cfg.tunnel_type.as_str();
    let cloud_url_label = if app.cfg.cloud_url.is_empty() { "wss://diromstream.kasperstudios.xyz/api/v1/bridge" } else { &app.cfg.cloud_url };
    let cf_info = if !app.cfg.tunnel_url.is_empty() && current == "cloudflare" {
        if app.cfg.streamer_mode {
            "URL: https://***.trycloudflare.com".to_string()
        } else {
            format!("URL: {}", app.cfg.tunnel_url)
        }
    } else if !app.cfg.tunnel_token.is_empty() {
        if app.cfg.streamer_mode {
            "Токен: ******************** (нажмите 'e' для изменения)".to_string()
        } else {
            format!("Токен: {}... (нажмите 'e' для изменения)", &app.cfg.tunnel_token.chars().take(12).collect::<String>())
        }
    } else {
        "Подключение по токену или Quick Tunnel (нажмите 'e' для ввода)".to_string()
    };
    let ngrok_info = if !app.cfg.tunnel_url.is_empty() && current == "ngrok" {
        if app.cfg.streamer_mode {
            "URL: https://***.ngrok-free.app".to_string()
        } else {
            format!("URL: {}", app.cfg.tunnel_url)
        }
    } else if !app.cfg.tunnel_token.is_empty() {
        if app.cfg.streamer_mode {
            "Authtoken: ******************** (нажмите 'e' для изменения)".to_string()
        } else {
            format!("Authtoken: {}... (нажмите 'e' для изменения)", &app.cfg.tunnel_token.chars().take(12).collect::<String>())
        }
    } else {
        "Проброс порта :8790 через authtoken ngrok (нажмите 'e' для ввода)".to_string()
    };
    let custom_info = if !app.cfg.tunnel_url.is_empty() {
        if app.cfg.streamer_mode {
            "URL: https://***.custom.url (нажмите 'e' для изменения)".to_string()
        } else {
            format!("URL: {} (нажмите 'e' для изменения)", app.cfg.tunnel_url)
        }
    } else {
        "Вручную вставить любой публичный URL туннеля (нажмите 'e' для ввода)".to_string()
    };

    let lan_ip_str = if app.cfg.streamer_mode {
        "***.***.***.***"
    } else {
        &app.lan_ip
    };
    let lan_info = format!("http://{}:8790 (для одной локальной сети с Диромом)", lan_ip_str);
    let cloud_info = format!("Исходящий WSS к {} [Нажмите 'e' для ключа]", cloud_url_label);

    let items = vec![
        make_tunnel_item("1", "Локальная сеть LAN (Прямой IP)", &lan_info, current == "lan", app.selected_index == 0),
        make_tunnel_item("2", "Kasper Cloud API (Zero-Config Magic)", &cloud_info, current == "cloud", app.selected_index == 1),
        make_tunnel_item("3", "Cloudflare Tunnel", &cf_info, current == "cloudflare", app.selected_index == 2),
        make_tunnel_item("4", "Ngrok Tunnel", &ngrok_info, current == "ngrok", app.selected_index == 3),
        make_tunnel_item("5", "Пользовательский URL (Custom Tunnel URL)", &custom_info, current == "custom", app.selected_index == 4),
    ];

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(147, 51, 234)))
            .title(Span::styled(" [Выбор Способа Связи] [Enter] Выбрать  [e] Изменить токен/URL  [h] Сменить имя хоста ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
    );
    f.render_widget(list, area);
}

fn make_tunnel_item<'a>(num: &'a str, title: &'a str, desc: &'a str, active: bool, selected: bool) -> ListItem<'a> {
    let pointer = if selected { " ❯ " } else { "   " };
    let badge = if active {
        Span::styled(" [АКТИВНО] ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("")
    };

    ListItem::new(vec![
        Line::from(vec![
            Span::styled(pointer, Style::default().fg(Color::Rgb(244, 63, 94)).add_modifier(Modifier::BOLD)),
            Span::styled(format!("[{}] ", num), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(title, if selected { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White) }),
            Span::raw(" "),
            badge,
        ]),
        Line::from(vec![
            Span::raw("       "),
            Span::styled(desc.to_string(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ])
}

fn render_input_prompt(f: &mut ratatui::Frame, area: Rect, title: &str, prompt: &str, buffer: &str) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(prompt, Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ❯ ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(buffer, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Нажмите [Enter] для сохранения, [Esc] для отмены", Style::default().fg(Color::DarkGray))),
    ];

    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow))
            .title(Span::styled(format!(" ✏️ {} ", title), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
    );
    f.render_widget(p, area);
}

fn render_feedback_menu(f: &mut ratatui::Frame, area: Rect, _app: &AppState) {
    let text = vec![
        Line::from(Span::styled("💡 Центр Обратной Связи Цитадели", Style::default().fg(Color::Rgb(216, 180, 254)).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from("  Куда направить идею или баг-репорт:"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [1] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("👾 Discord сервер Kasper Studios ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("(https://discord.gg/kasper)", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [2] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("🐙 GitHub Issues проекта DDB ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("(https://github.com/kasper-studios/ddb/issues)", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Нажмите [1] или [2] для открытия в браузере, [Esc] для возврата", Style::default().fg(Color::DarkGray))),
    ];

    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(147, 51, 234)))
            .title(" Обратная Связь ")
    );
    f.render_widget(p, area);
}

fn render_popup_notice(f: &mut ratatui::Frame, area: Rect, msg: &str, color: Color) {
    let popup_area = Rect {
        x: area.width / 4,
        y: area.height / 2 - 2,
        width: area.width / 2,
        height: 5,
    };

    f.render_widget(Clear, popup_area);

    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(msg, Style::default().fg(color).add_modifier(Modifier::BOLD))),
    ])
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color))
            .title(" Уведомление ")
    );

    f.render_widget(p, popup_area);
}
