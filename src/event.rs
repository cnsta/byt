//! Event stream merging terminal input, ticks, and async work completions.
//!
//! The TUI loop wants a single `Event` enum to react to. We unify:
//! - crossterm input events
//! - periodic ticks for re-polling VPN state
//! - signals from background tasks (e.g. "the `nmcli up` call finished")
//!
//! Background work is driven via a `tokio::sync::mpsc` channel so handlers can
//! send their own events without owning a reference to the terminal.

use std::time::Duration;

use crossterm::event::{Event as CtEvent, EventStream};
use futures::StreamExt;
use tokio::sync::mpsc;

/// Events the application loop reacts to.
#[derive(Debug)]
pub enum Event {
    /// Raw terminal input (keypress, resize, etc.).
    Term(CtEvent),
    /// Time to re-poll VPN state.
    Tick,
    /// An async action finished. The payload is application-defined.
    Action(crate::app::Action),
    /// User pressed something equivalent to Ctrl-C, or got SIGTERM.
    Quit,
}

/// Owns the event sources. Call [`EventLoop::next`] in a loop.
pub struct EventLoop {
    term: EventStream,
    ticks: tokio::time::Interval,
    rx: mpsc::UnboundedReceiver<Event>,
    pub tx: mpsc::UnboundedSender<Event>,
}

impl EventLoop {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut ticks = tokio::time::interval(tick_rate);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self {
            term: EventStream::new(),
            ticks,
            rx,
            tx,
        }
    }

    /// Yield the next event from whichever source is ready first.
    ///
    /// Returns `None` only when the terminal stream ends (typically at shutdown).
    pub async fn next(&mut self) -> Option<Event> {
        tokio::select! {
            biased;
            Some(ev) = self.rx.recv() => Some(ev),
            term = self.term.next() => match term? {
                Ok(ev) => Some(Event::Term(ev)),
                Err(err) => {
                    tracing::error!(?err, "terminal event stream error");
                    Some(Event::Quit)
                }
            },
            _ = self.ticks.tick() => Some(Event::Tick),
        }
    }
}
