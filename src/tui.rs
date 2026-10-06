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
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::io;
use std::time::Duration;

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

fn tui_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let mut sys = sysinfo::System::new_all();

    loop {
        sys.refresh_all();
        let cfg = load_config();

        #[cfg(feature = "hermes")]
        let hermes = discover_hermes();

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(7),
                    Constraint::Length(8),
                    Constraint::Min(5),
                    Constraint::Length(3),
                ])
                .split(f.area());

            // Header Banner
            let banner_text = vec![
                Line::from(vec![
                    Span::styled("⚡ DDB (Dirom Device Bridge) ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                    Span::styled("v1.0.0 [Rust Native Monolith]", Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::raw("Суверенный агент устройств Цитадели | Архитектура: "),
                    Span::styled(format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH), Style::default().fg(Color::Yellow)),
                ]),
                Line::from(vec![
                    Span::raw("Хост: "),
                    Span::styled(&cfg.host_name, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(" | Сервер: "),
                    Span::styled("http://0.0.0.0:8790", Style::default().fg(Color::White)),
                ]),
            ];
            let banner = Paragraph::new(banner_text)
                .block(Block::default().borders(Borders::ALL).title("Статус Агента"));
            f.render_widget(banner, chunks[0]);

            // Hermes & Environment Status
            #[cfg(feature = "hermes")]
            let hermes_status = if hermes.found {
                format!(
                    "Подключен (pid={}, port={:?})",
                    hermes.pid.unwrap_or(0),
                    hermes.port
                )
            } else {
                "Не найден".to_string()
            };
            #[cfg(not(feature = "hermes"))]
            let hermes_status = "Отключено (публичный билд)".to_string();

            let env_text = vec![
                Line::from(vec![
                    Span::styled("Hermes Desktop: ", Style::default().fg(Color::Cyan)),
                    #[cfg(feature = "hermes")]
                    Span::styled(hermes_status, if hermes.found { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
                    #[cfg(not(feature = "hermes"))]
                    Span::styled(hermes_status, Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(vec![
                    Span::styled("Память (RAM):   ", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{:.1} GB / {:.1} GB свободно", 
                        sys.available_memory() as f64 / 1024.0 / 1024.0 / 1024.0,
                        sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Туннель:        ", Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{} (LAN direct mode)", cfg.tunnel_type)),
                ]),
            ];
            let env_widget = Paragraph::new(env_text)
                .block(Block::default().borders(Borders::ALL).title("Интеграции & Ресурсы"));
            f.render_widget(env_widget, chunks[1]);

            // Permissions Matrix
            let p = &cfg.permissions;
            let perms_text = vec![
                Line::from(vec![
                    Span::raw("  [1] Шелл команды (exec):       "),
                    Span::styled(if p.exec { "[✓ РАЗРЕШЕНО]" } else { "[✗ ЗАПРЕЩЕНО]" }, if p.exec { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
                ]),
                Line::from(vec![
                    Span::raw("  [2] Hermes Desktop A2A:        "),
                    Span::styled(if p.hermes { "[✓ РАЗРЕШЕНО]" } else { "[✗ ЗАПРЕЩЕНО]" }, if p.hermes { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
                ]),
                Line::from(vec![
                    Span::raw("  [3] Скриншот экрана (screen):  "),
                    Span::styled(if p.screen { "[✓ РАЗРЕШЕНО]" } else { "[✗ ЗАПРЕЩЕНО]" }, if p.screen { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
                ]),
                Line::from(vec![
                    Span::raw("  [4] Файловая система (read):   "),
                    Span::styled(if p.fs.read { "[✓ РАЗРЕШЕНО]" } else { "[✗ ЗАПРЕЩЕНО]" }, if p.fs.read { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
                ]),
                Line::from(vec![
                    Span::raw("  [5] Файловая система (write):  "),
                    Span::styled(if p.fs.write { "[✓ РАЗРЕШЕНО]" } else { "[✗ ЗАПРЕЩЕНО]" }, if p.fs.write { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
                ]),
            ];
            let perms_widget = Paragraph::new(perms_text)
                .block(Block::default().borders(Borders::ALL).title("Матрица Разрешений (Permissions Matrix)"));
            f.render_widget(perms_widget, chunks[2]);

            // Footer
            let footer = Paragraph::new("Нажмите 'q' или 'Esc' для выхода в фон | 'r' для рефреша")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(footer, chunks[3]);
        })?;

        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
