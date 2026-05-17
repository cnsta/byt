//! Terminal init/restore helpers.
//!
//! Wraps ratatui's `init`/`restore` so both happen behind a single import and
//! we can install a panic hook that restores the terminal even on panic.

use ratatui::DefaultTerminal;

/// Enter raw mode, switch to the alternate screen, install a panic hook that
/// undoes both before printing the panic.
pub fn init() -> std::io::Result<DefaultTerminal> {
    install_panic_hook();
    Ok(ratatui::init())
}

/// Leave the alternate screen, disable raw mode.
pub fn restore() -> std::io::Result<()> {
    ratatui::restore();
    Ok(())
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort: if restore fails we'll still get the panic printed,
        // just maybe in raw mode.
        let _ = restore();
        default(info);
    }));
}
