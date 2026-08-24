//! Iced application: state, messages, update, view, subscriptions.
//!
//! The view is intentionally simple: a list of connections, a status line,
//! and two action buttons. Every action also has a keybind, the buttons are
//! there for discoverability and mouse users.

use std::path::PathBuf;

use crate::config::{self, Config};
use iced::event::{self, Status};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::toggler;
use iced::widget::{Space, button, column, container, row, scrollable, stack, svg, text};
use iced::{Color, Element, Event, Length, Subscription, Task, Theme};

use crate::vpn::{self, Connection, ConnectionState, Snapshot, VpnKind};

const LOGO_BYTES: &[u8] = include_bytes!("../assets/byt.svg");
const ICON_IMPORT: &[u8] = include_bytes!("../assets/import.svg");
const ICON_DISCONNECT: &[u8] = include_bytes!("../assets/disconnect.svg");
const ICON_REFRESH: &[u8] = include_bytes!("../assets/refresh.svg");

fn scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("connections")
}

#[derive(Debug, Default)]
pub struct App {
    snapshot: Snapshot,
    selected: usize,
    pending: Option<String>,
    status: Option<String>,
    config: Config,
    confirming_delete: Option<Connection>,
    scroll_offset: scrollable::AbsoluteOffset,
    scroll_viewport_height: f32,
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    SnapshotReady(Result<Snapshot, String>),
    SelectPrev,
    SelectNext,
    SelectIndex(usize),
    ActivateSelected,
    Disconnect,
    StartImport,
    FilesChosen(Vec<PathBuf>),
    OperationDone(Result<String, String>),
    StartDelete,
    ConfirmDelete,
    CancelDelete,
    Scrolled(scrollable::Viewport),
    ToggleQuitOnSwitch(bool),
    ActivationDone(Result<String, String>),
    Quit,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let mut app = Self::default();
        app.config = config::load();
        (app, Task::done(Message::Refresh))
    }

    pub fn title(&self) -> String {
        "byt".to_owned()
    }

    pub fn theme(&self) -> Theme {
        Theme::Ferra
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

            Message::ToggleQuitOnSwitch(value) => {
                self.config.quit_on_switch = value;
                config::save(&self.config);
                Task::none()
            }

            Message::SelectPrev => {
                self.selected = self.selected.saturating_sub(1);
                self.ensure_selected_visible()
            }

            Message::SelectNext => {
                let max = self.snapshot.connections.len().saturating_sub(1);
                if self.selected < max {
                    self.selected += 1;
                }
                self.ensure_selected_visible()
            }

            Message::SelectIndex(i) => {
                if i < self.snapshot.connections.len() {
                    self.selected = i;
                }
                self.ensure_selected_visible()
            }

            Message::ActivateSelected => {
                // The delete dialog owns Enter while it's open, regardless of
                // what's selected underneath it.
                if self.confirming_delete.is_some() {
                    return Task::done(Message::ConfirmDelete);
                }
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
                        vpn::toggle_exclusive(&conn)
                            .await
                            .map(|toggled| match toggled {
                                vpn::Toggled::Up => format!("connected `{name}`"),
                                vpn::Toggled::Down => format!("disconnected `{name}`"),
                            })
                            .map_err(|e| e.to_string())
                    },
                    Message::ActivationDone,
                )
            }

            Message::ActivationDone(result) => {
                self.pending = None;
                match result {
                    Ok(msg) => {
                        self.status = Some(msg);
                        if self.config.quit_on_switch {
                            iced::exit()
                        } else {
                            Task::done(Message::Refresh)
                        }
                    }
                    Err(err) => {
                        self.status = Some(format!("error: {err}"));
                        Task::done(Message::Refresh)
                    }
                }
            }

            Message::Disconnect => {
                if self.pending.is_some() {
                    return Task::none();
                }
                self.pending = Some(String::new());
                Task::perform(
                    async {
                        vpn::disconnect()
                            .await
                            .map(|()| "disconnected".to_owned())
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
                            .set_title("Import VPN configurations")
                            .add_filter("VPN configs", &["conf", "ovpn"])
                            .add_filter("All files", &["*"])
                            .pick_files()
                            .await
                            .map(|handles| handles.iter().map(|h| h.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::FilesChosen,
                )
            }

            Message::FilesChosen(paths) => {
                if paths.is_empty() {
                    return Task::none();
                }
                self.pending = Some(match paths.as_slice() {
                    [one] => format!("importing {}", one.display()),
                    many => format!("importing {} configs", many.len()),
                });
                Task::perform(import_files(paths), Message::OperationDone)
            }

            Message::OperationDone(result) => {
                self.pending = None;
                match result {
                    Ok(msg) => self.status = Some(msg),
                    Err(err) => self.status = Some(format!("error: {err}")),
                }
                Task::done(Message::Refresh)
            }

            Message::StartDelete => {
                if self.pending.is_some() || self.confirming_delete.is_some() {
                    return Task::none();
                }
                let Some(conn) = self.snapshot.connections.get(self.selected).cloned() else {
                    return Task::none();
                };
                if conn.kind == VpnKind::Tailscale {
                    self.status = Some("Tailscale is not a NetworkManager connection".into());
                    return Task::none();
                }
                self.confirming_delete = Some(conn);
                Task::none()
            }

            Message::ConfirmDelete => {
                let Some(target) = self.confirming_delete.take() else {
                    return Task::none();
                };
                let name = target.name.clone();
                self.pending = Some(format!("deleting {name}"));
                Task::perform(
                    async move {
                        vpn::delete(&target)
                            .await
                            .map(|()| format!("deleted `{name}`"))
                            .map_err(|e| e.to_string())
                    },
                    Message::OperationDone,
                )
            }

            Message::CancelDelete => {
                self.confirming_delete = None;
                Task::none()
            }

            Message::Scrolled(viewport) => {
                self.scroll_offset = viewport.absolute_offset();
                self.scroll_viewport_height = viewport.bounds().height;
                Task::none()
            }

            Message::Quit => {
                if self.confirming_delete.is_some() {
                    self.confirming_delete = None;
                    return Task::none();
                }
                iced::exit()
            }
        }
    }

    fn ensure_selected_visible(&self) -> Task<Message> {
        let total = self.snapshot.connections.len();
        if total <= 1 {
            return Task::none();
        }
        let ratio = (self.selected as f32 / (total - 1) as f32).clamp(0.0, 1.0);
        iced::widget::operation::snap_to(
            scroll_id(),
            iced::widget::scrollable::RelativeOffset { x: 0.0, y: ratio },
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        let is_dialog_open = self.confirming_delete.is_some();
        let is_busy = self.pending.is_some() || is_dialog_open;

        let header = row![
            svg(svg::Handle::from_memory(LOGO_BYTES))
                .width(Length::Fixed(56.0))
                .height(Length::Fixed(30.0)),
            Space::new().width(Length::Fill),
            toggler(self.config.quit_on_switch)
                .label("Quit on Switch™")
                .text_size(12)
                .size(16)
                .on_toggle(Message::ToggleQuitOnSwitch),
            Space::new().width(Length::Fixed(12.0)),
            icon_button(ICON_IMPORT, Message::StartImport, is_busy),
            icon_button(ICON_DISCONNECT, Message::Disconnect, is_busy),
            icon_button(ICON_REFRESH, Message::Refresh, false),
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
            .id(scroll_id())
            .on_scroll(Message::Scrolled)
            .height(Length::Fill)
            .into()
        };

        let hint = self.status.clone().unwrap_or_else(|| {
            if is_dialog_open {
                "y/Enter confirm • n/Esc cancel".to_owned()
            } else {
                "↑/↓ select • (x)disconnect • (i)mport • (d)elete • (r)efresh • (q)uit".to_owned()
            }
        });

        let footer = container(text(hint).size(13))
            .padding(10)
            .width(Length::Fill);

        let main: Element<'_, Message> = container(column![header, body, footer].spacing(0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        if let Some(target) = &self.confirming_delete {
            let dialog = confirm_delete_view(&target.name);
            stack![main, dialog].into()
        } else {
            main
        }
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

/// Import a batch of config files sequentially, skipping names that already
/// exist as NetworkManager connections (`nmcli connection import` would
/// otherwise happily create duplicates with the same id).
async fn import_files(paths: Vec<PathBuf>) -> Result<String, String> {
    let mut existing: std::collections::HashSet<String> = match vpn::nm::list().await {
        Ok(list) => list.into_iter().map(|c| c.name).collect(),
        Err(err) => {
            tracing::warn!(%err, "could not list existing connections; skip-check disabled");
            std::collections::HashSet::new()
        }
    };

    let total = paths.len();
    let mut imported = 0_usize;
    let mut skipped = 0_usize;
    let mut failures: Vec<String> = Vec::new();
    let mut last_name = String::new();

    for path in paths {
        let label = path
            .file_name()
            .and_then(|s| s.to_str())
            .map_or_else(|| path.display().to_string(), ToOwned::to_owned);

        match vpn::import::preview_name(&path) {
            Ok((_, name)) if existing.contains(&name) => skipped += 1,
            Ok((kind, name)) => match vpn::import::import(kind, &path, &name).await {
                Ok(()) => {
                    imported += 1;
                    // Also catches duplicate names *within* the batch.
                    existing.insert(name.clone());
                    last_name = name;
                }
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "import failed");
                    failures.push(first_line(&format!("{label}: {err}")));
                }
            },
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "could not parse config");
                failures.push(first_line(&format!("{label}: {err}")));
            }
        }
    }

    let mut parts = Vec::new();
    if imported > 0 {
        parts.push(if imported == 1 && total == 1 {
            format!("imported `{last_name}`")
        } else {
            format!("imported {imported}")
        });
    }
    if skipped > 0 {
        parts.push(format!("skipped {skipped} existing"));
    }
    if !failures.is_empty() {
        parts.push(format!("{} failed — {}", failures.len(), failures[0]));
    }
    if parts.is_empty() {
        parts.push("nothing to import".to_owned());
    }

    let msg = parts.join(" • ");
    if failures.is_empty() {
        Ok(msg)
    } else {
        Err(msg)
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_owned()
}

fn handle_key(key: Key, modifiers: Modifiers) -> Option<Message> {
    match key.as_ref() {
        Key::Named(Named::ArrowUp) => Some(Message::SelectPrev),
        Key::Named(Named::ArrowDown) => Some(Message::SelectNext),
        Key::Named(Named::Enter) => Some(Message::ActivateSelected),
        Key::Named(Named::Escape) => Some(Message::Quit),
        Key::Character(c) => match c {
            "1" => Some(Message::SelectIndex(0)),
            "2" => Some(Message::SelectIndex(1)),
            "3" => Some(Message::SelectIndex(2)),
            "4" => Some(Message::SelectIndex(3)),
            "5" => Some(Message::SelectIndex(4)),
            "6" => Some(Message::SelectIndex(5)),
            "7" => Some(Message::SelectIndex(6)),
            "8" => Some(Message::SelectIndex(7)),
            "9" => Some(Message::SelectIndex(8)),
            "k" => Some(Message::SelectPrev),
            "j" => Some(Message::SelectNext),
            "x" => Some(Message::Disconnect),
            "i" => Some(Message::StartImport),
            "r" => Some(Message::Refresh),
            "d" => Some(Message::StartDelete),
            "y" | "Y" => Some(Message::ConfirmDelete),
            "n" | "N" => Some(Message::CancelDelete),
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
fn icon_button<'a>(
    icon_bytes: &'static [u8],
    msg: Message,
    disabled: bool,
) -> Element<'a, Message> {
    let icon = svg(svg::Handle::from_memory(icon_bytes))
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .style(move |theme: &Theme, _status| {
            let mut color = theme.extended_palette().background.base.text;
            if disabled {
                color.a = 0.4;
            }
            svg::Style { color: Some(color) }
        });

    let b = button(container(icon).center(Length::Fill))
        .width(Length::Fixed(36.0))
        .height(Length::Fixed(36.0))
        .style(icon_button_style);

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

    let trailing: Element<'_, Message> = if pending {
        text("…").size(18).into()
    } else if index < 9 {
        text(format!("{}", index + 1))
            .size(11)
            .color(Color::from_rgb(0.5, 0.5, 0.5))
            .into()
    } else {
        Space::new().into()
    };

    let inner = row![
        text(mark).color(mark_color).size(16),
        column![text(&c.name).size(14), text(detail).size(11)].spacing(1),
        Space::new().width(Length::Fill),
        trailing,
    ]
    .spacing(10)
    .padding([8, 12])
    .align_y(iced::Alignment::Center);

    container(
        button(inner)
            .padding(0)
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

fn icon_button_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let (background, text_color) = match status {
        button::Status::Hovered => (
            Some(palette.background.weak.color.into()),
            palette.background.weak.text,
        ),
        button::Status::Pressed => (
            Some(palette.background.strong.color.into()),
            palette.background.strong.text,
        ),
        button::Status::Disabled => {
            let mut faded = palette.background.base.text;
            faded.a = 0.4;
            (None, faded)
        }
        button::Status::Active => (None, palette.background.base.text),
    };

    button::Style {
        background,
        text_color,
        border: iced::Border {
            radius: f32::INFINITY.into(),
            ..iced::Border::default()
        },
        ..button::Style::default()
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

fn confirm_delete_view<'a>(name: &'a str) -> Element<'a, Message> {
    let card = container(
        column![
            text("Delete connection?").size(20),
            text(format!("\"{name}\" will be removed permanently.")).size(14),
            row![
                button(text("Yes").size(14))
                    .padding([8, 16])
                    .on_press(Message::ConfirmDelete),
                button(text("No").size(14))
                    .padding([8, 16])
                    .on_press(Message::CancelDelete),
            ]
            .spacing(12),
        ]
        .spacing(16)
        .align_x(iced::Alignment::Center),
    )
    .padding(24)
    .max_width(360)
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(palette.background.base.color.into()),
            border: iced::Border {
                radius: 8.0.into(),
                width: 1.0,
                color: palette.background.strong.color,
            },
            ..container::Style::default()
        }
    });

    // Backdrop: dims the underlying view and absorbs stray clicks. Clicking
    // outside the card cancels — standard modal UX.
    iced::widget::mouse_area(
        container(card)
            .center(Length::Fill)
            .style(|_| container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                ..container::Style::default()
            }),
    )
    .on_press(Message::CancelDelete)
    .into()
}
