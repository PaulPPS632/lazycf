//! Ciclo de vida de la terminal: alt screen + raw mode + captura de mouse.
//! `ratatui::init` instala un panic hook que restaura la terminal; añadimos
//! la captura de mouse y (si el terminal lo soporta) el protocolo de teclado
//! Kitty, necesario para distinguir Ctrl+Enter de un Enter normal.

use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::supports_keyboard_enhancement;
use ratatui::DefaultTerminal;

/// Recuerda si activamos el protocolo de teclado, para hacer el pop exacto en
/// `restore` sin volver a consultar al terminal (la consulta puede bloquear).
static KEYBOARD_PUSHED: AtomicBool = AtomicBool::new(false);

/// Entra en alt screen + raw mode + captura de mouse; devuelve la terminal.
pub fn init() -> DefaultTerminal {
    let terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    // Protocolo de teclado Kitty (si el terminal lo soporta): necesario para
    // distinguir Ctrl+Enter de un Enter normal en el editor SQL.
    if supports_keyboard_enhancement().unwrap_or(false)
        && execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok()
    {
        KEYBOARD_PUSHED.store(true, Ordering::SeqCst);
    }
    terminal
}

/// Restaura la terminal (quita protocolo de teclado, captura de mouse, alt screen).
pub fn restore() {
    if KEYBOARD_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
}
