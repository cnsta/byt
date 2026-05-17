//! Rendering. Intentionally minimal — the user is familiar with ratatui and
//! will want to design the UI themselves. This is enough to verify the wiring.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::vpn::ConnectionState;

pub fn render(app: &App, frame: &mut Frame<'_>) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(frame.area());

    // Header
    let header =
        Paragraph::new("byt — select a VPN [↑/↓ move • enter connect • d disconnect • q quit]")
            .block(Block::default().borders(Borders::ALL).title(" byt "));
    frame.render_widget(header, chunks[0]);

    // List
    let items: Vec<ListItem> = app
        .snapshot
        .connections
        .iter()
        .map(|c| {
            let (mark, color) = match c.state {
                ConnectionState::Active => ("●", Color::Green),
                ConnectionState::Inactive => ("○", Color::DarkGray),
                ConnectionState::Unavailable => ("✕", Color::Red),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {mark} "), Style::default().fg(color)),
                Span::raw(format!("{:10} ", c.kind.as_str())),
                Span::styled(&c.name, Style::default().add_modifier(Modifier::BOLD)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Connections "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(list, chunks[1], &mut state);

    // Status line
    let status = app.status_line.as_deref().unwrap_or("ready");
    frame.render_widget(Paragraph::new(status), chunks[2]);
}
