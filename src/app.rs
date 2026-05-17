//! Iced application: state, messages, update, view, subscriptions.
//!
//! The view is intentionally simple: a list of connections, a status line,
//! and two action buttons. Every action also has a keybind, the buttons are
//! there for discoverability and mouse users.

use std::path::PathBuf;

use iced::event::{self, Status};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Color, Element, Event, Length, Subscription, Task, Theme};

use crate::vpn::{self, Connection, ConnectionState, Snapshot, VpnKind};

#[derive(Debug, Default)]
pub struct App {
    snapshot: Snapshot,
    selected: usize,
    /// Name of the connection currently being acted on (activate/deactivate).
    pending: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    SnapshotReady(Result<Snapshot, String>),
    SelectPrev,
    SelectNext,
    SelectIndex(usize),
    ActivateSelected,
    DisconnectAll,
    StartImport,
    FileChosen(Option<PathBuf>),
    OperationDone(Result<String, String>),

    Quit,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        (Self::default(), Task::done(Message::Refresh))
    }

    pub fn title(&self) -> String {
        "byt".to_owned()
    }

    pub fn theme(&self) -> Theme {
        Theme::TokyoNightStorm
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => Task::perform(
                async { vpn::snapshot().await.map_err(|e| e.to_string()) },
                Message::SnapshotReady,
            ),

            Message::SnapshotReady(Ok(snap)) => {
                if !snap.connections.is_empty() && self.selected >= snap.connections.len() {
                    self.selected = snap.connections.len() - 1;
                }
                self.snapshot = snap;
                Task::none()
            }
            Message::SnapshotReady(Err(err)) => {
                self.status = Some(format!("error: {err}"));
                Task::none()
            }

            Message::SelectPrev => {
                self.selected = self.selected.saturating_sub(1);
                Task::none()
            }
            Message::SelectNext => {
                let max = self.snapshot.connections.len().saturating_sub(1);
                if self.selected < max {
                    self.selected += 1;
                }
                Task::none()
            }
            Message::SelectIndex(i) => {
                if i < self.snapshot.connections.len() {
                    self.selected = i;
                }
                Task::none()
            }

            Message::ActivateSelected => {
                if self.pending.is_some() {
                    return Task::none();
                }
                let Some(conn) = self.snapshot.connections.get(self.selected) else {
                    return Task::none();
                };
                if conn.state == ConnectionState::Unavailable {
                    self.status = Some(format!("{} is unavailable", conn.name));
                    return Task::none();
                }
                self.pending = Some(conn.name.clone());
                let conn = conn.clone();
                Task::perform(
                    async move {
                        let name = conn.name.clone();
                        vpn::activate_exclusive(&conn)
                            .await
                            .map(|()| name)
                            .map_err(|e| e.to_string())
                    },
                    Message::OperationDone,
                )
            }

            Message::DisconnectAll => {
                if self.pending.is_some() {
                    return Task::none();
                }
                self.pending = Some(String::new());
                Task::perform(
                    async {
                        vpn::disconnect_all()
                            .await
                            .map(|()| "all disconnected".to_owned())
                            .map_err(|e| e.to_string())
                    },
                    Message::OperationDone,
                )
            }

            Message::StartImport => {
                if self.pending.is_some() {
                    return Task::none();
                }
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import VPN configuration")
                            .add_filter("VPN configs", &["conf", "ovpn"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::FileChosen,
                )
            }

            Message::FileChosen(None) => Task::none(),
            Message::FileChosen(Some(path)) => {
                self.pending = Some(format!("importing {}", path.display()));
                Task::perform(
                    async move {
                        let kind = vpn::detect_config_kind(&path).map_err(|e| e.to_string())?;
                        let suggested = match kind {
                            vpn::ConfigKind::WireGuard => vpn::wireguard::parse_conf(&path)
                                .map_err(|e| e.to_string())?
                                .suggested_name(),
                            vpn::ConfigKind::OpenVpn => vpn::openvpn::parse_conf(&path)
                                .map_err(|e| e.to_string())?
                                .suggested_name(),
                        };
                        vpn::import::import(kind, &path, &suggested)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(format!("imported `{suggested}`"))
                    },
                    Message::OperationDone,
                )
            }

            Message::OperationDone(result) => {
                self.pending = None;
                match result {
                    Ok(msg) => self.status = Some(msg),
                    Err(err) => self.status = Some(format!("error: {err}")),
                }
                Task::done(Message::Refresh)
            }

            Message::Quit => iced::exit(),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let header = row![
            text("byt").size(28),
            Space::new().width(Length::Fill),
            action_button("Import (i)", Message::StartImport, self.pending.is_some()),
            action_button(
                "Disconnect all (d)",
                Message::DisconnectAll,
                self.pending.is_some()
            ),
            action_button("Refresh (r)", Message::Refresh, false),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .padding(12);

        let body: Element<'_, Message> = if self.snapshot.connections.is_empty() {
            container(text("No VPN connections found.").size(16))
                .center(Length::Fill)
                .into()
        } else {
            scrollable(column(self.snapshot.connections.iter().enumerate().map(
                |(i, c)| {
                    connection_row(
                        c,
                        i,
                        i == self.selected,
                        self.pending.as_deref() == Some(c.name.as_str()),
                    )
                },
            )))
            .height(Length::Fill)
            .into()
        };

        let hint = self.status.clone().unwrap_or_else(|| {
            "↑/↓ or j/k select • Enter connect • d disconnect all • i import • r refresh • q quit"
                .to_owned()
        });

        let footer = container(text(hint).size(13))
            .padding(10)
            .width(Length::Fill);

        container(column![header, body, footer].spacing(0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([keyboard_subscription(), Subscription::run(change_stream)])
    }
}

// subscriptions
fn keyboard_subscription() -> Subscription<Message> {
    event::listen_with(|event, status, _id| {
        if status == Status::Captured {
            return None;
        }
        if let Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event {
            handle_key(key, modifiers)
        } else {
            None
        }
    })
}

fn handle_key(key: Key, modifiers: Modifiers) -> Option<Message> {
    match key.as_ref() {
        Key::Named(Named::ArrowUp) => Some(Message::SelectPrev),
        Key::Named(Named::ArrowDown) => Some(Message::SelectNext),
        Key::Named(Named::Enter) => Some(Message::ActivateSelected),
        Key::Named(Named::Escape) => Some(Message::Quit),
        Key::Character(c) => match c {
            "k" => Some(Message::SelectPrev),
            "j" => Some(Message::SelectNext),
            "d" => Some(Message::DisconnectAll),
            "i" => Some(Message::StartImport),
            "r" => Some(Message::Refresh),
            "q" => Some(Message::Quit),
            "c" if modifiers.control() => Some(Message::Quit),
            _ => None,
        },
        _ => None,
    }
}

/// Background stream emitting `Refresh` whenever NM or systemd signal a change.
fn change_stream() -> impl futures::Stream<Item = Message> {
    use futures::SinkExt;
    use futures::StreamExt;

    iced::stream::channel(16, async |mut out| {
        let mut changes = match vpn::nm::changes().await {
            Ok(s) => s,
            Err(err) => {
                tracing::error!(?err, "failed to subscribe to NetworkManager signals");
                return;
            }
        };
        while changes.next().await.is_some() {
            if out.send(Message::Refresh).await.is_err() {
                break;
            }
        }
    })
}

//  widgets
fn action_button<'a>(label: &'a str, msg: Message, disabled: bool) -> Element<'a, Message> {
    let b = button(text(label).size(13)).padding([6, 12]);
    if disabled {
        b.into()
    } else {
        b.on_press(msg).into()
    }
}

fn connection_row<'a>(
    c: &'a Connection,
    index: usize,
    selected: bool,
    pending: bool,
) -> Element<'a, Message> {
    let (mark, mark_color) = match c.state {
        ConnectionState::Active => ("●", Color::from_rgb(0.35, 0.85, 0.45)),
        ConnectionState::Inactive => ("○", Color::from_rgb(0.55, 0.55, 0.55)),
        ConnectionState::Unavailable => ("✕", Color::from_rgb(0.85, 0.4, 0.4)),
    };

    let detail = c
        .detail
        .as_deref()
        .map(|d| format!("{}  ·  {d}", kind_label(c.kind)))
        .unwrap_or_else(|| kind_label(c.kind).to_owned());

    let pending_marker: Element<'_, Message> = if pending {
        text("…").size(20).into()
    } else {
        Space::new().into()
    };

    let inner = row![
        text(mark).color(mark_color).size(22),
        column![text(&c.name).size(16), text(detail).size(12)].spacing(2),
        Space::new().width(Length::Fill),
        pending_marker,
    ]
    .spacing(12)
    .padding(10)
    .align_y(iced::Alignment::Center);

    container(
        button(inner)
            .on_press(Message::SelectIndex(index))
            .style(move |theme: &Theme, status| row_button_style(theme, status, selected))
            .width(Length::Fill),
    )
    .into()
}

fn kind_label(k: VpnKind) -> &'static str {
    match k {
        VpnKind::Tailscale => "Tailscale",
        VpnKind::WireGuard => "WireGuard",
        VpnKind::OpenVpn => "OpenVPN",
    }
}

fn row_button_style(theme: &Theme, status: button::Status, selected: bool) -> button::Style {
    let palette = theme.extended_palette();
    let base = if selected {
        palette.background.weak.color
    } else {
        palette.background.base.color
    };
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => palette.background.weak.color,
        _ => base,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: palette.background.base.text,
        border: iced::Border::default(),
        ..button::Style::default()
    }
}
