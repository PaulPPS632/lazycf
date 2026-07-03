//! Bucle de eventos async. Una tarea tokio hace `select!` sobre la stream de
//! crossterm, un tick (lógica) y un render (frame), y empuja `Event` por un
//! canal mpsc. La app consume con `next().await`, así la UI nunca bloquea.

use std::time::Duration;

use crossterm::event::{
    Event as CtEvent, EventStream, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
};
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Evento normalizado que consume la app.
#[derive(Debug, Clone)]
pub enum Event {
    /// Cadencia de lógica de aplicación.
    Tick,
    /// Cadencia de render (redibujar frame).
    Render,
    /// Tecla pulsada (solo `Press`, no `Release`).
    Key(KeyEvent),
    /// Click izquierdo o scroll (los demás eventos de mouse se descartan).
    Mouse(MouseEvent),
    /// Terminal redimensionada (dispara un redraw; el tamaño se lee de `frame.area()`).
    Resize,
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    // Se conserva para que la tarea viva mientras exista el handler; al soltar
    // el handler, `tx.closed()` corta el bucle.
    _task: JoinHandle<()>,
}

impl EventHandler {
    /// `tick_hz`: frecuencia de lógica. `render_hz`: frecuencia de dibujo.
    pub fn new(tick_hz: f64, render_hz: f64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut reader = EventStream::new();
            let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / tick_hz));
            let mut render = tokio::time::interval(Duration::from_secs_f64(1.0 / render_hz));
            loop {
                let crossterm_event = reader.next().fuse();
                tokio::select! {
                    _ = tx.closed() => break,
                    _ = tick.tick() => { let _ = tx.send(Event::Tick); }
                    _ = render.tick() => { let _ = tx.send(Event::Render); }
                    maybe = crossterm_event => match maybe {
                        Some(Ok(CtEvent::Key(key))) if key.kind == KeyEventKind::Press => {
                            let _ = tx.send(Event::Key(key));
                        }
                        Some(Ok(CtEvent::Resize(_, _))) => { let _ = tx.send(Event::Resize); }
                        // Solo click izquierdo y scroll; el resto de mouse se descarta.
                        Some(Ok(CtEvent::Mouse(m))) => match m.kind {
                            MouseEventKind::Down(MouseButton::Left)
                            | MouseEventKind::ScrollUp
                            | MouseEventKind::ScrollDown => { let _ = tx.send(Event::Mouse(m)); }
                            _ => {}
                        },
                        // Otros eventos (focus, paste, release) se ignoran.
                        Some(Ok(_)) | Some(Err(_)) => {}
                        None => {}
                    },
                }
            }
        });
        Self { rx, _task: task }
    }

    /// Siguiente evento. `None` si la tarea murió (no debería en uso normal).
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
