//! Application state and the update loop.
//!
//! Follows a unidirectional "elm-ish" pattern:
//! - `App` holds state
//! - `Action` describes a discrete change
//! - `handle_event` translates inputs into actions
//! - `update` applies actions and may spawn async work
//!
//! Keeping these layers separate is what lets us add tests later without a
//! real terminal — you can drive the app with a vector of events.

use std::time::Duration;

use color_eyre::eyre::WrapErr;
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::UnboundedSender;

use crate::event::{Event, EventLoop};
use crate::ui;
use crate::vpn::{self, Connection, Snapshot};

const TICK_RATE: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
pub struct App {
    pub snapshot: Snapshot,
    pub selected: usize,
    pub status_line: Option<String>,
    pub should_quit: bool,
}

/// Discrete state transitions. Background tasks send these back to the loop.
#[derive(Debug)]
pub enum Action {
    SnapshotReady(Snapshot),
    OperationFinished { message: String },
    OperationFailed { message: String },
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> color_eyre::Result<()> {
        let mut events = EventLoop::new(TICK_RATE);

        // Kick off an initial snapshot.
        spawn_refresh(events.tx.clone());

        while !self.should_quit {
            terminal
                .draw(|frame| ui::render(self, frame))
                .wrap_err("draw failed")?;

            let Some(event) = events.next().await else {
                break;
            };

            if let Some(action) = self.handle_event(event, &events.tx) {
                self.update(action);
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event, tx: &UnboundedSender<Event>) -> Option<Action> {
        match event {
            Event::Term(crossterm::event::Event::Key(key))
                if key.kind == KeyEventKind::Press =>
            {
                self.handle_key(key, tx)
            }
            Event::Tick => {
                spawn_refresh(tx.clone());
                None
            }
            Event::Action(action) => Some(action),
            Event::Quit => {
                self.should_quit = true;
                None
            }
            _ => None,
        }
    }

    fn handle_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        tx: &UnboundedSender<Event>,
    ) -> Option<Action> {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Char('q')) => {
                self.should_quit = true;
            }
            (_, KeyCode::Down | KeyCode::Char('j')) => {
                self.move_selection(1);
            }
            (_, KeyCode::Up | KeyCode::Char('k')) => {
                self.move_selection(-1);
            }
            (_, KeyCode::Enter) => {
                if let Some(conn) = self.snapshot.connections.get(self.selected).cloned() {
                    spawn_activate(conn, tx.clone());
                }
            }
            (_, KeyCode::Char('d')) => {
                spawn_disconnect_all(tx.clone());
            }
            _ => {}
        }
        None
    }

    fn move_selection(&mut self, delta: i32) {
        let len = self.snapshot.connections.len() as i32;
        if len == 0 {
            return;
        }
        let new = (self.selected as i32 + delta).rem_euclid(len);
        self.selected = new as usize;
    }

    fn update(&mut self, action: Action) {
        match action {
            Action::SnapshotReady(snap) => {
                if self.selected >= snap.connections.len() {
                    self.selected = snap.connections.len().saturating_sub(1);
                }
                self.snapshot = snap;
            }
            Action::OperationFinished { message } => {
                self.status_line = Some(message);
            }
            Action::OperationFailed { message } => {
                self.status_line = Some(format!("error: {message}"));
            }
        }
    }
}

fn spawn_refresh(tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        match vpn::snapshot().await {
            Ok(snap) => {
                let _ = tx.send(Event::Action(Action::SnapshotReady(snap)));
            }
            Err(err) => {
                let _ = tx.send(Event::Action(Action::OperationFailed {
                    message: err.to_string(),
                }));
            }
        }
    });
}

fn spawn_activate(connection: Connection, tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        let name = connection.name.clone();
        match vpn::activate_exclusive(&connection).await {
            Ok(()) => {
                let _ = tx.send(Event::Action(Action::OperationFinished {
                    message: format!("connected to {name}"),
                }));
            }
            Err(err) => {
                let _ = tx.send(Event::Action(Action::OperationFailed {
                    message: err.to_string(),
                }));
            }
        }
    });
}

fn spawn_disconnect_all(tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        match vpn::disconnect_all().await {
            Ok(()) => {
                let _ = tx.send(Event::Action(Action::OperationFinished {
                    message: "all VPNs disconnected".into(),
                }));
            }
            Err(err) => {
                let _ = tx.send(Event::Action(Action::OperationFailed {
                    message: err.to_string(),
                }));
            }
        }
    });
}
