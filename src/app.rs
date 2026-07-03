//! Estado central y bucle principal. Enruta eventos al panel con foco o al
//! popup, despacha `Action`s (sync e async), y coordina auth, cuentas y DNS.

use chrono::{SecondsFormat, Utc};
use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::action::Action;
use crate::api::CfClient;
use crate::components::command_bar::CommandBar;
use crate::components::d1::D1View;
use crate::components::detail::Detail;
use crate::components::dns::DnsView;
use crate::components::input::TextInput;
use crate::components::popup::{
    AccountPicker, BindingEdit, Confirm, Help, HelpSection, HttpTest, Message, NewBucket, NewTunnel,
    Popup, RField, RecordForm, TokenEntry,
};
use crate::components::r2::{BucketInfo, R2View};
use crate::components::sidebar::Sidebar;
use crate::components::tunnels::TunnelsView;
use crate::components::workers::{Loadable, WorkersView};
use crate::components::{Component, Module};
use crate::config::Config;
use crate::event::{Event, EventHandler};
use crate::model::{Account, Zone};
use crate::secrets;
use crate::ui::layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Auth,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Modules,
    Zones,
    Records,
    Tunnels,
    Workers,
    D1Dbs,
    D1Tables,
    D1Editor,
    D1Results,
    R2Buckets,
}

pub struct App {
    running: bool,
    screen: Screen,
    focus: Focus,
    events: EventHandler,
    action_tx: UnboundedSender<Action>,
    action_rx: UnboundedReceiver<Action>,

    // Sesión y cuentas.
    client: Option<CfClient>,
    config: Config,
    accounts: Vec<Account>,
    active_account: usize,
    status: String,

    // DNS.
    all_zones: Vec<Zone>,
    dns: DnsView,

    // Túneles.
    tunnels: TunnelsView,

    // Workers.
    workers: WorkersView,
    /// Señal para detener el tail activo (cierra el WS y borra la sesión).
    tail_stop: Option<tokio::sync::oneshot::Sender<()>>,

    // D1.
    d1: D1View,

    // R2.
    r2: R2View,

    // Componentes de shell.
    sidebar: Sidebar,
    detail: Detail,
    command_bar: CommandBar,
    popup: Option<Popup>,

    // Rects del último frame (para hit-testing del mouse).
    rect_sidebar: Rect,
    rect_zones: Option<Rect>,
    rect_records: Option<Rect>,
    rect_tunnels: Option<Rect>,
    rect_workers: Option<Rect>,
    rect_d1_dbs: Option<Rect>,
    rect_d1_tables: Option<Rect>,
    rect_d1_editor: Option<Rect>,
    rect_d1_results: Option<Rect>,
    rect_r2: Option<Rect>,
}

impl App {
    pub fn new() -> Result<Self> {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let config = Config::load().unwrap_or_default();
        let events = EventHandler::new(4.0, 60.0);

        let mut app = Self {
            running: true,
            screen: Screen::Auth,
            focus: Focus::Modules,
            events,
            action_tx,
            action_rx,
            client: None,
            config,
            accounts: Vec::new(),
            active_account: 0,
            status: String::new(),
            all_zones: Vec::new(),
            dns: DnsView::new(),
            tunnels: TunnelsView::new(),
            workers: WorkersView::new(),
            tail_stop: None,
            d1: D1View::new(),
            r2: R2View::new(),
            sidebar: Sidebar::new(),
            detail: Detail::new(),
            command_bar: CommandBar,
            popup: None,
            rect_sidebar: Rect::default(),
            rect_zones: None,
            rect_records: None,
            rect_tunnels: None,
            rect_workers: None,
            rect_d1_dbs: None,
            rect_d1_tables: None,
            rect_d1_editor: None,
            rect_d1_results: None,
            rect_r2: None,
        };

        match secrets::load_token() {
            Ok(Some(token)) => {
                app.status = "Verificando token…".into();
                app.popup = Some(Popup::Token(TokenEntry {
                    input: TextInput::new(token.clone()),
                    verifying: true,
                    error: None,
                }));
                app.spawn_verify(token);
            }
            Ok(None) => {
                app.status = "Introduce tu API token de Cloudflare".into();
                app.popup = Some(Popup::Token(TokenEntry::default()));
            }
            Err(e) => {
                app.status = "Error leyendo el keyring".into();
                app.popup = Some(Popup::Token(TokenEntry {
                    input: TextInput::default(),
                    verifying: false,
                    error: Some(e.to_string()),
                }));
            }
        }

        Ok(app)
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        while self.running {
            tokio::select! {
                Some(event) = self.events.next() => self.handle_event(&mut terminal, event)?,
                Some(action) = self.action_rx.recv() => self.dispatch(action),
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, terminal: &mut DefaultTerminal, event: Event) -> Result<()> {
        match event {
            Event::Render | Event::Resize => {
                terminal.draw(|frame| self.draw(frame))?;
            }
            Event::Key(key) => self.on_key(key),
            Event::Mouse(m) => self.on_mouse(m),
            Event::Tick => {}
        }
        Ok(())
    }

    // --- Entrada de teclado ---

    fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.dispatch(Action::Quit);
            return;
        }

        if self.popup.is_some() {
            if let Some(action) = self.popup_key(key) {
                self.dispatch(action);
            }
            return;
        }

        // El editor SQL captura todo el texto (incluidas q/x/?/A). Solo Tab y
        // Shift-Tab salen del panel; ejecutar/editar lo gestiona route_focus_key.
        if self.focus == Focus::D1Editor {
            match key.code {
                KeyCode::Tab => self.dispatch(Action::CycleFocus { back: false }),
                KeyCode::BackTab => self.dispatch(Action::CycleFocus { back: true }),
                _ => self.route_focus_key(key),
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.dispatch(Action::Quit),
            KeyCode::Char('?') => self.dispatch(Action::OpenHelp),
            KeyCode::Tab => self.dispatch(Action::CycleFocus { back: false }),
            KeyCode::BackTab => self.dispatch(Action::CycleFocus { back: true }),
            KeyCode::Char('A') => self.dispatch(Action::OpenAccountPicker),
            _ => self.route_focus_key(key),
        }
    }

    // --- Mouse ---

    fn on_mouse(&mut self, m: MouseEvent) {
        // Solo en la pantalla principal y sin popup abierto.
        if self.screen != Screen::Main || self.popup.is_some() {
            return;
        }
        let pos = Position {
            x: m.column,
            y: m.row,
        };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click_at(pos),
            MouseEventKind::ScrollDown => self.scroll_at(pos, 1),
            MouseEventKind::ScrollUp => self.scroll_at(pos, -1),
            _ => {}
        }
    }

    /// Click izquierdo: enfoca el panel bajo el cursor y selecciona la fila.
    fn click_at(&mut self, pos: Position) {
        if self.rect_sidebar.contains(pos) {
            self.focus = Focus::Modules;
            let rel = pos.y.saturating_sub(self.rect_sidebar.y + 1) as usize;
            let before = self.sidebar.module();
            if self.sidebar.module_at(rel) && self.sidebar.module() != before {
                self.on_module_entered();
            }
            return;
        }
        if let Some(r) = self.rect_zones
            && r.contains(pos)
        {
            self.focus = Focus::Zones;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            if self.dns.zone_at(rel)
                && let Some(zone_id) = self.dns.selected_zone_id()
            {
                self.load_records(zone_id);
            }
            return;
        }
        if let Some(r) = self.rect_records
            && r.contains(pos)
        {
            self.focus = Focus::Records;
            // La tabla tiene borde (+1) y fila de cabecera (+1).
            if pos.y >= r.y + 2 {
                self.dns.record_at((pos.y - (r.y + 2)) as usize);
            }
            return;
        }
        if let Some(r) = self.rect_tunnels
            && r.contains(pos)
        {
            self.focus = Focus::Tunnels;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            if self.tunnels.tunnel_at(rel)
                && let Some(tunnel_id) = self.tunnels.selected_id()
            {
                self.load_ingress(tunnel_id);
            }
            return;
        }
        if let Some(r) = self.rect_workers
            && r.contains(pos)
        {
            self.focus = Focus::Workers;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            if self.workers.script_at(rel)
                && let Some(script) = self.workers.selected_name()
            {
                self.load_metrics(script);
            }
            return;
        }
        if let Some(r) = self.rect_d1_dbs
            && r.contains(pos)
        {
            self.focus = Focus::D1Dbs;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            if self.d1.db_at(rel)
                && let Some(db_id) = self.d1.selected_db_id()
            {
                self.load_tables(db_id);
            }
            return;
        }
        if let Some(r) = self.rect_d1_tables
            && r.contains(pos)
        {
            self.focus = Focus::D1Tables;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            if self.d1.table_at(rel) {
                self.load_table_schema();
            }
            return;
        }
        if let Some(r) = self.rect_d1_editor
            && r.contains(pos)
        {
            self.focus = Focus::D1Editor;
            return;
        }
        if let Some(r) = self.rect_d1_results
            && r.contains(pos)
        {
            self.focus = Focus::D1Results;
            return;
        }
        if let Some(r) = self.rect_r2
            && r.contains(pos)
        {
            self.focus = Focus::R2Buckets;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            if self.r2.bucket_at(rel)
                && let Some(name) = self.r2.selected_name()
            {
                self.load_bucket_info(name);
            }
        }
    }

    /// Scroll: enfoca el panel bajo el cursor y mueve su selección.
    fn scroll_at(&mut self, pos: Position, delta: i32) {
        if self.rect_sidebar.contains(pos) {
            self.focus = Focus::Modules;
            let before = self.sidebar.module();
            self.sidebar.move_by(delta);
            if self.sidebar.module() != before {
                self.on_module_entered();
            }
            return;
        }
        if let Some(r) = self.rect_zones
            && r.contains(pos)
        {
            self.focus = Focus::Zones;
            self.change_zone(delta);
            return;
        }
        if let Some(r) = self.rect_records
            && r.contains(pos)
        {
            self.focus = Focus::Records;
            self.dns.select_record(delta);
            return;
        }
        if let Some(r) = self.rect_tunnels
            && r.contains(pos)
        {
            self.focus = Focus::Tunnels;
            self.change_tunnel(delta);
            return;
        }
        if let Some(r) = self.rect_workers
            && r.contains(pos)
        {
            self.focus = Focus::Workers;
            self.change_worker(delta);
            return;
        }
        if let Some(r) = self.rect_d1_dbs
            && r.contains(pos)
        {
            self.focus = Focus::D1Dbs;
            self.change_db(delta);
            return;
        }
        if let Some(r) = self.rect_d1_tables
            && r.contains(pos)
        {
            self.focus = Focus::D1Tables;
            self.change_table(delta);
            return;
        }
        if let Some(r) = self.rect_d1_results
            && r.contains(pos)
        {
            self.focus = Focus::D1Results;
            self.d1.scroll_result(delta);
            return;
        }
        if let Some(r) = self.rect_r2
            && r.contains(pos)
        {
            self.focus = Focus::R2Buckets;
            self.change_bucket(delta);
        }
    }

    /// Enruta la tecla al panel con foco.
    fn route_focus_key(&mut self, key: KeyEvent) {
        match self.focus {
            Focus::Modules => {
                let before = self.sidebar.module();
                self.sidebar.handle_key(key);
                if self.sidebar.module() != before {
                    self.on_module_entered();
                }
            }
            Focus::Zones => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_zone(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_zone(1),
                KeyCode::Char('p') => self.confirm_purge(),
                KeyCode::Char('r') => self.load_zones(),
                _ => {}
            },
            Focus::Records => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.dns.select_record(-1),
                KeyCode::Down | KeyCode::Char('j') => self.dns.select_record(1),
                KeyCode::Char(' ') => self.confirm_toggle_proxy(),
                KeyCode::Char('a') => self.open_add_record(),
                KeyCode::Char('e') => self.open_edit_record(),
                KeyCode::Char('d') => self.confirm_delete(),
                KeyCode::Char('p') => self.confirm_purge(),
                KeyCode::Char('r') => self.reload_records(),
                _ => {}
            },
            Focus::Tunnels => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_tunnel(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_tunnel(1),
                KeyCode::Char('n') => self.open_new_tunnel(),
                KeyCode::Char('c') => self.confirm_cleanup(),
                KeyCode::Char('d') => self.confirm_delete_tunnel(),
                KeyCode::Char('r') => self.load_tunnels(),
                _ => {}
            },
            Focus::Workers => match key.code {
                // En la pestaña Variables, ↑↓ navegan los bindings; si no, los workers.
                KeyCode::Up | KeyCode::Char('k') => self.workers_up_down(-1),
                KeyCode::Down | KeyCode::Char('j') => self.workers_up_down(1),
                KeyCode::Left => {
                    self.workers.cycle_tab(-1);
                    self.load_active_tab();
                }
                KeyCode::Right => {
                    self.workers.cycle_tab(1);
                    self.load_active_tab();
                }
                KeyCode::Char(c @ '1'..='4') => {
                    self.workers.set_tab(c as usize - '1' as usize);
                    self.load_active_tab();
                }
                KeyCode::Char('e') if self.workers.active_tab == 2 => self.open_edit_binding(),
                KeyCode::Char('a') if self.workers.active_tab == 2 => self.open_add_secret(),
                KeyCode::Char('l') => self.toggle_tail(),
                KeyCode::Char('t') => self.open_http_test(),
                KeyCode::Char('r') => self.load_workers(),
                _ => {}
            },
            Focus::D1Dbs => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_db(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_db(1),
                KeyCode::Char('r') => self.load_databases(),
                _ => {}
            },
            Focus::D1Tables => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_table(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_table(1),
                KeyCode::Enter => self.run_select(),
                KeyCode::Char('r') => self.reload_tables(),
                _ => {}
            },
            // Editor SQL: teclas normales escriben; F5/Ctrl+Enter ejecuta.
            Focus::D1Editor => match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.run_editor()
                }
                KeyCode::F(5) => self.run_editor(),
                KeyCode::Enter => self.d1.editor_mut().insert('\n'),
                KeyCode::Backspace => self.d1.editor_mut().backspace(),
                KeyCode::Delete => self.d1.editor_mut().delete(),
                KeyCode::Left => self.d1.editor_mut().left(),
                KeyCode::Right => self.d1.editor_mut().right(),
                KeyCode::Up => self.d1.editor_mut().up(),
                KeyCode::Down => self.d1.editor_mut().down(),
                KeyCode::Home => self.d1.editor_mut().home(),
                KeyCode::End => self.d1.editor_mut().end(),
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.d1.editor_mut().insert(c)
                }
                _ => {}
            },
            Focus::D1Results => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.d1.scroll_result(-1),
                KeyCode::Down | KeyCode::Char('j') => self.d1.scroll_result(1),
                KeyCode::PageUp => self.d1.scroll_result(-10),
                KeyCode::PageDown => self.d1.scroll_result(10),
                _ => {}
            },
            Focus::R2Buckets => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_bucket(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_bucket(1),
                KeyCode::Char('n') => self.open_new_bucket(),
                KeyCode::Char('d') => self.confirm_delete_bucket(),
                KeyCode::Char('r') => self.load_buckets(),
                _ => {}
            },
        }
    }

    /// Carga perezosa al entrar en un módulo por primera vez.
    fn on_module_entered(&mut self) {
        match self.sidebar.module() {
            Module::Dns if self.all_zones.is_empty() && !self.dns.loading_zones => {
                self.load_zones();
            }
            Module::Tunnels if self.tunnels.is_empty() && !self.tunnels.loading => {
                self.load_tunnels();
            }
            Module::Workers if self.workers.is_empty() && !self.workers.loading => {
                self.load_workers();
            }
            Module::D1 if self.d1.is_empty() && !self.d1.loading => {
                self.load_databases();
            }
            Module::R2 if self.r2.is_empty() && !self.r2.loading => {
                self.load_buckets();
            }
            _ => {}
        }
    }

    /// Teclas dirigidas al popup activo.
    fn popup_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Copiamos el tipo de popup para soltar el préstamo antes de mutarlo.
        enum Kind {
            Token,
            Confirm,
            Account,
            Help,
            NewTunnel,
            NewBucket,
            RecordForm,
            HttpTest,
            BindingEdit,
            Message,
        }
        let kind = match self.popup.as_ref()? {
            Popup::Token(_) => Kind::Token,
            Popup::Confirm(_) => Kind::Confirm,
            Popup::AccountPicker(_) => Kind::Account,
            Popup::Help(_) => Kind::Help,
            Popup::NewTunnel(_) => Kind::NewTunnel,
            Popup::NewBucket(_) => Kind::NewBucket,
            Popup::RecordForm(_) => Kind::RecordForm,
            Popup::HttpTest(_) => Kind::HttpTest,
            Popup::BindingEdit(_) => Kind::BindingEdit,
            Popup::Message(_) => Kind::Message,
        };

        match kind {
            Kind::Token => {
                let Some(Popup::Token(entry)) = self.popup.as_mut() else {
                    return None;
                };
                if entry.verifying {
                    return None;
                }
                match key.code {
                    KeyCode::Enter => {
                        if entry.input.value().trim().is_empty() {
                            entry.error = Some("El token está vacío".into());
                            None
                        } else {
                            entry.error = None;
                            entry.verifying = true;
                            Some(Action::SubmitToken(entry.input.take()))
                        }
                    }
                    KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(Action::OpenTokenPage)
                    }
                    KeyCode::Backspace => {
                        entry.input.backspace();
                        None
                    }
                    KeyCode::Delete => {
                        entry.input.delete();
                        None
                    }
                    KeyCode::Left => {
                        entry.input.left();
                        None
                    }
                    KeyCode::Right => {
                        entry.input.right();
                        None
                    }
                    KeyCode::Home => {
                        entry.input.home();
                        None
                    }
                    KeyCode::End => {
                        entry.input.end();
                        None
                    }
                    KeyCode::Char(c) => {
                        entry.input.insert(c);
                        None
                    }
                    KeyCode::Esc if self.screen == Screen::Main => {
                        self.popup = None;
                        None
                    }
                    _ => None,
                }
            }
            Kind::Confirm => match key.code {
                KeyCode::Char('s' | 'S' | 'y' | 'Y') | KeyCode::Enter => {
                    if let Some(Popup::Confirm(c)) = self.popup.take() {
                        Some(c.on_yes)
                    } else {
                        None
                    }
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.popup = None;
                    None
                }
                _ => None,
            },
            Kind::Account => {
                let Some(Popup::AccountPicker(p)) = self.popup.as_mut() else {
                    return None;
                };
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        p.move_by(-1);
                        None
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        p.move_by(1);
                        None
                    }
                    KeyCode::Enter => Some(Action::SwitchAccount(p.selected())),
                    KeyCode::Esc => {
                        self.popup = None;
                        None
                    }
                    _ => None,
                }
            }
            Kind::NewTunnel => match key.code {
                KeyCode::Esc => {
                    self.popup = None;
                    None
                }
                KeyCode::Enter => {
                    let name = match self.popup.as_ref() {
                        Some(Popup::NewTunnel(t)) => t.name.value().trim().to_string(),
                        _ => String::new(),
                    };
                    if name.is_empty() {
                        if let Some(Popup::NewTunnel(t)) = self.popup.as_mut() {
                            t.error = Some("El nombre es obligatorio".into());
                        }
                        None
                    } else {
                        self.popup = None;
                        Some(Action::CreateTunnel(name))
                    }
                }
                _ => {
                    if let Some(Popup::NewTunnel(t)) = self.popup.as_mut() {
                        edit_input(&mut t.name, key.code);
                    }
                    None
                }
            },
            Kind::NewBucket => match key.code {
                KeyCode::Esc => {
                    self.popup = None;
                    None
                }
                KeyCode::Enter => {
                    let name = match self.popup.as_ref() {
                        Some(Popup::NewBucket(b)) => b.name.value().trim().to_string(),
                        _ => String::new(),
                    };
                    if name.is_empty() {
                        if let Some(Popup::NewBucket(b)) = self.popup.as_mut() {
                            b.error = Some("El nombre es obligatorio".into());
                        }
                        None
                    } else {
                        self.popup = None;
                        Some(Action::CreateBucket(name))
                    }
                }
                _ => {
                    if let Some(Popup::NewBucket(b)) = self.popup.as_mut() {
                        edit_input(&mut b.name, key.code);
                    }
                    None
                }
            },
            Kind::RecordForm => {
                // Esc cierra siempre; mientras se envía, se ignora el resto.
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::RecordForm(f)) if f.submitting) {
                    return None;
                }
                if key.code == KeyCode::Enter {
                    let valid = matches!(self.popup.as_ref(), Some(Popup::RecordForm(f))
                        if !f.name.value().trim().is_empty() && !f.content.value().trim().is_empty());
                    if let Some(Popup::RecordForm(f)) = self.popup.as_mut() {
                        if !valid {
                            f.error = Some("Nombre y contenido son obligatorios".into());
                            return None;
                        }
                        // Mantener el form abierto durante el envío para poder
                        // mostrar el error del API sin perder lo escrito.
                        f.error = None;
                        f.submitting = true;
                        return Some(Action::SubmitRecord {
                            zone_id: f.zone_id.clone(),
                            editing_id: f.editing_id.clone(),
                            proxied: f.proxied && f.proxiable(),
                            rtype: f.rtype().to_string(),
                            name: f.name.take(),
                            content: f.content.take(),
                            ttl: f.ttl.take(),
                            priority: f.priority.take(),
                        });
                    }
                    return None;
                }
                if let Some(Popup::RecordForm(f)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab => f.move_field(-1),
                        KeyCode::Down | KeyCode::Tab => f.move_field(1),
                        KeyCode::Left if f.current() == RField::Type => f.cycle_type(-1),
                        KeyCode::Right if f.current() == RField::Type => f.cycle_type(1),
                        KeyCode::Char(' ') if f.current() == RField::Proxy => {
                            f.proxied = !f.proxied
                        }
                        code => {
                            if let Some(s) = f.active_text_mut() {
                                edit_input(s, code);
                            }
                        }
                    }
                }
                None
            }
            Kind::HttpTest => match key.code {
                KeyCode::Esc => {
                    self.popup = None;
                    None
                }
                KeyCode::Enter => {
                    let url = match self.popup.as_ref() {
                        Some(Popup::HttpTest(t)) if !t.sending => t.url.value().trim().to_string(),
                        _ => return None,
                    };
                    if url.is_empty() {
                        if let Some(Popup::HttpTest(t)) = self.popup.as_mut() {
                            t.error = Some("URL vacía".into());
                        }
                        None
                    } else {
                        if let Some(Popup::HttpTest(t)) = self.popup.as_mut() {
                            t.sending = true;
                            t.error = None;
                        }
                        Some(Action::HttpProbe(url))
                    }
                }
                _ => {
                    if let Some(Popup::HttpTest(t)) = self.popup.as_mut()
                        && !t.sending
                    {
                        edit_input(&mut t.url, key.code);
                    }
                    None
                }
            },
            Kind::BindingEdit => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::BindingEdit(b)) if b.submitting) {
                    return None;
                }
                if key.code == KeyCode::Enter {
                    let valid = matches!(self.popup.as_ref(), Some(Popup::BindingEdit(b))
                        if !b.name.value().trim().is_empty() && !b.value.value().trim().is_empty());
                    if let Some(Popup::BindingEdit(b)) = self.popup.as_mut() {
                        if !valid {
                            b.error = Some("Nombre y valor son obligatorios".into());
                            return None;
                        }
                        b.error = None;
                        b.submitting = true;
                        return Some(Action::SaveBinding {
                            script: b.script.clone(),
                            name: b.name.take(),
                            is_secret: b.is_secret,
                            value: b.value.take(),
                            adding: b.adding,
                        });
                    }
                    return None;
                }
                if let Some(Popup::BindingEdit(b)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab => b.move_field(-1),
                        KeyCode::Down | KeyCode::Tab => b.move_field(1),
                        code => edit_input(b.active_text_mut(), code),
                    }
                }
                None
            }
            Kind::Help | Kind::Message => {
                self.popup = None;
                None
            }
        }
    }

    // --- Despacho de acciones ---

    fn dispatch(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::CycleFocus { back } => self.cycle_focus(back),

            Action::SubmitToken(token) => {
                self.status = "Verificando token…".into();
                self.spawn_verify(token);
            }
            Action::TokenVerified { token, accounts } => {
                match secrets::save_token(&token) {
                    Ok(()) => self.status = "Autenticado".into(),
                    Err(e) => {
                        tracing::warn!("no se pudo guardar el token en el keyring: {e}");
                        self.status = "Autenticado (token no persistido en keyring)".into();
                    }
                }
                self.client = CfClient::new(token).ok();
                self.accounts = accounts;
                self.active_account = self
                    .config
                    .default_account_id
                    .as_ref()
                    .and_then(|id| self.accounts.iter().position(|a| &a.id == id))
                    .unwrap_or(0);
                self.screen = Screen::Main;
                self.popup = None;
                self.load_zones();
            }
            Action::AuthFailed(msg) => {
                self.screen = Screen::Auth;
                self.status = "Fallo de autenticación".into();
                match self.popup.as_mut() {
                    Some(Popup::Token(entry)) => {
                        entry.verifying = false;
                        entry.error = Some(msg);
                    }
                    _ => {
                        self.popup = Some(Popup::Token(TokenEntry {
                            input: TextInput::default(),
                            verifying: false,
                            error: Some(msg),
                        }));
                    }
                }
            }
            Action::OpenTokenPage => match crate::browser::open(crate::browser::TOKEN_PAGE) {
                Ok(()) => self.status = "Abriendo el dashboard en el navegador…".into(),
                Err(e) => {
                    self.status = format!("No se pudo abrir el navegador: {e}");
                    tracing::warn!("abrir navegador: {e}");
                }
            },
            Action::OpenHelp => self.popup = Some(Popup::Help(self.build_help())),

            Action::OpenAccountPicker => {
                if !self.accounts.is_empty() {
                    self.popup = Some(Popup::AccountPicker(AccountPicker::new(
                        self.accounts.clone(),
                        self.active_account,
                    )));
                }
            }
            Action::SwitchAccount(i) => {
                if i < self.accounts.len() {
                    self.active_account = i;
                    self.status = format!("Cuenta: {}", self.accounts[i].name);
                    self.apply_account_filter();
                    // Recursos account-scoped: resetear y recargar si procede.
                    self.stop_tail();
                    self.tunnels.reset();
                    self.workers.reset();
                    self.workers.set_subdomain(None);
                    self.d1.reset();
                    self.r2.reset();
                    match self.sidebar.module() {
                        Module::Tunnels => self.load_tunnels(),
                        Module::Workers => self.load_workers(),
                        Module::D1 => self.load_databases(),
                        Module::R2 => self.load_buckets(),
                        _ => {}
                    }
                }
                self.popup = None;
            }

            Action::ZonesLoaded(zones) => {
                self.all_zones = zones;
                self.apply_account_filter();
            }
            Action::RecordsLoaded { zone_id, records } => {
                if self.dns.selected_zone_id().as_deref() == Some(zone_id.as_str()) {
                    self.dns.set_records(records);
                }
            }
            Action::ToggleProxy => self.toggle_proxy(),
            Action::SubmitRecord {
                zone_id,
                editing_id,
                rtype,
                name,
                content,
                ttl,
                proxied,
                priority,
            } => self.spawn_submit_record(
                zone_id, editing_id, rtype, name, content, ttl, proxied, priority,
            ),
            Action::DeleteRecord { zone_id, record_id } => self.spawn_delete(zone_id, record_id),
            Action::PurgeCache { zone_id } => self.spawn_purge(zone_id),
            Action::DnsMutated(msg) => {
                self.status = msg;
                // Si venía de un formulario, ciérralo (éxito).
                if matches!(self.popup, Some(Popup::RecordForm(_))) {
                    self.popup = None;
                }
                if let Some(zone_id) = self.dns.selected_zone_id() {
                    self.load_records(zone_id);
                }
            }
            Action::DnsStatus(msg) => self.status = msg,
            Action::DnsError(e) => {
                // Si hay un formulario abierto, muestra el error ahí (no lo cierres)
                // para que el usuario corrija sin re-escribir.
                if let Some(Popup::RecordForm(f)) = self.popup.as_mut() {
                    f.submitting = false;
                    f.error = Some(e);
                } else {
                    self.dns.loading_zones = false;
                    self.dns.loading_records = false;
                    self.dns.error = Some(e.clone());
                    self.status = format!("Error: {e}");
                }
            }

            Action::TunnelsLoaded(tunnels) => {
                self.tunnels.set_tunnels(tunnels);
                if let Some(tunnel_id) = self.tunnels.selected_id() {
                    self.load_ingress(tunnel_id);
                }
            }
            Action::IngressLoaded { tunnel_id, rules } => {
                if self.tunnels.selected_id().as_deref() == Some(tunnel_id.as_str()) {
                    self.tunnels.set_ingress(rules);
                }
            }
            Action::CreateTunnel(name) => self.spawn_create_tunnel(name),
            Action::TunnelCreated { name, token } => {
                let body = if token.is_empty() {
                    format!("Túnel '{name}' creado.")
                } else {
                    format!(
                        "Túnel '{name}' creado.\n\nConéctalo con:\ncloudflared tunnel run --token {token}"
                    )
                };
                self.popup = Some(Popup::Message(Message {
                    title: "Túnel creado".into(),
                    body,
                    is_error: false,
                }));
                self.load_tunnels();
            }
            Action::CleanupConnections { tunnel_id } => self.spawn_cleanup(tunnel_id),
            Action::DeleteTunnel { tunnel_id } => self.spawn_delete_tunnel(tunnel_id),
            Action::TunnelMutated(msg) => {
                self.status = msg;
                self.load_tunnels();
            }
            Action::TunnelError(e) => {
                self.tunnels.loading = false;
                self.tunnels.loading_ingress = false;
                self.tunnels.error = Some(e.clone());
                self.status = format!("Error: {e}");
            }

            Action::WorkersLoaded(scripts) => {
                self.workers.set_scripts(scripts);
                self.load_active_tab();
            }
            Action::SubdomainLoaded(sub) => self.workers.set_subdomain(sub),
            Action::MetricsLoaded { script, metrics } => {
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers.set_metrics(metrics);
                }
            }
            Action::DeploymentsLoaded {
                script,
                deployments,
            } => {
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers.set_deployments(deployments);
                }
            }
            Action::BindingsLoaded { script, bindings } => {
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers.set_bindings(bindings);
                }
            }
            Action::WorkersError(e) => {
                self.workers.loading = false;
                self.workers.error = Some(e.clone());
                self.status = format!("Error: {e}");
            }
            Action::HttpProbe(url) => self.spawn_probe(url),
            Action::HttpResult {
                status,
                millis,
                info,
            } => {
                let (title, is_error) = match status {
                    Some(code) => (format!("Respuesta {code}"), !(200..400).contains(&code)),
                    None => ("Sin respuesta".to_string(), true),
                };
                let body = match status {
                    Some(_) => format!("{millis} ms\n\n{info}"),
                    None => format!("{millis} ms\n\n{info}"),
                };
                self.popup = Some(Popup::Message(Message {
                    title,
                    body,
                    is_error,
                }));
            }

            Action::StartTail(script) => self.spawn_tail(script),
            Action::StopTail => {
                self.stop_tail();
                self.status = "Tail detenido".into();
            }
            Action::TailStarted { script } => {
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.status = "Tail: ● en vivo".into();
                    self.workers.push_logs(vec!["· conectado".into()]);
                }
            }
            Action::TailLines { script, lines } => {
                if self.workers.tailing
                    && self.workers.selected_name().as_deref() == Some(script.as_str())
                {
                    self.workers.push_logs(lines);
                }
            }
            Action::TailError { script, msg } => {
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers.push_logs(vec![format!("✗ {msg}")]);
                    self.status = format!("Tail: {msg}");
                }
            }
            Action::TailEnded { script } => {
                self.tail_stop = None;
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers.set_tailing(false);
                    self.workers.push_logs(vec!["· tail finalizado".into()]);
                }
            }

            Action::SaveBinding {
                script,
                name,
                is_secret,
                value,
                adding,
            } => self.spawn_save_binding(script, name, is_secret, value, adding),
            Action::BindingSaved { script, msg } => {
                self.status = msg;
                if matches!(self.popup, Some(Popup::BindingEdit(_))) {
                    self.popup = None;
                }
                // Recarga la pestaña de variables para reflejar el cambio.
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.load_bindings(script);
                }
            }
            Action::BindingError(e) => {
                if let Some(Popup::BindingEdit(b)) = self.popup.as_mut() {
                    b.submitting = false;
                    b.error = Some(e);
                } else {
                    self.status = format!("Error: {e}");
                }
            }

            Action::D1DatabasesLoaded(dbs) => {
                self.d1.set_databases(dbs);
                if let Some(db_id) = self.d1.selected_db_id() {
                    self.load_tables(db_id);
                }
            }
            Action::D1TablesLoaded { db_id, tables } => {
                self.d1.set_tables(&db_id, tables);
                // Muestra el esquema de la primera tabla automáticamente.
                if self.d1.selected_db_id().as_deref() == Some(db_id.as_str())
                    && self.d1.selected_table().is_some()
                {
                    self.load_table_schema();
                }
            }
            Action::D1TablesError(e) => self.d1.set_tables_error(e),
            Action::D1ResultLoaded {
                db_id,
                title,
                outcome,
            } => {
                if self.d1.selected_db_id().as_deref() == Some(db_id.as_str()) {
                    match outcome {
                        Ok(o) => self.d1.set_result(title, o),
                        Err(e) => self.d1.set_result_error(e),
                    }
                }
            }
            Action::D1Error(e) => {
                self.d1.loading = false;
                self.d1.error = Some(e.clone());
                self.status = format!("Error: {e}");
            }

            Action::R2BucketsLoaded(buckets) => {
                self.r2.set_buckets(buckets);
                if let Some(name) = self.r2.selected_name() {
                    self.load_bucket_info(name);
                }
            }
            Action::R2InfoLoaded { bucket, info } => {
                self.r2.set_info(&bucket, info.map(|b| *b));
            }
            Action::CreateBucket(name) => self.spawn_create_bucket(name),
            Action::DeleteBucket(name) => self.spawn_delete_bucket(name),
            Action::R2Mutated(msg) => {
                self.status = msg;
                self.load_buckets();
            }
            Action::R2Error(e) => {
                self.r2.loading = false;
                self.r2.error = Some(e.clone());
                self.status = format!("Error: {e}");
            }
        }
    }

    // --- Cuentas / zonas ---

    fn active_account_id(&self) -> Option<&str> {
        self.accounts.get(self.active_account).map(|a| a.id.as_str())
    }

    /// Filtra `all_zones` por la cuenta activa y carga los registros de la primera.
    fn apply_account_filter(&mut self) {
        let filtered: Vec<Zone> = match self.active_account_id() {
            Some(acc_id) => self
                .all_zones
                .iter()
                .filter(|z| z.account_id().is_none_or(|zid| zid == acc_id))
                .cloned()
                .collect(),
            None => self.all_zones.clone(),
        };
        self.dns.set_zones(filtered);
        if let Some(zone_id) = self.dns.selected_zone_id() {
            self.load_records(zone_id);
        }
    }

    fn change_zone(&mut self, delta: i32) {
        if self.dns.select_zone(delta)
            && let Some(zone_id) = self.dns.selected_zone_id()
        {
            self.load_records(zone_id);
        }
    }

    fn reload_records(&mut self) {
        if let Some(zone_id) = self.dns.selected_zone_id() {
            self.load_records(zone_id);
        }
    }

    // --- Confirmaciones ---

    fn confirm_purge(&mut self) {
        let Some(zone) = self.dns.selected_zone() else {
            return;
        };
        let (zone_id, zone_name) = (zone.id.clone(), zone.name.clone());
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Purgar caché".into(),
            body: format!("¿Purgar TODA la caché de {zone_name}?"),
            on_yes: Action::PurgeCache { zone_id },
        }));
    }

    fn confirm_delete(&mut self) {
        let (Some(zone), Some(record)) = (self.dns.selected_zone(), self.dns.selected_record())
        else {
            return;
        };
        let zone_id = zone.id.clone();
        let record_id = record.id.clone();
        let label = format!("{} {}", record.record_type, record.name);
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar registro".into(),
            body: format!("¿Borrar el registro {label}?"),
            on_yes: Action::DeleteRecord { zone_id, record_id },
        }));
    }

    fn confirm_toggle_proxy(&mut self) {
        let Some(record) = self.dns.selected_record() else {
            return;
        };
        if !record.is_proxiable() {
            self.status = "Este tipo de registro no es proxiable".into();
            return;
        }
        let turning_on = record.proxied != Some(true);
        let name = record.name.clone();
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Cambiar proxy".into(),
            body: format!(
                "¿{} el proxy de {name}?",
                if turning_on { "Activar" } else { "Desactivar" }
            ),
            on_yes: Action::ToggleProxy,
        }));
    }

    fn open_add_record(&mut self) {
        let Some(zone_id) = self.dns.selected_zone_id() else {
            return;
        };
        self.popup = Some(Popup::RecordForm(RecordForm::create(zone_id)));
    }

    fn open_edit_record(&mut self) {
        let Some(zone_id) = self.dns.selected_zone_id() else {
            return;
        };
        let Some(record) = self.dns.selected_record() else {
            return;
        };
        self.popup = Some(Popup::RecordForm(RecordForm::edit(zone_id, record)));
    }

    fn open_new_tunnel(&mut self) {
        self.popup = Some(Popup::NewTunnel(NewTunnel::default()));
    }

    // --- Tareas async ---

    fn spawn_verify(&self, token: String) {
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match CfClient::new(token.clone()) {
                Ok(client) => match client.authenticate().await {
                    Ok(info) => Action::TokenVerified {
                        token,
                        accounts: info.accounts,
                    },
                    Err(e) => Action::AuthFailed(e.to_string()),
                },
                Err(e) => Action::AuthFailed(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn load_zones(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.dns.loading_zones = true;
        self.dns.error = None;
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client.list_zones().await {
                Ok(zones) => Action::ZonesLoaded(zones),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn load_records(&mut self, zone_id: String) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.dns.begin_loading_records();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client.list_dns_records(&zone_id).await {
                Ok(records) => Action::RecordsLoaded { zone_id, records },
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn toggle_proxy(&mut self) {
        let (Some(client), Some(zone_id)) = (self.client.clone(), self.dns.selected_zone_id())
        else {
            return;
        };
        let Some(record) = self.dns.selected_record() else {
            return;
        };
        if !record.is_proxiable() {
            self.status = "Este tipo de registro no es proxiable".into();
            return;
        }
        let record_id = record.id.clone();
        let new_val = record.proxied != Some(true);
        let tx = self.action_tx.clone();
        self.status = "Cambiando proxy…".into();
        tokio::spawn(async move {
            let action = match client.set_dns_proxied(&zone_id, &record_id, new_val).await {
                Ok(_) => Action::DnsMutated(
                    if new_val {
                        "Proxy activado"
                    } else {
                        "Proxy desactivado"
                    }
                    .into(),
                ),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn spawn_delete(&mut self, zone_id: String, record_id: String) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Borrando registro…".into();
        tokio::spawn(async move {
            let action = match client.delete_dns_record(&zone_id, &record_id).await {
                Ok(_) => Action::DnsMutated("Registro borrado".into()),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_submit_record(
        &mut self,
        zone_id: String,
        editing_id: Option<String>,
        rtype: String,
        name: String,
        content: String,
        ttl: String,
        proxied: bool,
        priority: String,
    ) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let tx = self.action_tx.clone();
        let editing = editing_id.is_some();
        self.status = if editing {
            "Guardando registro…"
        } else {
            "Creando registro…"
        }
        .into();

        let rtype_up = rtype.trim().to_uppercase();
        let ttl_num: u32 = ttl.trim().parse().unwrap_or(1);
        let mut body = serde_json::json!({
            "type": rtype_up,
            "name": name.trim(),
            "content": content.trim(),
            "ttl": ttl_num,
        });
        if matches!(rtype_up.as_str(), "A" | "AAAA" | "CNAME") {
            body["proxied"] = serde_json::Value::Bool(proxied);
        }
        if rtype_up == "MX" {
            body["priority"] = serde_json::json!(priority.trim().parse::<u32>().unwrap_or(10));
        }

        tokio::spawn(async move {
            let result = match &editing_id {
                Some(id) => client.update_dns_record(&zone_id, id, &body).await,
                None => client.create_dns_record(&zone_id, &body).await,
            };
            let action = match result {
                Ok(_) => Action::DnsMutated(
                    if editing {
                        "Registro actualizado"
                    } else {
                        "Registro creado"
                    }
                    .into(),
                ),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn spawn_purge(&mut self, zone_id: String) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Purgando caché…".into();
        tokio::spawn(async move {
            let action = match client.purge_everything(&zone_id).await {
                Ok(_) => Action::DnsStatus("Caché purgada".into()),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    // --- Túneles ---

    fn change_tunnel(&mut self, delta: i32) {
        if self.tunnels.select(delta)
            && let Some(tunnel_id) = self.tunnels.selected_id()
        {
            self.load_ingress(tunnel_id);
        }
    }

    fn confirm_cleanup(&mut self) {
        let Some(tunnel) = self.tunnels.selected() else {
            return;
        };
        let (tunnel_id, name) = (tunnel.id.clone(), tunnel.name.clone());
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Limpiar conexiones".into(),
            body: format!("¿Desconectar todas las conexiones de {name}?"),
            on_yes: Action::CleanupConnections { tunnel_id },
        }));
    }

    fn confirm_delete_tunnel(&mut self) {
        let Some(tunnel) = self.tunnels.selected() else {
            return;
        };
        let (tunnel_id, name) = (tunnel.id.clone(), tunnel.name.clone());
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar túnel".into(),
            body: format!("¿Borrar el túnel {name}? Se limpian sus conexiones primero."),
            on_yes: Action::DeleteTunnel { tunnel_id },
        }));
    }

    fn load_tunnels(&mut self) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.tunnels.loading = true;
        self.tunnels.error = None;
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client.list_tunnels(&account_id).await {
                Ok(tunnels) => Action::TunnelsLoaded(tunnels),
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn load_ingress(&mut self, tunnel_id: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.tunnels.begin_loading_ingress();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            // Un 404 aquí = túnel local sin config remota → se trata como vacío.
            let rules = match client.tunnel_ingress(&account_id, &tunnel_id).await {
                Ok(rules) => rules,
                Err(e) => {
                    tracing::debug!("ingress {tunnel_id}: {e}");
                    Vec::new()
                }
            };
            let _ = tx.send(Action::IngressLoaded { tunnel_id, rules });
        });
    }

    fn spawn_create_tunnel(&mut self, name: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Creando túnel…".into();
        tokio::spawn(async move {
            let action = match client.create_tunnel(&account_id, &name).await {
                Ok(t) => Action::TunnelCreated {
                    name: t.name,
                    token: t.token,
                },
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn spawn_cleanup(&mut self, tunnel_id: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Limpiando conexiones…".into();
        tokio::spawn(async move {
            let action = match client.cleanup_tunnel_connections(&account_id, &tunnel_id).await {
                Ok(()) => Action::TunnelMutated("Conexiones limpiadas".into()),
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn spawn_delete_tunnel(&mut self, tunnel_id: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Borrando túnel…".into();
        tokio::spawn(async move {
            // Limpiar conexiones primero (si no hay, se ignora el error).
            let _ = client.cleanup_tunnel_connections(&account_id, &tunnel_id).await;
            let action = match client.delete_tunnel(&account_id, &tunnel_id).await {
                Ok(()) => Action::TunnelMutated("Túnel borrado".into()),
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    // --- Workers ---

    fn change_worker(&mut self, delta: i32) {
        if self.workers.select(delta) {
            // Cambiar de worker detiene el tail y limpia sus logs.
            self.stop_tail();
            self.workers.clear_logs();
            self.workers.reset_tabs();
            self.load_active_tab();
        }
    }

    /// ↑↓ en Workers: navega bindings en la pestaña Variables; si no, workers.
    fn workers_up_down(&mut self, delta: i32) {
        if self.workers.active_tab == 2 && self.workers.bindings_ready_nonempty() {
            self.workers.select_binding(delta);
        } else {
            self.change_worker(delta);
        }
    }

    fn open_edit_binding(&mut self) {
        let Some(script) = self.workers.selected_name() else {
            return;
        };
        let Some(b) = self.workers.selected_binding() else {
            return;
        };
        if !(b.btype == "plain_text" || b.btype == "secret_text") {
            self.status = "Solo se pueden editar variables y secretos".into();
            return;
        }
        self.popup = Some(Popup::BindingEdit(BindingEdit::edit(script, b)));
    }

    fn open_add_secret(&mut self) {
        let Some(script) = self.workers.selected_name() else {
            return;
        };
        self.popup = Some(Popup::BindingEdit(BindingEdit::add_secret(script)));
    }

    /// `l`: inicia el live-tail si no hay uno; si ya está activo, lo detiene.
    fn toggle_tail(&mut self) {
        if self.workers.tailing {
            self.dispatch(Action::StopTail);
        } else if let Some(script) = self.workers.selected_name() {
            self.dispatch(Action::StartTail(script));
        }
    }

    /// Señala el cierre del tail activo (el task borra la sesión al terminar).
    fn stop_tail(&mut self) {
        if let Some(tx) = self.tail_stop.take() {
            let _ = tx.send(());
        }
        self.workers.set_tailing(false);
    }

    /// Carga (perezosa) los datos de la pestaña activa del worker seleccionado.
    fn load_active_tab(&mut self) {
        let Some(script) = self.workers.selected_name() else {
            return;
        };
        match self.workers.active_tab {
            0 if self.workers.metrics.is_idle() => self.load_metrics(script),
            1 if self.workers.deployments.is_idle() => self.load_deployments(script),
            2 if self.workers.bindings.is_idle() => self.load_bindings(script),
            _ => {}
        }
    }

    fn open_http_test(&mut self) {
        let url = self
            .workers
            .suggested_url()
            .unwrap_or_else(|| "https://".into());
        self.popup = Some(Popup::HttpTest(HttpTest {
            url: TextInput::new(url),
            error: None,
            sending: false,
        }));
    }

    fn load_workers(&mut self) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.workers.loading = true;
        self.workers.error = None;
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let sub = client.workers_subdomain(&account_id).await.ok().flatten();
            let _ = tx.send(Action::SubdomainLoaded(sub));
            let action = match client.list_scripts(&account_id).await {
                Ok(scripts) => Action::WorkersLoaded(scripts),
                Err(e) => Action::WorkersError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn load_metrics(&mut self, script: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.workers.begin_metrics();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let end = Utc::now();
            let start = end - chrono::Duration::hours(24);
            let start_s = start.to_rfc3339_opts(SecondsFormat::Secs, true);
            let end_s = end.to_rfc3339_opts(SecondsFormat::Secs, true);
            let metrics = match client
                .worker_metrics(&account_id, &script, &start_s, &end_s)
                .await
            {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::debug!("métricas {script}: {e}");
                    None
                }
            };
            let _ = tx.send(Action::MetricsLoaded { script, metrics });
        });
    }

    fn load_deployments(&mut self, script: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.workers.begin_deployments();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let deployments = client.list_deployments(&account_id, &script).await.ok();
            let _ = tx.send(Action::DeploymentsLoaded { script, deployments });
        });
    }

    fn load_bindings(&mut self, script: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.workers.begin_bindings();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let bindings = client.worker_bindings(&account_id, &script).await.ok();
            let _ = tx.send(Action::BindingsLoaded { script, bindings });
        });
    }

    fn spawn_probe(&mut self, url: String) {
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let r = crate::api::workers::http_probe(url).await;
            let _ = tx.send(Action::HttpResult {
                status: r.status,
                millis: r.millis,
                info: r.info,
            });
        });
    }

    /// Inicia el live-tail: crea la sesión, conecta el WS y transmite líneas.
    /// Un `oneshot` corta el bucle; al salir cierra el WS y borra la sesión.
    fn spawn_tail(&mut self, script: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        // Detén cualquier tail previo antes de abrir otro.
        self.stop_tail();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        self.tail_stop = Some(stop_tx);
        self.workers.set_tab(3);
        self.workers.clear_logs();
        self.workers.set_tailing(true);
        self.workers.push_logs(vec!["· conectando…".into()]);
        self.status = "Iniciando tail…".into();

        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            use futures::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;

            let (tail_id, url) = match client.create_tail(&account_id, &script).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(Action::TailError {
                        script: script.clone(),
                        msg: e.to_string(),
                    });
                    let _ = tx.send(Action::TailEnded { script });
                    return;
                }
            };
            let mut ws = match crate::api::workers::connect_tail(&url).await {
                Ok(w) => w,
                Err(e) => {
                    let _ = tx.send(Action::TailError {
                        script: script.clone(),
                        msg: e.to_string(),
                    });
                    client.delete_tail(&account_id, &script, &tail_id).await.ok();
                    let _ = tx.send(Action::TailEnded { script });
                    return;
                }
            };
            // Filtro vacío = todos los eventos (protocolo trace-v1).
            let _ = ws
                .send(Message::Text("{\"filters\":[]}".into()))
                .await;
            let _ = tx.send(Action::TailStarted {
                script: script.clone(),
            });

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    msg = ws.next() => match msg {
                        Some(Ok(Message::Text(t))) => {
                            let lines = crate::api::workers::parse_tail(t.as_str());
                            if !lines.is_empty() {
                                let _ = tx.send(Action::TailLines { script: script.clone(), lines });
                            }
                        }
                        Some(Ok(Message::Ping(p))) => {
                            let _ = ws.send(Message::Pong(p)).await;
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            let _ = tx.send(Action::TailError {
                                script: script.clone(),
                                msg: e.to_string(),
                            });
                            break;
                        }
                    }
                }
            }
            let _ = ws.close(None).await;
            client.delete_tail(&account_id, &script, &tail_id).await.ok();
            let _ = tx.send(Action::TailEnded { script });
        });
    }

    /// Guarda una variable/secreto. Los secretos usan `PUT .../secrets` (aislado);
    /// las vars planas usan `PATCH .../settings` conservando el resto con `inherit`.
    fn spawn_save_binding(
        &mut self,
        script: String,
        name: String,
        is_secret: bool,
        value: String,
        _adding: bool,
    ) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        // Nombres de los demás bindings (para preservarlos con `inherit`).
        let others: Vec<String> = match &self.workers.bindings {
            Loadable::Ready(bs) => bs
                .iter()
                .map(|b| b.name.clone())
                .filter(|n| *n != name)
                .collect(),
            _ => Vec::new(),
        };
        let tx = self.action_tx.clone();
        self.status = "Guardando…".into();
        tokio::spawn(async move {
            let result = if is_secret {
                client.put_secret(&account_id, &script, &name, &value).await
            } else {
                let mut arr: Vec<serde_json::Value> =
                    vec![serde_json::json!({ "type": "plain_text", "name": name, "text": value })];
                for n in &others {
                    arr.push(serde_json::json!({ "type": "inherit", "name": n }));
                }
                client
                    .update_worker_bindings(&account_id, &script, serde_json::Value::Array(arr))
                    .await
            };
            let action = match result {
                Ok(()) => Action::BindingSaved {
                    script,
                    msg: if is_secret {
                        "Secreto guardado".into()
                    } else {
                        "Variable guardada".into()
                    },
                },
                Err(e) => Action::BindingError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    // --- D1 ---

    fn change_db(&mut self, delta: i32) {
        if self.d1.select_db(delta)
            && let Some(db_id) = self.d1.selected_db_id()
        {
            self.load_tables(db_id);
        }
    }

    fn change_table(&mut self, delta: i32) {
        if self.d1.select_table(delta) {
            self.load_table_schema();
        }
    }

    fn reload_tables(&mut self) {
        if let Some(db_id) = self.d1.selected_db_id() {
            self.load_tables(db_id);
        }
    }

    /// Ejecuta el contenido del editor SQL contra la base seleccionada.
    fn run_editor(&mut self) {
        let Some(db_id) = self.d1.selected_db_id() else {
            self.status = "Selecciona una base".into();
            return;
        };
        let sql = self.d1.sql_trimmed();
        if sql.is_empty() {
            self.status = "Escribe una consulta".into();
            return;
        }
        self.spawn_d1_query(db_id, "consulta".into(), sql);
    }

    /// `Enter` sobre una tabla: vuelca `SELECT *` en el editor y lo ejecuta.
    fn run_select(&mut self) {
        let (Some(db_id), Some(table)) = (self.d1.selected_db_id(), self.d1.selected_table())
        else {
            return;
        };
        let sql = format!("SELECT * FROM {} LIMIT 50", quote_ident(&table));
        self.d1.set_sql(sql.clone());
        self.spawn_d1_query(db_id, format!("{table} · SELECT * LIMIT 50"), sql);
    }

    fn load_table_schema(&mut self) {
        let (Some(db_id), Some(table)) = (self.d1.selected_db_id(), self.d1.selected_table())
        else {
            return;
        };
        let sql = format!("PRAGMA table_info({})", quote_ident(&table));
        self.spawn_d1_query(db_id, format!("{table} · columnas"), sql);
    }

    fn load_databases(&mut self) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.d1.loading = true;
        self.d1.error = None;
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client.list_databases(&account_id).await {
                Ok(dbs) => Action::D1DatabasesLoaded(dbs),
                Err(e) => Action::D1Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn load_tables(&mut self, db_id: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.d1.begin_tables(db_id.clone());
        let tx = self.action_tx.clone();
        let sql = "SELECT name FROM sqlite_master WHERE type IN ('table','view') \
                   AND name NOT LIKE 'sqlite_%' ORDER BY name";
        tokio::spawn(async move {
            let action = match client.d1_query(&account_id, &db_id, sql).await {
                Ok(o) => Action::D1TablesLoaded {
                    db_id,
                    tables: o.rows.into_iter().filter_map(|mut r| r.drain(..1).next()).collect(),
                },
                Err(e) => Action::D1TablesError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn spawn_d1_query(&mut self, db_id: String, title: String, sql: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.d1.begin_result();
        self.status = "Ejecutando SQL…".into();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let outcome = client
                .d1_query(&account_id, &db_id, &sql)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Action::D1ResultLoaded {
                db_id,
                title,
                outcome,
            });
        });
    }

    // --- R2 ---

    fn change_bucket(&mut self, delta: i32) {
        if self.r2.select(delta)
            && let Some(name) = self.r2.selected_name()
        {
            self.load_bucket_info(name);
        }
    }

    fn open_new_bucket(&mut self) {
        self.popup = Some(Popup::NewBucket(NewBucket::default()));
    }

    fn confirm_delete_bucket(&mut self) {
        let Some(name) = self.r2.selected_name() else {
            return;
        };
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar bucket".into(),
            body: format!("¿Borrar el bucket {name}? Debe estar vacío."),
            on_yes: Action::DeleteBucket(name),
        }));
    }

    fn load_buckets(&mut self) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.r2.loading = true;
        self.r2.error = None;
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client.list_buckets(&account_id).await {
                Ok(buckets) => Action::R2BucketsLoaded(buckets),
                Err(e) => Action::R2Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// Carga detalle + uso + dominios del bucket en una sola tarea.
    fn load_bucket_info(&mut self, name: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        self.r2.begin_info(name.clone());
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let info = match client.bucket_detail(&account_id, &name).await {
                Ok(detail) => {
                    let usage = client
                        .bucket_usage(&account_id, &name)
                        .await
                        .unwrap_or_default();
                    let domains = client
                        .bucket_domains(&account_id, &name)
                        .await
                        .unwrap_or_default();
                    Some(Box::new(BucketInfo {
                        detail,
                        usage,
                        domains,
                    }))
                }
                Err(e) => {
                    tracing::debug!("detalle bucket {name}: {e}");
                    None
                }
            };
            let _ = tx.send(Action::R2InfoLoaded { bucket: name, info });
        });
    }

    fn spawn_create_bucket(&mut self, name: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Creando bucket…".into();
        tokio::spawn(async move {
            let action = match client.create_bucket(&account_id, &name).await {
                Ok(()) => Action::R2Mutated(format!("Bucket '{name}' creado")),
                Err(e) => Action::R2Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    fn spawn_delete_bucket(&mut self, name: String) {
        let (Some(client), Some(account_id)) =
            (self.client.clone(), self.active_account_id().map(String::from))
        else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Borrando bucket…".into();
        tokio::spawn(async move {
            let action = match client.delete_bucket(&account_id, &name).await {
                Ok(()) => Action::R2Mutated(format!("Bucket '{name}' borrado")),
                Err(e) => Action::R2Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    // --- Foco ---

    fn panes(&self) -> &'static [Focus] {
        static DNS_PANES: &[Focus] = &[Focus::Modules, Focus::Zones, Focus::Records];
        static TUNNEL_PANES: &[Focus] = &[Focus::Modules, Focus::Tunnels];
        static WORKER_PANES: &[Focus] = &[Focus::Modules, Focus::Workers];
        static D1_PANES: &[Focus] = &[
            Focus::Modules,
            Focus::D1Dbs,
            Focus::D1Tables,
            Focus::D1Editor,
            Focus::D1Results,
        ];
        static R2_PANES: &[Focus] = &[Focus::Modules, Focus::R2Buckets];
        static BASE_PANES: &[Focus] = &[Focus::Modules];
        match self.sidebar.module() {
            Module::Dns => DNS_PANES,
            Module::Tunnels => TUNNEL_PANES,
            Module::Workers => WORKER_PANES,
            Module::D1 => D1_PANES,
            Module::R2 => R2_PANES,
            _ => BASE_PANES,
        }
    }

    fn cycle_focus(&mut self, back: bool) {
        let panes = self.panes();
        let cur = panes.iter().position(|p| *p == self.focus).unwrap_or(0);
        let n = panes.len();
        let next = if back {
            (cur + n - 1) % n
        } else {
            (cur + 1) % n
        };
        self.focus = panes[next];
    }

    /// Construye la ayuda contextual: globales + atajos del foco actual.
    fn build_help(&self) -> Help {
        let mut global = Vec::new();
        if self.panes().len() > 1 {
            global.push(("Tab / ⇧Tab", "cambiar de panel"));
        }
        global.push(("A", "cambiar de cuenta"));
        global.push(("?", "esta ayuda"));
        global.push(("q / Ctrl-C", "salir"));

        let mut sections = vec![HelpSection::new("Global", global)];
        sections.push(match self.focus {
            Focus::Modules => {
                HelpSection::new("Módulos", vec![("↑ ↓ / k j", "navegar módulos")])
            }
            Focus::Zones => HelpSection::new(
                "Zonas",
                vec![
                    ("↑ ↓ / k j", "navegar zonas"),
                    ("p", "purgar caché (con confirmación)"),
                    ("r", "recargar zonas"),
                ],
            ),
            Focus::Records => HelpSection::new(
                "Registros",
                vec![
                    ("↑ ↓ / k j", "navegar registros"),
                    ("a", "añadir registro"),
                    ("e", "editar registro"),
                    ("Espacio", "proxy on/off (A/AAAA/CNAME)"),
                    ("d", "borrar registro (con confirmación)"),
                    ("p", "purgar caché"),
                    ("r", "recargar registros"),
                ],
            ),
            Focus::Tunnels => HelpSection::new(
                "Túneles",
                vec![
                    ("↑ ↓ / k j", "navegar túneles"),
                    ("n", "nuevo túnel"),
                    ("c", "limpiar conexiones (con confirmación)"),
                    ("d", "borrar túnel (con confirmación)"),
                    ("r", "recargar túneles"),
                ],
            ),
            Focus::Workers => HelpSection::new(
                "Workers",
                vec![
                    ("1-4 / ←→", "pestaña (métricas/impl./vars/logs)"),
                    ("↑ ↓ / k j", "navegar workers (o variables en pestaña Vars)"),
                    ("e", "editar variable/secreto (pestaña Vars)"),
                    ("a", "añadir secreto (pestaña Vars)"),
                    ("l", "live-tail de logs on/off (pestaña Logs)"),
                    ("t", "probar una ruta (GET)"),
                    ("r", "recargar workers"),
                ],
            ),
            Focus::D1Dbs => HelpSection::new(
                "Bases D1",
                vec![
                    ("↑ ↓ / k j", "navegar bases"),
                    ("r", "recargar bases"),
                ],
            ),
            Focus::D1Tables => HelpSection::new(
                "Tablas D1",
                vec![
                    ("↑ ↓ / k j", "navegar tablas (muestra columnas)"),
                    ("Enter", "SELECT * FROM tabla LIMIT 50"),
                    ("r", "recargar tablas"),
                ],
            ),
            Focus::D1Editor => HelpSection::new(
                "Editor SQL",
                vec![
                    ("F5 / Ctrl+Enter", "ejecutar la consulta"),
                    ("Enter", "salto de línea"),
                    ("(texto)", "escribe SQL libremente"),
                ],
            ),
            Focus::D1Results => HelpSection::new(
                "Resultados",
                vec![
                    ("↑ ↓ / k j", "desplazar filas"),
                    ("PageUp/PageDown", "desplazar de 10 en 10"),
                ],
            ),
            Focus::R2Buckets => HelpSection::new(
                "Buckets R2",
                vec![
                    ("↑ ↓ / k j", "navegar buckets (uso/dominios)"),
                    ("n", "nuevo bucket"),
                    ("d", "borrar bucket (con confirmación)"),
                    ("r", "recargar buckets"),
                ],
            ),
        });
        Help { sections }
    }

    // --- Render ---

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let shell = layout::shell(area);
        let main_active = self.screen == Screen::Main;

        self.rect_sidebar = shell.sidebar;
        self.rect_zones = None;
        self.rect_records = None;
        self.rect_tunnels = None;
        self.rect_workers = None;
        self.rect_d1_dbs = None;
        self.rect_d1_tables = None;
        self.rect_d1_editor = None;
        self.rect_d1_results = None;
        self.rect_r2 = None;

        self.sidebar.draw(
            frame,
            shell.sidebar,
            main_active && self.focus == Focus::Modules,
        );

        match self.sidebar.module() {
            Module::Dns if main_active => {
                let (zones_area, records_area) = layout::dns_split(shell.main);
                self.dns
                    .draw_zones(frame, zones_area, self.focus == Focus::Zones);
                self.dns
                    .draw_records(frame, records_area, self.focus == Focus::Records);
                self.rect_zones = Some(zones_area);
                self.rect_records = Some(records_area);
            }
            Module::Tunnels if main_active => {
                let (list_area, detail_area) =
                    crate::components::tunnels::split(shell.main);
                self.tunnels
                    .draw_list(frame, list_area, self.focus == Focus::Tunnels);
                self.tunnels.draw_detail(frame, detail_area, false);
                self.rect_tunnels = Some(list_area);
            }
            Module::Workers if main_active => {
                let (list_area, detail_area) =
                    crate::components::workers::split(shell.main);
                self.workers
                    .draw_list(frame, list_area, self.focus == Focus::Workers);
                self.workers.draw_detail(frame, detail_area, false);
                self.rect_workers = Some(list_area);
            }
            Module::D1 if main_active => {
                let (dbs_area, tables_area, editor_area, result_area) =
                    crate::components::d1::split(shell.main);
                self.d1
                    .draw_dbs(frame, dbs_area, self.focus == Focus::D1Dbs);
                self.d1
                    .draw_tables(frame, tables_area, self.focus == Focus::D1Tables);
                self.d1
                    .draw_editor(frame, editor_area, self.focus == Focus::D1Editor);
                self.d1
                    .draw_result(frame, result_area, self.focus == Focus::D1Results);
                self.rect_d1_dbs = Some(dbs_area);
                self.rect_d1_tables = Some(tables_area);
                self.rect_d1_editor = Some(editor_area);
                self.rect_d1_results = Some(result_area);
            }
            Module::R2 if main_active => {
                let (list_area, detail_area) = crate::components::r2::split(shell.main);
                self.r2
                    .draw_list(frame, list_area, self.focus == Focus::R2Buckets);
                self.r2.draw_detail(frame, detail_area);
                self.rect_r2 = Some(list_area);
            }
            _ => {
                self.detail.draw(
                    frame,
                    shell.main,
                    self.sidebar.module(),
                    main_active && self.focus != Focus::Modules,
                );
            }
        }

        let (left, right) = self.status_line();
        self.command_bar.draw(frame, shell.command_bar, &left, &right);

        if let Some(popup) = &mut self.popup {
            crate::components::popup::draw(frame, area, popup);
        }
    }

    fn status_line(&self) -> (String, String) {
        let acc = self
            .accounts
            .get(self.active_account)
            .map(|a| a.name.as_str())
            .unwrap_or("");
        let left = if self.status.is_empty() {
            acc.to_string()
        } else if acc.is_empty() {
            self.status.clone()
        } else {
            format!("{acc}  ·  {}", self.status)
        };
        let right = if self.popup.is_some() {
            String::new()
        } else {
            match self.focus {
                Focus::Modules => "↑↓ módulo · Tab → · A cuenta · ? ayuda · q salir".into(),
                Focus::Zones => "↑↓ zona · Tab → · p purgar · r · A · ? ayuda".into(),
                Focus::Records => {
                    "↑↓ · Espacio proxy · a nuevo · e editar · d borrar · ? ayuda".into()
                }
                Focus::Tunnels => "↑↓ túnel · n nuevo · c limpiar · d borrar · ? ayuda".into(),
                Focus::Workers => {
                    "↑↓ · 1-4 pestaña · e editar · a secreto · l logs · t probar · ?".into()
                }
                Focus::D1Dbs => "↑↓ base · Tab → editor · r recargar · A · ? ayuda".into(),
                Focus::D1Tables => "↑↓ tabla · Enter SELECT * · Tab → editor · r · ?".into(),
                Focus::D1Editor => "escribe SQL · F5/Ctrl+Enter ejecutar · Tab → · ?".into(),
                Focus::D1Results => "↑↓ scroll · PgUp/PgDn · Tab → · ? ayuda".into(),
                Focus::R2Buckets => "↑↓ bucket · n nuevo · d borrar · r · A · ? ayuda".into(),
            }
        };
        (left, right)
    }
}

/// Entrecomilla un identificador SQL (tabla) escapando comillas dobles.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Aplica una tecla de edición estándar (←→ Inicio/Fin Supr Retroceso, texto) a un input.
fn edit_input(input: &mut TextInput, code: KeyCode) {
    match code {
        KeyCode::Left => input.left(),
        KeyCode::Right => input.right(),
        KeyCode::Home => input.home(),
        KeyCode::End => input.end(),
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        KeyCode::Char(c) => input.insert(c),
        _ => {}
    }
}
