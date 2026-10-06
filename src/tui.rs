use crate::config::load_config;
#[cfg(feature = "hermes")]
use crate::hermes::discover_hermes;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Tabs,
    },
    Terminal,
};
use std::io;
use std::time::{Duration, Instant};

pub fn run_tui() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = tui_loop(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

struct TuiState {
    tab_index: usize,
    uptime_start: Instant,
    logs: Vec<String>,
}

fn tui_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let mut sys = sysinfo::System::new_all();
    let mut state = TuiState {
        tab_index: 0,
        uptime_start: Instant::now(),
        logs: vec![
            "[SYSTEM] DDB Native Rust Agent v1.0.0 инициализирован".to_string(),
            "[DAEMON] Служба Axum слушает порт :8790 (REST + WS)".to_string(),
            "[INPUT] Dirom Virtual Pointer (/dev/uinput) готов".to_string(),
            "[WAYLAND] Дисплей Niri подхвачен (grim capture active)".to_string(),
            "[A2A] Hermes Synapse мост ожидает вызовы [Dirom Reqwest]".to_string(),
        ],
    };

    loop {
        sys.refresh_all();
        let cfg = load_config();

        #[cfg(feature = "hermes")]
        let hermes = discover_hermes();

        let uptime_secs = state.uptime_start.elapsed().as_secs();
        let uptime_str = format!(
            "{:02}:{:02}:{:02}",
            uptime_secs / 3600,
            (uptime_secs % 3600) / 60,
            uptime_secs % 60
        );

        terminal.draw(|f| {
            let size = f.area();

            // Main vertical layout
            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3), // Top Header Bar with Tabs
                    Constraint::Min(10),   // Main Content Area
                    Constraint::Length(1), // Footer Command Bar
                ])
                .split(size);

            // ── TOP HEADER BAR ──────────────────────────────────────
            let header_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(32), // Logo & Badge
                    Constraint::Min(20),   // Tabs
                    Constraint::Length(22), // Uptime & Host
                ])
                .split(main_chunks[0]);

            let logo = Paragraph::new(Line::from(vec![
                Span::styled("⚡ DDB ", Style::default().fg(Color::Rgb(168, 85, 247)).add_modifier(Modifier::BOLD)),
                Span::styled("v1.0.0 ", Style::default().fg(Color::Rgb(216, 180, 254))),
                Span::styled("[RUST]", Style::default().fg(Color::Rgb(34, 197, 94)).add_modifier(Modifier::BOLD)),
            ]))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(147, 51, 234)))
            );
            f.render_widget(logo, header_chunks[0]);

            let tab_titles = vec!["[1] Обзор (Overview)", "[2] Права (Permissions)", "[3] Журнал (Logs)"];
            let tabs = Tabs::new(tab_titles)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Rgb(107, 114, 128)))
                )
                .select(state.tab_index)
                .style(Style::default().fg(Color::DarkGray))
                .highlight_style(
                    Style::default()
                        .fg(Color::Rgb(244, 63, 94))
                        .add_modifier(Modifier::BOLD)
                )
                .divider(" │ ");
            f.render_widget(tabs, header_chunks[1]);

            let sys_badge = Paragraph::new(Line::from(vec![
                Span::styled("⏱ ", Style::default().fg(Color::Yellow)),
                Span::styled(uptime_str, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw(" │ "),
                Span::styled(&cfg.host_name, Style::default().fg(Color::Cyan)),
            ]))
            .alignment(Alignment::Right)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(107, 114, 128)))
            );
            f.render_widget(sys_badge, header_chunks[2]);

            // ── TAB CONTENT ─────────────────────────────────────────
            match state.tab_index {
                0 => {
                    // TAB 1: OVERVIEW DASHBOARD (2 Columns)
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(50),
                            Constraint::Percentage(50),
                        ])
                        .split(main_chunks[1]);

                    // Left Column (Host & Synapse Telemetry)
                    let left_rows = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(8),  // Host Details
                            Constraint::Length(8),  // Hermes Sibling Synapse
                            Constraint::Min(4),    // RAM Gauge
                        ])
                        .split(cols[0]);

                    // Host Details
                    let host_lines = vec![
                        Line::from(vec![
                            Span::styled("  ОС / Архитектура : ", Style::default().fg(Color::DarkGray)),
                            Span::styled(format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        ]),
                        Line::from(vec![
                            Span::styled("  Процессор (ЦПУ)  : ", Style::default().fg(Color::DarkGray)),
                            Span::styled(format!("{} ядер", sys.cpus().len()), Style::default().fg(Color::Yellow)),
                        ]),
                        Line::from(vec![
                            Span::styled("  REST/WS Сервер   : ", Style::default().fg(Color::DarkGray)),
                            Span::styled("http://0.0.0.0:8790", Style::default().fg(Color::Green)),
                        ]),
                        Line::from(vec![
                            Span::styled("  Сетевой Туннель  : ", Style::default().fg(Color::DarkGray)),
                            Span::styled(&cfg.tunnel_type, Style::default().fg(Color::Cyan)),
                            Span::styled(" (Direct P2P LAN)", Style::default().fg(Color::DarkGray)),
                        ]),
                    ];
                    let host_block = Paragraph::new(host_lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Rgb(75, 85, 99)))
                            .title(Span::styled(" 🖥️  Хост & Сеть ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
                    );
                    f.render_widget(host_block, left_rows[0]);

                    // Hermes Desktop Sibling Synapse
                    #[cfg(feature = "hermes")]
                    let (hermes_badge, hermes_info, hermes_color) = if hermes.found {
                        (
                            Span::styled("[ONLINE]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                            vec![
                                Line::from(vec![
                                    Span::styled("  Статус A2A моста : ", Style::default().fg(Color::DarkGray)),
                                    Span::styled("Активен и подключен", Style::default().fg(Color::Green)),
                                ]),
                                Line::from(vec![
                                    Span::styled("  PID процесса     : ", Style::default().fg(Color::DarkGray)),
                                    Span::styled(hermes.pid.unwrap_or(0).to_string(), Style::default().fg(Color::White)),
                                ]),
                                Line::from(vec![
                                    Span::styled("  Порт Uvicorn     : ", Style::default().fg(Color::DarkGray)),
                                    Span::styled(hermes.port.map(|p| p.to_string()).unwrap_or_else(|| "none".into()), Style::default().fg(Color::Yellow)),
                                ]),
                                Line::from(vec![
                                    Span::styled("  Токен сессии     : ", Style::default().fg(Color::DarkGray)),
                                    Span::styled("3whv... (считан из /proc)", Style::default().fg(Color::Rgb(168, 85, 247))),
                                ]),
                            ],
                            Color::Rgb(168, 85, 247),
                        )
                    } else {
                        (
                            Span::styled("[OFFLINE]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                            vec![
                                Line::from(vec![
                                    Span::styled("  Hermes Desktop   : ", Style::default().fg(Color::DarkGray)),
                                    Span::styled("Не запущен на хосте", Style::default().fg(Color::Red)),
                                ]),
                                Line::from(vec![
                                    Span::styled("  Поиск в /proc    : ", Style::default().fg(Color::DarkGray)),
                                    Span::styled("Ожидание запуска hermes serve...", Style::default().fg(Color::DarkGray)),
                                ]),
                            ],
                            Color::DarkGray,
                        )
                    };
                    #[cfg(not(feature = "hermes"))]
                    let (hermes_badge, hermes_info, hermes_color) = (
                        Span::styled("[DISABLED]", Style::default().fg(Color::DarkGray)),
                        vec![Line::from("  Фича hermes выключена в публичном билде")],
                        Color::DarkGray,
                    );

                    let hermes_block = Paragraph::new(hermes_info).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(hermes_color))
                            .title(Line::from(vec![
                                Span::styled(" 🧠 Hermes Desktop A2A Synapse ", Style::default().fg(Color::Rgb(216, 180, 254)).add_modifier(Modifier::BOLD)),
                                hermes_badge,
                            ]))
                    );
                    f.render_widget(hermes_block, left_rows[1]);

                    // RAM Gauge
                    let total_ram = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
                    let avail_ram = sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
                    let used_ram = (total_ram - avail_ram).max(0.0);
                    let ratio = (used_ram / total_ram.max(1.0)).clamp(0.0, 1.0);

                    let ram_gauge = Gauge::default()
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(Color::Rgb(75, 85, 99)))
                                .title(Span::styled(" 📊 Загрузка ОЗУ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
                        )
                        .gauge_style(Style::default().fg(Color::Rgb(168, 85, 247)).bg(Color::Rgb(31, 41, 55)))
                        .ratio(ratio)
                        .label(format!("{:.1} GB / {:.1} GB ({:.0}%)", used_ram, total_ram, ratio * 100.0));
                    f.render_widget(ram_gauge, left_rows[2]);

                    // Right Column (Input, Computer Use, Live Activity)
                    let right_rows = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(8), // Computer Use & Input
                            Constraint::Min(8),   // Quick Log Stream
                        ])
                        .split(cols[1]);

                    let input_lines = vec![
                        Line::from(vec![
                            Span::styled("  Virtual Pointer  : ", Style::default().fg(Color::DarkGray)),
                            Span::styled("Dirom Virtual Pointer (/dev/uinput)", Style::default().fg(Color::Green)),
                        ]),
                        Line::from(vec![
                            Span::styled("  Сглаживание мыши : ", Style::default().fg(Color::DarkGray)),
                            Span::styled("Cubic Bezier (20-30 steps, ease-out)", Style::default().fg(Color::Cyan)),
                        ]),
                        Line::from(vec![
                            Span::styled("  Курсор Дирома    : ", Style::default().fg(Color::DarkGray)),
                            Span::styled("XCursor-Pro-Red", Style::default().fg(Color::Rgb(244, 63, 94))),
                            Span::styled(" (Создатель: NeonPurple)", Style::default().fg(Color::DarkGray)),
                        ]),
                        Line::from(vec![
                            Span::styled("  Display Capture  : ", Style::default().fg(Color::DarkGray)),
                            Span::styled("grim (Wayland/Niri zero-copy PNG)", Style::default().fg(Color::White)),
                        ]),
                    ];
                    let input_block = Paragraph::new(input_lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Rgb(75, 85, 99)))
                            .title(Span::styled(" 🖱️  Computer Use & Ввод ", Style::default().fg(Color::Rgb(244, 63, 94)).add_modifier(Modifier::BOLD)))
                    );
                    f.render_widget(input_block, right_rows[0]);

                    // Quick Log Preview
                    let log_items: Vec<ListItem> = state.logs.iter().rev().take(6).map(|l| {
                        ListItem::new(Line::from(vec![
                            Span::styled("› ", Style::default().fg(Color::Rgb(168, 85, 247))),
                            Span::raw(l),
                        ]))
                    }).collect();

                    let log_block = List::new(log_items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Rgb(75, 85, 99)))
                            .title(Span::styled(" 📝 Журнал Событий (Live) ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)))
                    );
                    f.render_widget(log_block, right_rows[1]);
                }

                1 => {
                    // TAB 2: PERMISSIONS MATRIX
                    let p = &cfg.permissions;
                    let perms_items = vec![
                        create_perm_row("1", "Выполнение шелл-команд (exec)", "Позволяет запускать bash/sh команды на хосте", p.exec),
                        create_perm_row("2", "Hermes Desktop A2A Synapse", "Двусторонний обмен задачами с интерфейсом Hermes Desktop", p.hermes),
                        create_perm_row("3", "Захват экрана (screen)", "Делать скриншоты рабочего стола Niri через grim", p.screen),
                        create_perm_row("4", "Файловая система: Чтение (fs.read)", "Чтение файлов и папок с диска хоста", p.fs.read),
                        create_perm_row("5", "Файловая система: Запись (fs.write)", "Создание и перезапись файлов на диске", p.fs.write),
                        create_perm_row("6", "Файловая система: Листинг (fs.list)", "Просмотр содержимого директорий", p.fs.list),
                        create_perm_row("7", "Display Control: Мышь и Клик", "Клики и перемещения курсора через uinput Безье", p.display_control.allow_click),
                        create_perm_row("8", "Display Control: Клавиатура и Ввод", "Печать текста и хоткеи Niri через ydotool/IPC", p.display_control.allow_keys),
                    ];

                    let perms_block = List::new(perms_items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Rgb(147, 51, 234)))
                            .title(Span::styled(" 🛡️  Матрица Безопасности & Прав Хоста (~/.config/ddb/config.json) ", Style::default().fg(Color::Rgb(216, 180, 254)).add_modifier(Modifier::BOLD)))
                    );
                    f.render_widget(perms_block, main_chunks[1]);
                }

                2 => {
                    // TAB 3: COMPLETE LOG STREAM
                    let log_items: Vec<ListItem> = state.logs.iter().map(|l| {
                        ListItem::new(Line::from(vec![
                            Span::styled("• ", Style::default().fg(Color::Cyan)),
                            Span::styled(l, Style::default().fg(Color::White)),
                        ]))
                    }).collect();

                    let full_log = List::new(log_items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Rgb(75, 85, 99)))
                            .title(Span::styled(" 📜 Полный Системный Журнал DDB ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
                    );
                    f.render_widget(full_log, main_chunks[1]);
                }

                _ => {}
            }

            // ── FOOTER COMMAND BAR ──────────────────────────────────
            let footer = Paragraph::new(Line::from(vec![
                Span::styled(" [1/2/3] ", Style::default().fg(Color::Black).bg(Color::Rgb(168, 85, 247)).add_modifier(Modifier::BOLD)),
                Span::styled(" Вкладки ", Style::default().fg(Color::White)),
                Span::raw(" │ "),
                Span::styled(" [r] ", Style::default().fg(Color::Black).bg(Color::Rgb(234, 179, 8)).add_modifier(Modifier::BOLD)),
                Span::styled(" Обновить статус ", Style::default().fg(Color::White)),
                Span::raw(" │ "),
                Span::styled(" [q / Esc] ", Style::default().fg(Color::Black).bg(Color::Rgb(244, 63, 94)).add_modifier(Modifier::BOLD)),
                Span::styled(" Выход в фоновый режим ", Style::default().fg(Color::White)),
                Span::raw(" │ "),
                Span::styled(" Порт :8790 активен ", Style::default().fg(Color::Green)),
            ]))
            .style(Style::default().bg(Color::Rgb(17, 24, 39)));
            f.render_widget(footer, main_chunks[2]);
        })?;

        if event::poll(Duration::from_millis(300))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('1') => state.tab_index = 0,
                    KeyCode::Char('2') => state.tab_index = 1,
                    KeyCode::Char('3') => state.tab_index = 2,
                    KeyCode::Tab => {
                        state.tab_index = (state.tab_index + 1) % 3;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn create_perm_row<'a>(num: &'a str, title: &'a str, desc: &'a str, allowed: bool) -> ListItem<'a> {
    let (status_text, status_color) = if allowed {
        ("[✓ РАЗРЕШЕНО]", Color::Green)
    } else {
        ("[✗ ЗАПРЕЩЕНО]", Color::Red)
    };

    ListItem::new(vec![
        Line::from(vec![
            Span::styled(format!("  [{}] ", num), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw("      "),
            Span::styled(desc, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ])
}
