//! Helpers de presentación: layout del shell, tema de colores, widgets
//! compartidos y el tipo de estado asíncrono `Loadable`.

pub mod layout;
pub mod theme;
pub mod widgets;

/// Estado de carga de un dato asíncrono, compartido por todas las vistas.
#[derive(Default)]
pub enum Loadable<T> {
    #[default]
    Idle,
    Loading,
    Failed,
    Ready(T),
}

impl<T> Loadable<T> {
    pub fn is_idle(&self) -> bool {
        matches!(self, Loadable::Idle)
    }
}
