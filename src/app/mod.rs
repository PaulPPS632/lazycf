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
    AccountPicker, AccountRow, BindingEdit, BucketDomains, ChooseDomain, ChoosePurpose,
    ConsumerEditForm, Confirm, CorsEditForm, DomainAddForm, DomainChoice, Help, HelpSection,
    HttpTest, ImageView, LogDetail, Message, PeekView, Popup, PresignForm, PromptKind,
    R2CredsForm, RField, RecordForm, RenameForm, RouteField, RouteForm, SendField,
    SendMessageForm, TextPrompt, TokenEntry, UploadForm, ZoneRef,
};
use crate::components::queues::QueuesView;
use crate::components::r2::{BucketInfo, Entry, R2View};
use crate::components::sidebar::Sidebar;
use crate::components::tunnels::TunnelsView;
use crate::components::workers::WorkersView;
use crate::ui::Loadable;
use crate::components::{Component, Module};
use crate::config::Config;
use crate::event::{Event, EventHandler};
use crate::model::{Account, Zone};
use crate::secrets;
use crate::ui::layout;

// Métodos de `App` por módulo (bloques `impl App` en archivos separados).
// El núcleo transversal (event loop, foco, popups, dispatch, draw, sesiones)
// vive aquí; cada archivo aporta los `load_*`/`spawn_*`/`open_*`/… de su módulo.
mod d1;
mod dns;
mod queues;
mod r2;
mod tunnels;
mod workers;

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
    TunnelRoutes,
    Workers,
    /// Columna 3 de Workers (detalle con pestañas).
    WorkersDetail,
    Queues,
    /// Columna 3 de Queues (detalle con pestañas).
    QueuesDetail,
    D1Dbs,
    D1Tables,
    D1Editor,
    D1Where,
    D1Results,
    R2Buckets,
    R2Objects,
    /// Barra de filtro del navegador de objetos (solo se entra con `/`).
    R2Filter,
    /// Barra de filtro de los logs de Workers (solo se entra con `/`).
    WorkersLogFilter,
}

/// Un token verificado, con su cliente HTTP y sus cuentas visibles.
/// Todas las sesiones conviven; solo una (cuenta) está activa a la vez.
struct Session {
    token: String,
    client: CfClient,
    accounts: Vec<Account>,
}

pub struct App {
    running: bool,
    screen: Screen,
    focus: Focus,
    /// Render bajo demanda: `true` cuando algo cambió desde el último frame.
    dirty: bool,
    events: EventHandler,
    action_tx: UnboundedSender<Action>,
    action_rx: UnboundedReceiver<Action>,

    // Sesiones (multi-token) y cuenta activa.
    sessions: Vec<Session>,
    active_session: usize,
    active_account: usize,
    /// Verificaciones de token en vuelo (arranque / añadir).
    pending_verifications: usize,
    config: Config,
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

    // Queues.
    queues: QueuesView,
    /// Script del consumer worker a tailear en cuanto `Workers` cargue
    /// (salto desde Queues cuando la lista de scripts aún no está lista).
    pending_tail: Option<String>,

    // D1.
    d1: D1View,

    // R2.
    r2: R2View,
    /// Objeto pendiente de URL prefirmada (a la espera de credenciales R2).
    pending_presign: Option<String>,
    /// Señal de cancelación de la búsqueda profunda en vuelo.
    search_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,

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
    rect_tunnel_routes: Option<Rect>,
    rect_workers: Option<Rect>,
    rect_workers_detail: Option<Rect>,
    rect_queues: Option<Rect>,
    rect_queues_detail: Option<Rect>,
    rect_d1_dbs: Option<Rect>,
    rect_d1_tables: Option<Rect>,
    rect_d1_editor: Option<Rect>,
    rect_d1_where: Option<Rect>,
    rect_d1_results: Option<Rect>,
    rect_r2: Option<Rect>,
    rect_r2_objects: Option<Rect>,
    rect_r2_filter: Option<Rect>,
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
            dirty: true,
            events,
            action_tx,
            action_rx,
            sessions: Vec::new(),
            active_session: 0,
            active_account: 0,
            pending_verifications: 0,
            config,
            status: String::new(),
            all_zones: Vec::new(),
            dns: DnsView::new(),
            tunnels: TunnelsView::new(),
            workers: WorkersView::new(),
            tail_stop: None,
            queues: QueuesView::new(),
            pending_tail: None,
            d1: D1View::new(),
            r2: R2View::new(),
            pending_presign: None,
            search_cancel: None,
            sidebar: Sidebar::new(),
            detail: Detail::new(),
            command_bar: CommandBar,
            popup: None,
            rect_sidebar: Rect::default(),
            rect_zones: None,
            rect_records: None,
            rect_tunnels: None,
            rect_tunnel_routes: None,
            rect_workers: None,
            rect_workers_detail: None,
            rect_queues: None,
            rect_queues_detail: None,
            rect_d1_dbs: None,
            rect_d1_tables: None,
            rect_d1_editor: None,
            rect_d1_where: None,
            rect_d1_results: None,
            rect_r2: None,
            rect_r2_objects: None,
            rect_r2_filter: None,
        };

        match secrets::load_tokens() {
            Ok(tokens) if !tokens.is_empty() => {
                app.status = format!("Verificando {} token(s)…", tokens.len());
                app.popup = Some(Popup::Token(TokenEntry {
                    input: TextInput::default(),
                    verifying: true,
                    error: None,
                }));
                app.pending_verifications = tokens.len();
                for token in tokens {
                    app.spawn_verify(token);
                }
            }
            Ok(_) => {
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

    // --- Sesiones ---

    /// Cliente HTTP de la sesión activa.
    fn client(&self) -> Option<CfClient> {
        self.sessions
            .get(self.active_session)
            .map(|s| s.client.clone())
    }

    /// Lanza una tarea async con el cliente + `account_id` de la sesión activa y
    /// una copia del canal de acciones. No-op si no hay sesión/cuenta activa.
    /// Sustituye el guard `(Some(client), Some(account_id)) = … else return`
    /// que antes se repetía en cada `load_*`/`spawn_*` account-scoped.
    fn spawn_api<F, Fut>(&self, f: F)
    where
        F: FnOnce(CfClient, String, UnboundedSender<Action>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (Some(client), Some(account_id)) =
            (self.client(), self.active_account_id().map(String::from))
        else {
            return;
        };
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            f(client, account_id, tx).await;
        });
    }

    /// Persiste todos los tokens de las sesiones en el keyring.
    fn persist_tokens(&self) {
        let tokens: Vec<String> = self.sessions.iter().map(|s| s.token.clone()).collect();
        if let Err(e) = secrets::save_tokens(&tokens) {
            tracing::warn!("no se pudieron guardar los tokens en el keyring: {e}");
        }
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
            // Render bajo demanda: el intervalo sigue a 60 Hz, pero solo se
            // reconstruyen los widgets si algo cambió desde el último frame
            // (tecla, mouse o Action). En reposo la CPU queda a ~0.
            Event::Render => {
                if self.dirty {
                    terminal.draw(|frame| self.draw(frame))?;
                    self.dirty = false;
                }
            }
            Event::Resize => {
                terminal.draw(|frame| self.draw(frame))?;
                self.dirty = false;
            }
            Event::Key(key) => {
                self.dirty = true;
                self.on_key(key);
            }
            Event::Mouse(m) => {
                self.dirty = true;
                self.on_mouse(m);
            }
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

        // Las barras de filtro (R2 objetos / logs Workers) capturan todo el
        // texto (incluidas q/?/A). Tab/Shift-Tab siguen el ciclo de paneles;
        // el filtro se mantiene aplicado al salir.
        if matches!(self.focus, Focus::R2Filter | Focus::WorkersLogFilter) {
            match key.code {
                KeyCode::Tab => self.dispatch(Action::CycleFocus { back: false }),
                KeyCode::BackTab => self.dispatch(Action::CycleFocus { back: true }),
                _ => self.route_focus_key(key),
            }
            return;
        }

        // El editor SQL y la barra WHERE capturan todo el texto (incluidas
        // q/x/?/A). Solo Tab y Shift-Tab salen del panel; el resto lo gestiona
        // route_focus_key (Enter ejecuta/aplica, texto edita el input).
        if matches!(self.focus, Focus::D1Editor | Focus::D1Where) {
            match key.code {
                // Con el popup de sugerencias abierto, Tab acepta en vez de salir.
                KeyCode::Tab if self.d1.suggestions_open() => {
                    self.d1.accept_suggestion();
                }
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
        if let Some(r) = self.rect_tunnel_routes
            && r.contains(pos)
        {
            self.focus = Focus::TunnelRoutes;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            self.tunnels.route_at(rel);
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
        if let Some(r) = self.rect_workers_detail
            && r.contains(pos)
        {
            self.focus = Focus::WorkersDetail;
            // Selecciona la fila clicada en las pestañas con lista. Offsets:
            // borde(1)+tabs(1)+separador(1) = contenido en r.y+3; en Logs se
            // suman cabecera(1)+filtro(1) = lista en r.y+5.
            match self.workers.active_tab {
                1 => {
                    let top = r.y + 3;
                    if pos.y >= top {
                        self.workers.deploy_at((pos.y - top) as usize);
                    }
                }
                3 => {
                    let top = r.y + 5;
                    if pos.y >= top {
                        self.workers.log_at((pos.y - top) as usize);
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(r) = self.rect_queues
            && r.contains(pos)
        {
            self.focus = Focus::Queues;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            if self.queues.queue_at(rel) {
                self.queues.reset_tabs();
                self.load_active_queue_tab();
            }
            return;
        }
        if let Some(r) = self.rect_queues_detail
            && r.contains(pos)
        {
            self.focus = Focus::QueuesDetail;
            // Borde(1)+tabs(1)+separador(1) = contenido en r.y+3 (solo la
            // pestaña Consumers tiene filas seleccionables).
            if self.queues.active_tab == 1 {
                let top = r.y + 3;
                if pos.y >= top {
                    self.queues.consumer_at((pos.y - top) as usize);
                }
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
        if let Some(r) = self.rect_d1_where
            && r.contains(pos)
        {
            self.focus = Focus::D1Where;
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
                self.r2.reset_browser();
                self.load_objects();
            }
            return;
        }
        if let Some(r) = self.rect_r2_objects
            && r.contains(pos)
        {
            self.focus = Focus::R2Objects;
            let rel = pos.y.saturating_sub(r.y + 1) as usize;
            // Segundo click sobre una carpeta/.. ya seleccionada → abrir.
            if !self.r2.entry_at(rel)
                && matches!(self.r2.selected_entry(), Some(Entry::Folder(_) | Entry::Up))
            {
                self.open_entry();
            }
            return;
        }
        if let Some(r) = self.rect_r2_filter
            && r.contains(pos)
        {
            self.focus = Focus::R2Filter;
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
        if let Some(r) = self.rect_tunnel_routes
            && r.contains(pos)
        {
            self.focus = Focus::TunnelRoutes;
            self.tunnels.select_route(delta);
            return;
        }
        if let Some(r) = self.rect_workers
            && r.contains(pos)
        {
            self.focus = Focus::Workers;
            self.change_worker(delta);
            return;
        }
        if let Some(r) = self.rect_workers_detail
            && r.contains(pos)
        {
            self.focus = Focus::WorkersDetail;
            self.workers_detail_nav(delta);
            return;
        }
        if let Some(r) = self.rect_queues
            && r.contains(pos)
        {
            self.focus = Focus::Queues;
            self.change_queue(delta);
            return;
        }
        if let Some(r) = self.rect_queues_detail
            && r.contains(pos)
        {
            self.focus = Focus::QueuesDetail;
            if self.queues.active_tab == 1 {
                self.queues.select_consumer(delta);
            }
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
            self.d1.move_cell(delta, 0);
            return;
        }
        if let Some(r) = self.rect_r2
            && r.contains(pos)
        {
            self.focus = Focus::R2Buckets;
            self.change_bucket(delta);
            return;
        }
        if let Some(r) = self.rect_r2_objects
            && r.contains(pos)
        {
            self.focus = Focus::R2Objects;
            // Scroll-down en la última fila con página pendiente → carga más.
            if delta > 0
                && self.r2.at_last_visible()
                && self.r2.has_more()
                && !self.r2.loading_objects
            {
                self.load_more_objects();
            } else {
                self.r2.select_entry(delta);
            }
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
                KeyCode::Char('a') => self.open_new_route(),
                KeyCode::Char('c') => self.confirm_cleanup(),
                KeyCode::Char('d') => self.confirm_delete_tunnel(),
                KeyCode::Char('r') => self.load_tunnels(),
                _ => {}
            },
            Focus::TunnelRoutes => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.tunnels.select_route(-1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.tunnels.select_route(1);
                }
                KeyCode::Char('a') => self.open_new_route(),
                KeyCode::Char('e') => self.open_edit_route(),
                KeyCode::Char('d') => self.confirm_delete_route(),
                KeyCode::Char('r') => {
                    if let Some(id) = self.tunnels.selected_id() {
                        self.load_ingress(id);
                    }
                }
                _ => {}
            },
            // Columna 2: lista de workers. ↑↓ cambia de worker; Tab → detalle.
            Focus::Workers => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_worker(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_worker(1),
                KeyCode::Left => {
                    self.workers.cycle_tab(-1);
                    self.load_active_tab();
                }
                KeyCode::Right => {
                    self.workers.cycle_tab(1);
                    self.load_active_tab();
                }
                KeyCode::Char(c @ '1'..='5') => {
                    self.workers.set_tab(c as usize - '1' as usize);
                    self.load_active_tab();
                }
                KeyCode::Char('l') => self.toggle_tail(),
                KeyCode::Char('t') => self.open_http_test(),
                KeyCode::Char('r') => self.load_workers(),
                _ => {}
            },
            // Columna 3: detalle. ↑↓ navega el contenido de la pestaña activa.
            Focus::WorkersDetail => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.workers_detail_nav(-1),
                KeyCode::Down | KeyCode::Char('j') => self.workers_detail_nav(1),
                KeyCode::Left => {
                    self.workers.cycle_tab(-1);
                    self.load_active_tab();
                }
                KeyCode::Right => {
                    self.workers.cycle_tab(1);
                    self.load_active_tab();
                }
                KeyCode::Char(c @ '1'..='5') => {
                    self.workers.set_tab(c as usize - '1' as usize);
                    self.load_active_tab();
                }
                KeyCode::Enter => self.workers_enter(),
                KeyCode::End if self.workers.active_tab == 3 => self.workers.log_follow_end(),
                KeyCode::Char('/') if self.workers.active_tab == 3 => {
                    self.focus = Focus::WorkersLogFilter;
                }
                KeyCode::Char('E') if self.workers.active_tab == 3 => {
                    self.workers.toggle_log_errors_only();
                }
                KeyCode::Char('y') if self.workers.active_tab == 3 => self.copy_log_event(),
                KeyCode::Char('e') if self.workers.active_tab == 2 => self.open_edit_binding(),
                KeyCode::Char('a') if self.workers.active_tab == 2 => self.open_add_secret(),
                KeyCode::Char('l') => self.toggle_tail(),
                KeyCode::Char('t') => self.open_http_test(),
                KeyCode::Char('r') => self.load_workers(),
                _ => {}
            },
            Focus::WorkersLogFilter => match key.code {
                KeyCode::Enter => self.focus = Focus::WorkersDetail, // fija el filtro
                KeyCode::Esc => {
                    self.workers.clear_log_filter();
                    self.focus = Focus::WorkersDetail;
                }
                code => {
                    edit_input(self.workers.log_filter_mut(), code);
                    self.workers.apply_log_filter(); // live
                }
            },
            // Columna 2: lista de colas. ↑↓ cambia de cola; Tab → detalle.
            Focus::Queues => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_queue(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_queue(1),
                KeyCode::Left => {
                    self.queues.cycle_tab(-1);
                    self.load_active_queue_tab();
                }
                KeyCode::Right => {
                    self.queues.cycle_tab(1);
                    self.load_active_queue_tab();
                }
                KeyCode::Char(c @ '1'..='3') => {
                    self.queues.set_tab(c as usize - '1' as usize);
                    self.load_active_queue_tab();
                }
                KeyCode::Char('n') => self.open_new_queue(),
                KeyCode::Char('d') => self.confirm_delete_queue(),
                KeyCode::Char('s') => self.open_send_message(),
                KeyCode::Char('p') => self.confirm_pause_toggle(),
                KeyCode::Char('P') => self.confirm_purge_queue(),
                KeyCode::Char('m') => self.open_peek(),
                KeyCode::Char('l') => self.open_consumer_logs(),
                KeyCode::Char('r') => self.load_queues(),
                _ => {}
            },
            // Columna 3: detalle. ↑↓ navega los consumers en la pestaña 2.
            Focus::QueuesDetail => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.queues.select_consumer(-1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.queues.select_consumer(1);
                }
                KeyCode::Left => {
                    self.queues.cycle_tab(-1);
                    self.load_active_queue_tab();
                }
                KeyCode::Right => {
                    self.queues.cycle_tab(1);
                    self.load_active_queue_tab();
                }
                KeyCode::Char(c @ '1'..='3') => {
                    self.queues.set_tab(c as usize - '1' as usize);
                    self.load_active_queue_tab();
                }
                KeyCode::Char('n') => self.open_new_queue(),
                KeyCode::Char('d') => self.confirm_delete_queue(),
                KeyCode::Char('s') => self.open_send_message(),
                KeyCode::Char('p') => self.confirm_pause_toggle(),
                KeyCode::Char('P') => self.confirm_purge_queue(),
                KeyCode::Char('m') => self.open_peek(),
                KeyCode::Char('l') => self.open_consumer_logs(),
                KeyCode::Char('e') | KeyCode::Enter if self.queues.active_tab == 1 => {
                    self.open_edit_consumer();
                }
                KeyCode::Char('r') => self.load_queues(),
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
                    self.d1.close_suggestions();
                    self.run_editor()
                }
                KeyCode::F(5) => {
                    self.d1.close_suggestions();
                    self.run_editor()
                }
                // Ctrl+Espacio: fuerza el popup de sugerencias.
                KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.d1.update_suggestions(true)
                }
                KeyCode::Enter if self.d1.suggestions_open() => self.d1.accept_suggestion(),
                KeyCode::Esc if self.d1.suggestions_open() => self.d1.close_suggestions(),
                KeyCode::Up if self.d1.suggestions_open() => self.d1.sug_move(-1),
                KeyCode::Down if self.d1.suggestions_open() => self.d1.sug_move(1),
                KeyCode::Enter => self.d1.editor_mut().insert('\n'),
                KeyCode::Backspace => {
                    self.d1.editor_mut().backspace();
                    self.d1.update_suggestions(false);
                }
                KeyCode::Delete => self.d1.editor_mut().delete(),
                KeyCode::Left => {
                    self.d1.close_suggestions();
                    self.d1.editor_mut().left()
                }
                KeyCode::Right => {
                    self.d1.close_suggestions();
                    self.d1.editor_mut().right()
                }
                KeyCode::Up => self.d1.editor_mut().up(),
                KeyCode::Down => self.d1.editor_mut().down(),
                KeyCode::Home => {
                    self.d1.close_suggestions();
                    self.d1.editor_mut().home()
                }
                KeyCode::End => {
                    self.d1.close_suggestions();
                    self.d1.editor_mut().end()
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.d1.editor_mut().insert(c);
                    self.d1.update_suggestions(false);
                }
                _ => {}
            },
            // Barra WHERE: mismo autocompletado que el editor (columnas del
            // resultado + keywords de cláusula).
            Focus::D1Where => match key.code {
                KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.d1.update_where_suggestions(true)
                }
                KeyCode::Enter if self.d1.suggestions_open() => self.d1.accept_suggestion(),
                KeyCode::Esc if self.d1.suggestions_open() => self.d1.close_suggestions(),
                KeyCode::Up if self.d1.suggestions_open() => self.d1.sug_move(-1),
                KeyCode::Down if self.d1.suggestions_open() => self.d1.sug_move(1),
                KeyCode::Enter => self.apply_where_filter(),
                KeyCode::Esc => self.focus = Focus::D1Results,
                code @ (KeyCode::Char(_) | KeyCode::Backspace) => {
                    edit_input(self.d1.where_mut(), code);
                    self.d1.update_where_suggestions(false);
                }
                code => {
                    self.d1.close_suggestions();
                    edit_input(self.d1.where_mut(), code);
                }
            },
            Focus::D1Results => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.d1.move_cell(-1, 0),
                KeyCode::Down | KeyCode::Char('j') => self.d1.move_cell(1, 0),
                KeyCode::Left | KeyCode::Char('h') => self.d1.move_cell(0, -1),
                KeyCode::Right | KeyCode::Char('l') => self.d1.move_cell(0, 1),
                KeyCode::PageUp => self.d1.page_rows(-10),
                KeyCode::PageDown => self.d1.page_rows(10),
                KeyCode::Enter => self.open_cell_view(),
                KeyCode::Char('y') => self.copy_cell(),
                KeyCode::Char('Y') => self.copy_row(),
                _ => {}
            },
            Focus::R2Buckets => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.change_bucket(-1),
                KeyCode::Down | KeyCode::Char('j') => self.change_bucket(1),
                KeyCode::Char('n') => self.open_new_bucket(),
                KeyCode::Char('d') => self.confirm_delete_bucket(),
                KeyCode::Char('c') => self.open_cors_edit(),
                KeyCode::Char('p') => self.confirm_toggle_public(),
                KeyCode::Char('t') => self.open_bucket_domains(),
                KeyCode::Char('r') => self.load_buckets(),
                _ => {}
            },
            Focus::R2Objects => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.r2.select_entry(-1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Última fila con página pendiente → carga más (sin envolver).
                    if self.r2.at_last_visible()
                        && self.r2.has_more()
                        && !self.r2.loading_objects
                    {
                        self.load_more_objects();
                    } else {
                        self.r2.select_entry(1);
                    }
                }
                KeyCode::Esc if self.r2.is_searching() => self.exit_search(),
                KeyCode::Enter => self.open_entry(),
                KeyCode::Backspace | KeyCode::Char('h') if self.r2.is_searching() => {
                    self.exit_search();
                }
                KeyCode::Backspace | KeyCode::Char('h') if !self.r2.prefix.is_empty() => {
                    let parent = self.r2.parent_prefix();
                    self.navigate_to(parent);
                }
                KeyCode::Char('/') => self.focus = Focus::R2Filter,
                KeyCode::Char(' ') => self.toggle_mark(),
                KeyCode::Char('s') if !self.r2.is_searching() => self.open_search(),
                KeyCode::Char('n') if !self.r2.is_searching() => self.open_new_folder(),
                KeyCode::Char('m') => self.open_move(),
                KeyCode::Char('i') => self.show_object_info(),
                KeyCode::Char('y') => self.copy_object_url(),
                KeyCode::Char('u') if !self.r2.is_searching() => self.open_upload(),
                KeyCode::Char('d') => self.spawn_download(),
                KeyCode::Char('x') => self.confirm_delete_object(),
                KeyCode::Char('p') => self.open_presign(),
                KeyCode::Char('o') => self.open_object_browser(),
                KeyCode::Char('e') => self.open_rename(),
                KeyCode::Char('v') => self.spawn_preview(),
                KeyCode::Char('r') => {
                    // En búsqueda, `r` relanza el mismo término.
                    if let Some(t) = self.r2.search_term().map(String::from) {
                        self.start_deep_search(t);
                    } else {
                        self.load_objects();
                    }
                }
                _ => {}
            },
            Focus::R2Filter => match key.code {
                KeyCode::Enter => self.focus = Focus::R2Objects, // fija el filtro
                KeyCode::Esc => {
                    self.r2.clear_filter();
                    self.focus = Focus::R2Objects;
                }
                code => {
                    edit_input(self.r2.filter_mut(), code);
                    self.r2.apply_filter(); // live: recalcula en cada tecla
                }
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
            Module::Queues if self.queues.is_empty() && !self.queues.loading => {
                self.load_queues();
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
            TextPrompt,
            RecordForm,
            RouteForm,
            HttpTest,
            BindingEdit,
            Upload,
            Rename,
            R2Creds,
            Presign,
            CorsEdit,
            ChooseDomain,
            BucketDomains,
            DomainAdd,
            LogDetail,
            SendMessage,
            ConsumerEdit,
            PeekView,
            Message,
        }
        let kind = match self.popup.as_ref()? {
            Popup::Token(_) => Kind::Token,
            Popup::Confirm(_) => Kind::Confirm,
            Popup::AccountPicker(_) => Kind::Account,
            Popup::Help(_) => Kind::Help,
            Popup::TextPrompt(_) => Kind::TextPrompt,
            Popup::RecordForm(_) => Kind::RecordForm,
            Popup::RouteForm(_) => Kind::RouteForm,
            Popup::HttpTest(_) => Kind::HttpTest,
            Popup::BindingEdit(_) => Kind::BindingEdit,
            Popup::Upload(_) => Kind::Upload,
            Popup::Rename(_) => Kind::Rename,
            Popup::R2Creds(_) => Kind::R2Creds,
            Popup::Presign(_) => Kind::Presign,
            Popup::CorsEdit(_) => Kind::CorsEdit,
            Popup::ChooseDomain(_) => Kind::ChooseDomain,
            Popup::BucketDomains(_) => Kind::BucketDomains,
            Popup::DomainAdd(_) => Kind::DomainAdd,
            Popup::LogDetail(_) => Kind::LogDetail,
            Popup::SendMessage(_) => Kind::SendMessage,
            Popup::ConsumerEdit(_) => Kind::ConsumerEdit,
            Popup::PeekView(_) => Kind::PeekView,
            // El visor de imagen se cierra con cualquier tecla, como Help.
            Popup::ImageView(_) => Kind::Help,
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
                    KeyCode::Enter => {
                        // Fila "(sin cuentas)" no es seleccionable como activa.
                        let row = p.selected_row()?;
                        if row.account == usize::MAX {
                            return None;
                        }
                        Some(Action::SwitchTo {
                            session: row.session,
                            account: row.account,
                        })
                    }
                    // Añadir un token nuevo: abre la entrada de token encima.
                    KeyCode::Char('a') => {
                        self.popup = Some(Popup::Token(TokenEntry::default()));
                        None
                    }
                    // Eliminar el token de la fila seleccionada (con confirmación).
                    KeyCode::Char('d' | 'x') => {
                        let row = p.selected_row()?;
                        let session = row.session;
                        let suffix = self
                            .sessions
                            .get(session)
                            .map(|s| mask_token(&s.token))
                            .unwrap_or_default();
                        let n = self
                            .sessions
                            .get(session)
                            .map(|s| s.accounts.len())
                            .unwrap_or(0);
                        self.popup = Some(Popup::Confirm(Confirm {
                            title: "Eliminar token".into(),
                            body: format!(
                                "¿Eliminar el token {suffix} y sus {n} cuenta(s) de lazycf?\n(No borra nada en Cloudflare.)"
                            ),
                            on_yes: Action::DeleteToken(session),
                        }));
                        None
                    }
                    KeyCode::Esc => {
                        self.popup = None;
                        None
                    }
                    _ => None,
                }
            }
            Kind::TextPrompt => self.text_prompt_key(key),
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
            Kind::RouteForm => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::RouteForm(f)) if f.submitting) {
                    return None;
                }
                if key.code == KeyCode::Enter {
                    if let Some(Popup::RouteForm(f)) = self.popup.as_mut() {
                        let Some(hostname) = f.full_hostname() else {
                            f.error = Some("Selecciona un dominio (no hay zonas)".into());
                            return None;
                        };
                        if f.service.value().trim().is_empty() {
                            f.error = Some("La URL del servicio es obligatoria".into());
                            return None;
                        }
                        let service = f.service.value().trim().to_string();
                        let tunnel_id = f.tunnel_id.clone();
                        f.error = None;
                        f.submitting = true;
                        // Editar (hostname fijo) vs crear (nueva regla + CNAME).
                        if f.editing.is_some() {
                            return Some(Action::EditTunnelRoute {
                                tunnel_id,
                                hostname,
                                service,
                                path: f.path.take(),
                            });
                        }
                        let dns_zone = f.zone().map(|z| z.id.clone());
                        return Some(Action::AddTunnelRoute {
                            tunnel_id,
                            hostname,
                            service,
                            path: f.path.take(),
                            dns_zone,
                        });
                    }
                    return None;
                }
                if let Some(Popup::RouteForm(f)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab => f.move_field(-1),
                        KeyCode::Down | KeyCode::Tab => f.move_field(1),
                        KeyCode::Left if f.current() == RouteField::Domain => f.cycle_zone(-1),
                        KeyCode::Right if f.current() == RouteField::Domain => f.cycle_zone(1),
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
            Kind::Upload => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::Upload(u)) if u.submitting) {
                    return None;
                }
                if key.code == KeyCode::Enter {
                    let path = match self.popup.as_ref() {
                        Some(Popup::Upload(u)) => u.path.value().trim().to_string(),
                        _ => return None,
                    };
                    if let Some(Popup::Upload(u)) = self.popup.as_mut() {
                        if path.is_empty() {
                            u.error = Some("La ruta es obligatoria".into());
                            return None;
                        }
                        u.error = None;
                        u.submitting = true;
                    }
                    return Some(Action::UploadObject { path });
                }
                if let Some(Popup::Upload(u)) = self.popup.as_mut() {
                    edit_input(&mut u.path, key.code);
                }
                None
            }
            Kind::Rename => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::Rename(r)) if r.submitting) {
                    return None;
                }
                if key.code == KeyCode::Enter {
                    let (old_key, name, move_mode) = match self.popup.as_ref() {
                        Some(Popup::Rename(r)) => (
                            r.old_key.clone(),
                            r.name.value().trim().to_string(),
                            r.move_mode,
                        ),
                        _ => return None,
                    };
                    if name.is_empty() {
                        if let Some(Popup::Rename(r)) = self.popup.as_mut() {
                            r.error = Some(if move_mode {
                                "La clave es obligatoria".into()
                            } else {
                                "El nombre es obligatorio".into()
                            });
                        }
                        return None;
                    }
                    // Mover edita la clave completa; renombrar conserva la carpeta.
                    let new_key = if move_mode {
                        name.trim_start_matches('/').to_string()
                    } else {
                        let prefix = match old_key.rfind('/') {
                            Some(i) => &old_key[..=i],
                            None => "",
                        };
                        format!("{prefix}{name}")
                    };
                    if move_mode && (new_key.is_empty() || new_key.ends_with('/')) {
                        if let Some(Popup::Rename(r)) = self.popup.as_mut() {
                            r.error = Some("La clave no puede terminar en '/'".into());
                        }
                        return None;
                    }
                    if new_key == old_key {
                        self.popup = None;
                        return None;
                    }
                    if self.r2.key_exists(&new_key) {
                        if let Some(Popup::Rename(r)) = self.popup.as_mut() {
                            r.error = Some("Ya existe un objeto con ese nombre".into());
                        }
                        return None;
                    }
                    let content_type = self
                        .r2
                        .selected_file()
                        .filter(|f| f.key == old_key)
                        .and_then(|f| f.http_metadata.as_ref())
                        .and_then(|m| m.content_type.clone());
                    if let Some(Popup::Rename(r)) = self.popup.as_mut() {
                        r.error = None;
                        r.submitting = true;
                    }
                    return Some(Action::RenameObject {
                        old_key,
                        new_key,
                        content_type,
                    });
                }
                if let Some(Popup::Rename(r)) = self.popup.as_mut() {
                    edit_input(&mut r.name, key.code);
                }
                None
            }
            Kind::R2Creds => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    self.pending_presign = None;
                    return None;
                }
                if key.code == KeyCode::Enter {
                    let (ak, sk) = match self.popup.as_ref() {
                        Some(Popup::R2Creds(c)) => (
                            c.access_key.value().trim().to_string(),
                            c.secret.value().trim().to_string(),
                        ),
                        _ => return None,
                    };
                    if ak.is_empty() || sk.is_empty() {
                        if let Some(Popup::R2Creds(c)) = self.popup.as_mut() {
                            c.error = Some("Access Key y Secret son obligatorios".into());
                        }
                        return None;
                    }
                    self.popup = None;
                    return Some(Action::SaveR2Creds {
                        access_key: ak,
                        secret: sk,
                    });
                }
                if let Some(Popup::R2Creds(c)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab | KeyCode::Down | KeyCode::Tab => {
                            c.field = 1 - c.field;
                        }
                        code => {
                            let input = if c.field == 0 {
                                &mut c.access_key
                            } else {
                                &mut c.secret
                            };
                            edit_input(input, code);
                        }
                    }
                }
                None
            }
            Kind::Presign => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if key.code == KeyCode::Enter {
                    let (key_obj, expires) = match self.popup.as_ref() {
                        Some(Popup::Presign(p)) => {
                            (p.key.clone(), p.expires.value().trim().parse::<u64>())
                        }
                        _ => return None,
                    };
                    match expires {
                        Ok(secs @ 1..=604_800) => {
                            self.popup = None;
                            return Some(Action::GeneratePresign {
                                key: key_obj,
                                expires: secs,
                            });
                        }
                        _ => {
                            if let Some(Popup::Presign(p)) = self.popup.as_mut() {
                                p.error = Some("Segundos entre 1 y 604800 (7 días)".into());
                            }
                            return None;
                        }
                    }
                }
                if let Some(Popup::Presign(p)) = self.popup.as_mut() {
                    edit_input(&mut p.expires, key.code);
                }
                None
            }
            Kind::CorsEdit => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::CorsEdit(c)) if c.submitting) {
                    return None;
                }
                let save = key.code == KeyCode::F(5)
                    || (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL));
                if save {
                    let (bucket, text) = match self.popup.as_ref() {
                        Some(Popup::CorsEdit(c)) => (c.bucket.clone(), c.json.value().to_string()),
                        _ => return None,
                    };
                    match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(v) if v.is_array() => {
                            if let Some(Popup::CorsEdit(c)) = self.popup.as_mut() {
                                c.error = None;
                                c.submitting = true;
                            }
                            return Some(Action::SaveCors { bucket, rules: v });
                        }
                        Ok(_) => {
                            if let Some(Popup::CorsEdit(c)) = self.popup.as_mut() {
                                c.error = Some("Debe ser un array JSON de reglas".into());
                            }
                        }
                        Err(e) => {
                            if let Some(Popup::CorsEdit(c)) = self.popup.as_mut() {
                                c.error = Some(format!("JSON inválido: {e}"));
                            }
                        }
                    }
                    return None;
                }
                if let Some(Popup::CorsEdit(c)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Enter => c.json.insert('\n'),
                        KeyCode::Backspace => c.json.backspace(),
                        KeyCode::Delete => c.json.delete(),
                        KeyCode::Left => c.json.left(),
                        KeyCode::Right => c.json.right(),
                        KeyCode::Up => c.json.up(),
                        KeyCode::Down => c.json.down(),
                        KeyCode::Home => c.json.home(),
                        KeyCode::End => c.json.end(),
                        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            c.json.insert(ch)
                        }
                        _ => {}
                    }
                }
                None
            }
            Kind::ChooseDomain => {
                match key.code {
                    KeyCode::Esc => self.popup = None,
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(Popup::ChooseDomain(c)) = self.popup.as_mut() {
                            c.move_by(-1);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(Popup::ChooseDomain(c)) = self.popup.as_mut() {
                            c.move_by(1);
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(Popup::ChooseDomain(c)) = self.popup.as_ref()
                            && let Some(choice) = c.selected()
                        {
                            let url = crate::api::r2::object_url(&choice.domain, &c.key);
                            let purpose = c.purpose;
                            self.popup = None;
                            match purpose {
                                ChoosePurpose::Abrir => self.open_url_in_browser(url),
                                ChoosePurpose::Copiar => {
                                    crate::tui::osc52_copy(&url);
                                    self.status = format!("URL copiada: {url}");
                                }
                            }
                        }
                    }
                    _ => {}
                }
                None
            }
            Kind::BucketDomains => {
                match key.code {
                    KeyCode::Esc => self.popup = None,
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(Popup::BucketDomains(d)) = self.popup.as_mut() {
                            d.move_by(-1);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(Popup::BucketDomains(d)) = self.popup.as_mut() {
                            d.move_by(1);
                        }
                    }
                    KeyCode::Char('a') => {
                        let bucket = match self.popup.as_ref() {
                            Some(Popup::BucketDomains(d)) => d.bucket.clone(),
                            _ => return None,
                        };
                        // Las zonas pueden no estar cargadas si no se pasó por DNS.
                        if self.all_zones.is_empty() {
                            self.load_zones();
                        }
                        let zones = self.account_zone_refs();
                        self.popup = Some(Popup::DomainAdd(DomainAddForm::new(bucket, zones)));
                    }
                    KeyCode::Char('d' | 'x') => {
                        if let Some(Popup::BucketDomains(d)) = self.popup.as_ref()
                            && let Some(dom) = d.selected()
                        {
                            let bucket = d.bucket.clone();
                            let domain = dom.domain.clone();
                            self.popup = Some(Popup::Confirm(Confirm {
                                title: "Quitar dominio".into(),
                                body: format!(
                                    "¿Desconectar {domain} del bucket {bucket}?\n(No borra la zona ni sus registros DNS.)"
                                ),
                                on_yes: Action::RemoveCustomDomain { bucket, domain },
                            }));
                        }
                    }
                    _ => {}
                }
                None
            }
            Kind::DomainAdd => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::DomainAdd(f)) if f.submitting) {
                    return None;
                }
                if key.code == KeyCode::Enter {
                    if let Some(Popup::DomainAdd(f)) = self.popup.as_mut() {
                        let Some(domain) = f.full_domain() else {
                            f.error = Some("Selecciona un dominio (no hay zonas)".into());
                            return None;
                        };
                        let zone_id = f.zone()?.id.clone();
                        let bucket = f.bucket.clone();
                        f.error = None;
                        f.submitting = true;
                        return Some(Action::AddCustomDomain {
                            bucket,
                            domain,
                            zone_id,
                        });
                    }
                    return None;
                }
                if let Some(Popup::DomainAdd(f)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab => f.move_field(-1),
                        KeyCode::Down | KeyCode::Tab => f.move_field(1),
                        KeyCode::Left if f.field == 1 => f.cycle_zone(-1),
                        KeyCode::Right if f.field == 1 => f.cycle_zone(1),
                        code => {
                            if f.field == 0 {
                                edit_input(&mut f.subdomain, code);
                            }
                        }
                    }
                }
                None
            }
            Kind::LogDetail => {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => self.popup = None,
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(Popup::LogDetail(d)) = self.popup.as_mut() {
                            d.scroll = d.scroll.saturating_sub(1);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(Popup::LogDetail(d)) = self.popup.as_mut() {
                            let max = d.lines.len().saturating_sub(1) as u16;
                            d.scroll = (d.scroll + 1).min(max);
                        }
                    }
                    KeyCode::PageUp => {
                        if let Some(Popup::LogDetail(d)) = self.popup.as_mut() {
                            d.scroll = d.scroll.saturating_sub(10);
                        }
                    }
                    KeyCode::PageDown => {
                        if let Some(Popup::LogDetail(d)) = self.popup.as_mut() {
                            let max = d.lines.len().saturating_sub(1) as u16;
                            d.scroll = (d.scroll + 10).min(max);
                        }
                    }
                    KeyCode::Char('y') => {
                        if let Some(Popup::LogDetail(d)) = self.popup.as_ref() {
                            crate::tui::osc52_copy(&d.raw);
                            self.status = "Evento copiado al portapapeles".into();
                        }
                    }
                    _ => {}
                }
                None
            }
            Kind::SendMessage => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::SendMessage(f)) if f.submitting) {
                    return None;
                }
                let send = key.code == KeyCode::F(5)
                    || (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL));
                if send {
                    let (queue_id, body, content_type, delay_text) = match self.popup.as_ref() {
                        Some(Popup::SendMessage(f)) => (
                            f.queue_id.clone(),
                            f.body.value().to_string(),
                            f.content_type().to_string(),
                            f.delay.value().trim().to_string(),
                        ),
                        _ => return None,
                    };
                    if body.trim().is_empty() {
                        if let Some(Popup::SendMessage(f)) = self.popup.as_mut() {
                            f.error = Some("El cuerpo del mensaje es obligatorio".into());
                        }
                        return None;
                    }
                    if content_type == "json" && serde_json::from_str::<serde_json::Value>(&body).is_err() {
                        if let Some(Popup::SendMessage(f)) = self.popup.as_mut() {
                            f.error = Some("JSON inválido".into());
                        }
                        return None;
                    }
                    let delay_seconds = if delay_text.is_empty() {
                        None
                    } else {
                        match delay_text.parse::<u64>() {
                            Ok(d) => Some(d),
                            Err(_) => {
                                if let Some(Popup::SendMessage(f)) = self.popup.as_mut() {
                                    f.error = Some("Delay debe ser un número de segundos".into());
                                }
                                return None;
                            }
                        }
                    };
                    if let Some(Popup::SendMessage(f)) = self.popup.as_mut() {
                        f.error = None;
                        f.submitting = true;
                    }
                    return Some(Action::SendMessage {
                        queue_id,
                        body,
                        content_type,
                        delay_seconds,
                    });
                }
                if let Some(Popup::SendMessage(f)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Tab => f.move_field(1),
                        KeyCode::BackTab => f.move_field(-1),
                        KeyCode::Left if f.current() == SendField::ContentType => {
                            f.cycle_content_type(-1)
                        }
                        KeyCode::Right if f.current() == SendField::ContentType => {
                            f.cycle_content_type(1)
                        }
                        code if f.current() == SendField::Body => {
                            match code {
                                KeyCode::Enter => f.body.insert('\n'),
                                KeyCode::Backspace => f.body.backspace(),
                                KeyCode::Delete => f.body.delete(),
                                KeyCode::Left => f.body.left(),
                                KeyCode::Right => f.body.right(),
                                KeyCode::Up => f.body.up(),
                                KeyCode::Down => f.body.down(),
                                KeyCode::Home => f.body.home(),
                                KeyCode::End => f.body.end(),
                                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    f.body.insert(ch)
                                }
                                _ => {}
                            }
                        }
                        code if f.current() == SendField::Delay => {
                            edit_input(&mut f.delay, code);
                        }
                        _ => {}
                    }
                }
                None
            }
            Kind::ConsumerEdit => {
                if key.code == KeyCode::Esc {
                    self.popup = None;
                    return None;
                }
                if matches!(self.popup.as_ref(), Some(Popup::ConsumerEdit(f)) if f.submitting) {
                    return None;
                }
                if key.code == KeyCode::Enter {
                    let (queue_id, consumer_id, body) = match self.popup.as_ref() {
                        Some(Popup::ConsumerEdit(f)) => match f.to_body() {
                            Ok(b) => (f.queue_id.clone(), f.consumer_id.clone(), b),
                            Err(e) => {
                                if let Some(Popup::ConsumerEdit(f)) = self.popup.as_mut() {
                                    f.error = Some(e);
                                }
                                return None;
                            }
                        },
                        _ => return None,
                    };
                    if let Some(Popup::ConsumerEdit(f)) = self.popup.as_mut() {
                        f.error = None;
                        f.submitting = true;
                    }
                    return Some(Action::UpdateConsumer {
                        queue_id,
                        consumer_id,
                        body,
                    });
                }
                if let Some(Popup::ConsumerEdit(f)) = self.popup.as_mut() {
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab => f.move_field(-1),
                        KeyCode::Down | KeyCode::Tab => f.move_field(1),
                        code => edit_input(f.active_text_mut(), code),
                    }
                }
                None
            }
            Kind::PeekView => match key.code {
                KeyCode::Esc => {
                    self.popup = None;
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(Popup::PeekView(v)) = self.popup.as_mut() {
                        v.move_by(-1);
                    }
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(Popup::PeekView(v)) = self.popup.as_mut() {
                        v.move_by(1);
                    }
                    None
                }
                KeyCode::Enter => {
                    if let Some(Popup::PeekView(v)) = self.popup.as_ref()
                        && let Some(msg) = v.selected()
                    {
                        let ts = msg
                            .timestamp_ms
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map(|d| d.to_rfc3339())
                            .unwrap_or_else(|| "—".into());
                        let lines = vec![
                            format!("ID: {}", msg.id),
                            format!("Timestamp: {ts}"),
                            format!("Intentos: {}", msg.attempts),
                            String::new(),
                            msg.body.clone(),
                        ];
                        let raw = msg.body.clone();
                        self.popup = Some(Popup::LogDetail(LogDetail {
                            title: format!("msg {}", msg.id),
                            lines,
                            raw,
                            scroll: 0,
                        }));
                    }
                    None
                }
                _ => None,
            },
            Kind::Help | Kind::Message => {
                self.popup = None;
                None
            }
        }
    }

    /// Teclado del `TextPrompt` unificado (nuevo túnel/bucket/cola/carpeta,
    /// búsqueda). Valida según el `PromptKind` y emite la `Action` que toca.
    fn text_prompt_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                self.popup = None;
                None
            }
            KeyCode::Enter => {
                let Some(Popup::TextPrompt(p)) = self.popup.as_ref() else {
                    return None;
                };
                let kind = p.kind;
                let value = p.input.value().trim().to_string();
                // Validación por tipo; devuelve la Action o un mensaje de error.
                let result: Result<Action, String> = if value.is_empty() {
                    Err(if kind == PromptKind::Search {
                        "El término es obligatorio".into()
                    } else {
                        "El nombre es obligatorio".into()
                    })
                } else {
                    match kind {
                        PromptKind::NewTunnel => Ok(Action::CreateTunnel(value)),
                        PromptKind::NewBucket => Ok(Action::CreateBucket(value)),
                        PromptKind::NewQueue => Ok(Action::CreateQueue(value)),
                        PromptKind::Search => Ok(Action::SearchObjects { term: value }),
                        PromptKind::NewFolder => {
                            if value.contains('/') {
                                Err("El nombre no puede contener '/'".into())
                            } else if self.r2.folder_exists(&format!("{}{value}/", self.r2.prefix)) {
                                Err("Ya existe esa carpeta".into())
                            } else {
                                Ok(Action::CreateFolder { name: value })
                            }
                        }
                    }
                };
                match result {
                    Ok(action) => {
                        self.popup = None;
                        Some(action)
                    }
                    Err(e) => {
                        if let Some(Popup::TextPrompt(p)) = self.popup.as_mut() {
                            p.error = Some(e);
                        }
                        None
                    }
                }
            }
            code => {
                if let Some(Popup::TextPrompt(p)) = self.popup.as_mut() {
                    edit_input(&mut p.input, code);
                }
                None
            }
        }
    }

    // --- Despacho de acciones ---

    fn dispatch(&mut self, action: Action) {
        // Toda Action puede mutar estado visible (incluye el live-tail que
        // llega por el canal): marca el frame como sucio.
        self.dirty = true;
        match action {
            Action::Quit => self.running = false,
            Action::CycleFocus { back } => self.cycle_focus(back),

            Action::SubmitToken(token) => {
                self.status = "Verificando token…".into();
                self.spawn_verify(token);
            }
            Action::TokenVerified { token, accounts } => {
                self.pending_verifications = self.pending_verifications.saturating_sub(1);
                if self.sessions.iter().any(|s| s.token == token) {
                    self.status = "Ese token ya está añadido".into();
                    if self.screen == Screen::Main
                        && matches!(self.popup, Some(Popup::Token(_)))
                    {
                        self.popup = None;
                    }
                } else if let Ok(client) = CfClient::new(token.clone()) {
                    let n = accounts.len();
                    self.sessions.push(Session {
                        token,
                        client,
                        accounts,
                    });
                    self.persist_tokens();
                    if self.screen == Screen::Auth {
                        // Primera sesión válida → entrar a la app.
                        let si = self.sessions.len() - 1;
                        self.active_session = si;
                        self.active_account = self
                            .config
                            .default_account_id
                            .as_ref()
                            .and_then(|id| {
                                self.sessions[si].accounts.iter().position(|a| &a.id == id)
                            })
                            .unwrap_or(0);
                        self.screen = Screen::Main;
                        self.popup = None;
                        self.status = "Autenticado".into();
                        self.load_zones();
                    } else {
                        // Token añadido desde el selector: reabrirlo actualizado.
                        self.status = format!("Token añadido ({n} cuenta(s))");
                        self.open_account_picker();
                    }
                }
            }
            Action::AuthFailed(msg) => {
                self.pending_verifications = self.pending_verifications.saturating_sub(1);
                // Solo bloquea si no queda ninguna sesión válida ni verificación en vuelo.
                if self.sessions.is_empty() && self.pending_verifications == 0 {
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
                } else if self.screen == Screen::Main
                    && let Some(Popup::Token(entry)) = self.popup.as_mut()
                {
                    // Fallo al añadir un token desde el selector.
                    entry.verifying = false;
                    entry.error = Some(msg);
                } else {
                    self.status = format!("Token inválido: {msg}");
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

            Action::OpenAccountPicker => self.open_account_picker(),
            Action::SwitchTo { session, account } => {
                self.popup = None;
                let valid = self
                    .sessions
                    .get(session)
                    .is_some_and(|s| account < s.accounts.len());
                if !valid {
                    return;
                }
                if session == self.active_session && account == self.active_account {
                    return; // ya activa
                }
                let session_changed = session != self.active_session;
                self.active_session = session;
                self.active_account = account;
                self.status = format!(
                    "Cuenta: {}",
                    self.sessions[session].accounts[account].name
                );
                // Recursos account-scoped: resetear y recargar si procede.
                self.stop_tail();
                self.tunnels.reset();
                self.workers.reset();
                self.workers.set_subdomain(None);
                self.queues.reset();
                self.pending_tail = None;
                self.d1.reset();
                self.r2.reset();
                if session_changed {
                    // Las zonas pertenecen al token: recargar con el nuevo cliente.
                    self.all_zones.clear();
                    self.load_zones();
                } else {
                    self.apply_account_filter();
                }
                match self.sidebar.module() {
                    Module::Tunnels => self.load_tunnels(),
                    Module::Workers => self.load_workers(),
                    Module::Queues => self.load_queues(),
                    Module::D1 => self.load_databases(),
                    Module::R2 => self.load_buckets(),
                    _ => {}
                }
            }
            Action::DeleteToken(session) => {
                if session >= self.sessions.len() {
                    return;
                }
                self.sessions.remove(session);
                self.persist_tokens();
                if self.sessions.is_empty() {
                    // Sin tokens: volver a la pantalla de autenticación.
                    self.active_session = 0;
                    self.active_account = 0;
                    self.stop_tail();
                    self.all_zones.clear();
                    self.dns = DnsView::new();
                    self.tunnels.reset();
                    self.workers.reset();
                    self.queues.reset();
                    self.pending_tail = None;
                    self.d1.reset();
                    self.r2.reset();
                    self.screen = Screen::Auth;
                    self.status = "Sin tokens · introduce uno nuevo".into();
                    self.popup = Some(Popup::Token(TokenEntry::default()));
                    return;
                }
                let active_removed = self.active_session == session;
                if self.active_session > session {
                    self.active_session -= 1;
                } else if active_removed {
                    // La sesión activa se borró: pasar a la primera restante.
                    self.active_session = 0;
                    self.active_account = 0;
                    self.stop_tail();
                    self.tunnels.reset();
                    self.workers.reset();
                    self.workers.set_subdomain(None);
                    self.queues.reset();
                    self.pending_tail = None;
                    self.d1.reset();
                    self.r2.reset();
                    self.all_zones.clear();
                    self.load_zones();
                    match self.sidebar.module() {
                        Module::Tunnels => self.load_tunnels(),
                        Module::Workers => self.load_workers(),
                        Module::Queues => self.load_queues(),
                        Module::D1 => self.load_databases(),
                        Module::R2 => self.load_buckets(),
                        _ => {}
                    }
                }
                self.status = "Token eliminado".into();
                self.open_account_picker();
            }

            Action::ZonesLoaded(zones) => {
                self.all_zones = zones;
                self.apply_account_filter();
                // Si hay un form esperando zonas (ruta o dominio R2), rellénalo.
                let refs = self.account_zone_refs();
                match self.popup.as_mut() {
                    Some(Popup::RouteForm(f)) => f.set_zones(refs),
                    Some(Popup::DomainAdd(f)) => f.set_zones(refs),
                    _ => {}
                }
                // Si la pestaña Rutas de Workers esperaba las zonas, reintenta.
                if self.sidebar.module() == Module::Workers
                    && self.workers.active_tab == 4
                    && self.workers.routing.is_idle()
                    && let Some(script) = self.workers.selected_name()
                {
                    self.load_routing(script);
                }
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
            Action::AddTunnelRoute {
                tunnel_id,
                hostname,
                service,
                path,
                dns_zone,
            } => self.spawn_add_route(tunnel_id, hostname, service, path, dns_zone),
            Action::EditTunnelRoute {
                tunnel_id,
                hostname,
                service,
                path,
            } => self.spawn_edit_route(tunnel_id, hostname, service, path),
            Action::DeleteTunnelRoute {
                tunnel_id,
                hostname,
            } => self.spawn_delete_route(tunnel_id, hostname),
            Action::TunnelRouteMutated(msg) => {
                self.status = msg;
                // Cierra el form de ruta (si venía de ahí) y recarga solo las
                // rutas del túnel actual, conservando la selección.
                if matches!(self.popup, Some(Popup::RouteForm(_))) {
                    self.popup = None;
                }
                if let Some(tunnel_id) = self.tunnels.selected_id() {
                    self.load_ingress(tunnel_id);
                }
            }
            Action::TunnelRouteError(e) => {
                // Mantén el formulario abierto para corregir sin re-escribir.
                if let Some(Popup::RouteForm(f)) = self.popup.as_mut() {
                    f.submitting = false;
                    f.error = Some(e);
                } else {
                    self.status = format!("Error: {e}");
                }
            }
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
                if let Some(script) = self.pending_tail.take() {
                    if self.workers.select_by_name(&script) {
                        self.workers.reset_tabs();
                        self.focus = Focus::WorkersDetail;
                        self.dispatch(Action::StartTail(script));
                        return;
                    } else {
                        self.status = format!("Worker '{script}' no está en la lista");
                    }
                }
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
                self.pending_tail = None;
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
                    self.workers
                        .push_event(crate::api::workers::TailEvent::info("· conectado"));
                }
            }
            Action::TailPush { script, event } => {
                if self.workers.tailing
                    && self.workers.selected_name().as_deref() == Some(script.as_str())
                {
                    self.workers.push_event(event);
                }
            }
            Action::TailError { script, msg } => {
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers
                        .push_event(crate::api::workers::TailEvent::error(format!("✗ {msg}")));
                    self.status = format!("Tail: {msg}");
                }
            }
            Action::TailEnded { script } => {
                self.tail_stop = None;
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers.set_tailing(false);
                    self.workers
                        .push_event(crate::api::workers::TailEvent::info("· tail finalizado"));
                }
            }
            Action::RoutingLoaded { script, routing } => {
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    self.workers.set_routing(routing);
                }
            }
            Action::RollbackDeployment { script, versions } => {
                self.spawn_rollback(script, versions);
            }
            Action::DeploymentRolledBack { script, msg } => {
                self.status = msg;
                if self.workers.selected_name().as_deref() == Some(script.as_str()) {
                    // Recarga: el deployment revertido pasa a ser el activo (índice 0).
                    self.load_deployments(script);
                }
            }
            Action::RollbackError(e) => self.status = format!("Rollback: {e}"),

            // --- Queues (Fase 4) ---
            Action::QueuesLoaded(qs) => {
                self.queues.set_queues(qs);
                self.load_active_queue_tab();
            }
            Action::QueueError(e) => {
                self.queues.loading = false;
                self.queues.error = Some(e.clone());
                self.status = format!("Error: {e}");
            }
            Action::QueueMutated(msg) => {
                self.status = msg;
                self.load_queues();
            }
            Action::CreateQueue(name) => self.spawn_create_queue(name),
            Action::DeleteQueue { queue_id } => self.spawn_delete_queue(queue_id),
            Action::PauseQueue {
                queue_id,
                queue_name,
                paused,
            } => self.spawn_pause_queue(queue_id, queue_name, paused),
            Action::PurgeQueue { queue_id } => self.spawn_purge_queue(queue_id),
            Action::SendMessage {
                queue_id,
                body,
                content_type,
                delay_seconds,
            } => self.spawn_send_message(queue_id, body, content_type, delay_seconds),
            Action::MessageSent(msg) => {
                self.status = msg;
                if matches!(self.popup, Some(Popup::SendMessage(_))) {
                    self.popup = None;
                }
                if self.queues.active_tab == 2
                    && let Some(id) = self.queues.selected_id()
                {
                    self.load_queue_metrics(id);
                }
            }
            Action::SendMessageError(e) => {
                if let Some(Popup::SendMessage(f)) = self.popup.as_mut() {
                    f.submitting = false;
                    f.error = Some(e);
                } else {
                    self.status = format!("Error: {e}");
                }
            }
            Action::ConsumersLoaded { queue_id, consumers } => {
                if self.queues.selected_id().as_deref() == Some(queue_id.as_str()) {
                    self.queues.set_consumers(consumers);
                }
            }
            Action::UpdateConsumer {
                queue_id,
                consumer_id,
                body,
            } => self.spawn_update_consumer(queue_id, consumer_id, body),
            Action::ConsumerSaved { queue_id, msg } => {
                self.status = msg;
                if matches!(self.popup, Some(Popup::ConsumerEdit(_))) {
                    self.popup = None;
                }
                self.load_queue_consumers(queue_id);
            }
            Action::ConsumerError(e) => {
                if let Some(Popup::ConsumerEdit(f)) = self.popup.as_mut() {
                    f.submitting = false;
                    f.error = Some(e);
                } else {
                    self.status = format!("Error: {e}");
                }
            }
            Action::QueueMetricsLoaded { queue_id, metrics } => {
                if self.queues.selected_id().as_deref() == Some(queue_id.as_str()) {
                    self.queues.set_metrics(metrics);
                }
            }
            Action::MessagesPulled { queue_id, outcome } => {
                if self.queues.selected_id().as_deref() != Some(queue_id.as_str()) {
                    return;
                }
                match outcome {
                    Ok(messages) => {
                        let name = self.queues.selected_name().unwrap_or_default();
                        self.popup = Some(Popup::PeekView(PeekView::new(name, messages)));
                    }
                    Err(e) => {
                        self.popup = Some(Popup::Message(Message {
                            title: "Peek no disponible".into(),
                            body: format!(
                                "{e}\n\nSolo se pueden espiar mensajes de colas con consumer HTTP pull (sin consumer worker)."
                            ),
                            is_error: true,
                        }));
                    }
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
            Action::D1TablesLoaded {
                db_id,
                tables,
                schema,
            } => {
                self.d1.set_tables(&db_id, tables, schema);
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
                    self.load_objects();
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

            Action::R2ObjectsLoaded {
                bucket,
                prefix,
                list,
            } => {
                if self.r2.selected_name().as_deref() == Some(bucket.as_str()) {
                    self.r2.set_objects(&prefix, list);
                }
            }
            Action::R2ObjectsError(e) => self.r2.set_objects_error(e),
            Action::R2MoreObjectsLoaded {
                bucket,
                prefix,
                list,
            } => {
                if self.r2.selected_name().as_deref() == Some(bucket.as_str()) {
                    self.r2.append_objects(&prefix, list);
                }
            }
            Action::SearchObjects { term } => self.start_deep_search(term),
            Action::SearchProgress {
                bucket,
                generation,
                page,
                hits,
            } => {
                if self.r2.selected_name().as_deref() == Some(bucket.as_str())
                    && self.r2.set_search_progress(generation, page, hits)
                {
                    self.status = format!("Buscando… página {page} · {hits} coincidencias");
                }
            }
            Action::SearchResults {
                bucket,
                generation,
                files,
                pages,
                capped,
                error,
            } => {
                if self.r2.selected_name().as_deref() == Some(bucket.as_str()) {
                    let n = files.len();
                    if self.r2.set_search_results(generation, files, pages, capped) {
                        self.search_cancel = None;
                        self.status = match error {
                            Some(e) => format!("Búsqueda parcial ({pages} pág.): ✗ {e}"),
                            None if capped => {
                                format!("{n} resultados (tope de 10.000 objetos alcanzado)")
                            }
                            None => format!("{n} resultados en {pages} página(s)"),
                        };
                    }
                }
            }
            Action::UploadObject { path } => self.spawn_upload(path),
            Action::DeleteObject { key } => self.spawn_delete_object(key),
            Action::DeleteObjects { keys } => self.spawn_delete_objects(keys),
            Action::CreateFolder { name } => self.spawn_create_folder(name),
            Action::RenameObject {
                old_key,
                new_key,
                content_type,
            } => self.spawn_rename_object(old_key, new_key, content_type),
            Action::ObjectMutated(msg) => {
                self.status = msg;
                if matches!(self.popup, Some(Popup::Upload(_)) | Some(Popup::Rename(_))) {
                    self.popup = None;
                }
                // Tras mutar en modo búsqueda se vuelve al browse (los
                // resultados quedarían desactualizados y repetir la búsqueda
                // costaría hasta 20 páginas).
                if self.r2.is_searching() {
                    self.exit_search();
                } else {
                    self.load_objects();
                }
                // El uso del bucket cambió: refresca la info.
                if let Some(name) = self.r2.selected_name() {
                    self.load_bucket_info(name);
                }
            }
            Action::ObjectStatus(msg) => {
                self.status = msg;
                // Si venía de un fallo al paginar, suelta el flag de carga
                // sin tocar el listado (inocuo en el resto de casos).
                self.r2.end_loading();
            }
            Action::ObjectError(e) => {
                if let Some(Popup::Upload(u)) = self.popup.as_mut() {
                    u.submitting = false;
                    u.error = Some(e);
                } else if let Some(Popup::Rename(r)) = self.popup.as_mut() {
                    r.submitting = false;
                    r.error = Some(e);
                } else {
                    self.status = format!("Error: {e}");
                }
            }
            Action::SaveR2Creds { access_key, secret } => {
                match secrets::save_r2_credentials(&access_key, &secret) {
                    Ok(()) => {
                        self.status = "Credenciales R2 guardadas".into();
                        // Continúa el flujo de presign si venía de ahí.
                        if let Some(key) = self.pending_presign.take() {
                            self.popup = Some(Popup::Presign(PresignForm {
                                key,
                                expires: TextInput::new("3600"),
                                error: None,
                            }));
                        }
                    }
                    Err(e) => self.status = format!("Keyring: {e}"),
                }
            }
            Action::GeneratePresign { key, expires } => self.generate_presign(key, expires),
            Action::ImageDecoded { key, result } => {
                match result {
                    Ok((w, h, rgb)) => {
                        let lines = crate::components::r2::image_lines(w, h, &rgb);
                        let title = key.rsplit('/').next().unwrap_or(&key).to_string();
                        self.popup = Some(Popup::ImageView(ImageView { title, lines }));
                        self.status.clear();
                    }
                    Err(e) => self.status = format!("Preview: {e}"),
                }
            }
            Action::SaveCors { bucket, rules } => self.spawn_save_cors(bucket, rules),
            Action::CorsMutated(msg) => {
                self.status = msg;
                if matches!(self.popup, Some(Popup::CorsEdit(_))) {
                    self.popup = None;
                }
                if let Some(name) = self.r2.selected_name() {
                    self.load_bucket_info(name);
                }
            }
            Action::CorsError(e) => {
                if let Some(Popup::CorsEdit(c)) = self.popup.as_mut() {
                    c.submitting = false;
                    c.error = Some(e);
                } else {
                    self.status = format!("Error: {e}");
                }
            }
            Action::SetPublicDomain { bucket, enabled } => {
                self.spawn_set_public_domain(bucket, enabled);
            }
            Action::AddCustomDomain {
                bucket,
                domain,
                zone_id,
            } => self.spawn_add_domain(bucket, domain, zone_id),
            Action::RemoveCustomDomain { bucket, domain } => {
                self.spawn_remove_domain(bucket, domain);
            }
            Action::DomainsMutated(msg) => {
                self.status = msg;
                // El popup de dominios es un snapshot: se cierra y se reabre
                // con `t` cuando la info recargue.
                if matches!(
                    self.popup,
                    Some(Popup::DomainAdd(_)) | Some(Popup::BucketDomains(_))
                ) {
                    self.popup = None;
                }
                if let Some(name) = self.r2.selected_name() {
                    self.load_bucket_info(name);
                }
            }
            Action::DomainError(e) => {
                if let Some(Popup::DomainAdd(f)) = self.popup.as_mut() {
                    f.submitting = false;
                    f.error = Some(e);
                } else {
                    self.status = format!("Error: {e}");
                }
            }
        }
    }

    // --- Cuentas / zonas ---

    fn active_account_id(&self) -> Option<&str> {
        self.sessions
            .get(self.active_session)?
            .accounts
            .get(self.active_account)
            .map(|a| a.id.as_str())
    }

    fn active_account_name(&self) -> &str {
        self.sessions
            .get(self.active_session)
            .and_then(|s| s.accounts.get(self.active_account))
            .map(|a| a.name.as_str())
            .unwrap_or("")
    }

    /// Abre el selector con todas las cuentas de todos los tokens.
    fn open_account_picker(&mut self) {
        let mut rows = Vec::new();
        for (si, s) in self.sessions.iter().enumerate() {
            let suffix = mask_token(&s.token);
            if s.accounts.is_empty() {
                // Token sin cuentas visibles: fila para poder eliminarlo.
                rows.push(AccountRow {
                    label: format!("(sin cuentas) · {suffix}"),
                    session: si,
                    account: usize::MAX,
                    active: false,
                });
            }
            for (ai, a) in s.accounts.iter().enumerate() {
                rows.push(AccountRow {
                    label: format!("{} · {suffix}", a.name),
                    session: si,
                    account: ai,
                    active: si == self.active_session && ai == self.active_account,
                });
            }
        }
        self.popup = Some(Popup::AccountPicker(AccountPicker::new(rows)));
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

    // --- Confirmaciones ---

    /// Zonas de la cuenta activa como `ZoneRef` (nombre + id) para el select.
    fn account_zone_refs(&self) -> Vec<ZoneRef> {
        let acc = self.active_account_id();
        self.all_zones
            .iter()
            .filter(|z| acc.is_none_or(|a| z.account_id().is_none_or(|zid| zid == a)))
            .map(|z| ZoneRef {
                name: z.name.clone(),
                id: z.id.clone(),
            })
            .collect()
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

    // --- Túneles ---

    // --- Workers ---

    // --- Queues (Fase 4) ---

    // --- D1 ---

    // --- R2 ---

    // --- Foco ---

    fn panes(&self) -> &'static [Focus] {
        static DNS_PANES: &[Focus] = &[Focus::Modules, Focus::Zones, Focus::Records];
        static TUNNEL_PANES: &[Focus] =
            &[Focus::Modules, Focus::Tunnels, Focus::TunnelRoutes];
        static WORKER_PANES: &[Focus] =
            &[Focus::Modules, Focus::Workers, Focus::WorkersDetail];
        static QUEUE_PANES: &[Focus] =
            &[Focus::Modules, Focus::Queues, Focus::QueuesDetail];
        static D1_PANES: &[Focus] = &[
            Focus::Modules,
            Focus::D1Dbs,
            Focus::D1Tables,
            Focus::D1Editor,
            Focus::D1Where,
            Focus::D1Results,
        ];
        // Orden visual: la barra de filtro va encima del listado (como la
        // barra WHERE de D1 sobre los resultados).
        static R2_PANES: &[Focus] = &[
            Focus::Modules,
            Focus::R2Buckets,
            Focus::R2Filter,
            Focus::R2Objects,
        ];
        match self.sidebar.module() {
            Module::Dns => DNS_PANES,
            Module::Tunnels => TUNNEL_PANES,
            Module::Workers => WORKER_PANES,
            Module::Queues => QUEUE_PANES,
            Module::D1 => D1_PANES,
            Module::R2 => R2_PANES,
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
                    ("Tab", "ir a las rutas del túnel"),
                    ("n", "nuevo túnel"),
                    ("a", "añadir ruta pública (+ DNS)"),
                    ("c", "limpiar conexiones (con confirmación)"),
                    ("d", "borrar túnel (con confirmación)"),
                    ("r", "recargar túneles"),
                ],
            ),
            Focus::TunnelRoutes => HelpSection::new(
                "Rutas del túnel",
                vec![
                    ("↑ ↓ / k j", "navegar rutas"),
                    ("a", "añadir ruta pública (+ DNS)"),
                    ("e", "editar ruta (servicio/ruta)"),
                    ("d", "borrar ruta (con confirmación)"),
                    ("r", "recargar rutas"),
                ],
            ),
            Focus::Workers => HelpSection::new(
                "Workers (lista)",
                vec![
                    ("↑ ↓ / k j", "cambiar de worker"),
                    ("Tab", "ir al detalle (columna 3)"),
                    ("1-5 / ←→", "pestaña (métricas/impl./vars/logs/rutas)"),
                    ("l", "live-tail de logs on/off"),
                    ("t", "probar una ruta (GET)"),
                    ("r", "recargar workers"),
                ],
            ),
            Focus::WorkersDetail => HelpSection::new(
                "Workers (detalle)",
                vec![
                    ("1-5 / ←→", "pestaña (métricas/impl./vars/logs/rutas)"),
                    ("↑ ↓ / k j", "navegar (impl. · vars · logs según pestaña)"),
                    ("Enter", "revertir despliegue (impl.) · ver detalle (logs)"),
                    ("e / a", "editar / añadir secreto (pestaña Vars)"),
                    ("l", "live-tail de logs on/off (pestaña Logs)"),
                    ("/ · E · y", "filtrar · solo errores · copiar evento (Logs)"),
                    ("End", "seguir el final del tail (Logs)"),
                    ("t", "probar una ruta (GET)"),
                    ("r", "recargar workers"),
                ],
            ),
            Focus::Queues => HelpSection::new(
                "Queues (lista)",
                vec![
                    ("↑ ↓ / k j", "cambiar de cola"),
                    ("Tab", "ir al detalle (columna 3)"),
                    ("1-3 / ←→", "pestaña (resumen/consumers/métricas)"),
                    ("n / d", "nueva cola / borrar (con confirmación)"),
                    ("s", "enviar mensaje"),
                    ("p / P", "pausar-reanudar entrega / purgar mensajes"),
                    ("m", "espiar mensajes (peek, solo http_pull)"),
                    ("l", "logs en vivo del consumer (salta a Workers)"),
                    ("r", "recargar colas"),
                ],
            ),
            Focus::QueuesDetail => HelpSection::new(
                "Queues (detalle)",
                vec![
                    ("1-3 / ←→", "pestaña (resumen/consumers/métricas)"),
                    ("↑ ↓ / k j", "navegar consumers (pestaña Consumers)"),
                    ("e / Enter", "editar consumer (batch/retries/DLQ)"),
                    ("s", "enviar mensaje"),
                    ("p / P", "pausar-reanudar entrega / purgar mensajes"),
                    ("m", "espiar mensajes (peek, solo http_pull)"),
                    ("l", "logs en vivo del consumer (salta a Workers)"),
                    ("r", "recargar colas"),
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
                    ("(texto)", "escribe SQL · sugiere al teclear"),
                    ("Ctrl+Espacio", "abrir sugerencias"),
                    ("Tab / Enter", "aceptar sugerencia (popup abierto)"),
                    ("↑ ↓ / Esc", "navegar / cerrar sugerencias"),
                ],
            ),
            Focus::D1Where => HelpSection::new(
                "Filtro WHERE",
                vec![
                    ("(texto)", "escribe una cláusula WHERE · sugiere al teclear"),
                    ("Ctrl+Espacio", "abrir sugerencias (columnas del resultado)"),
                    ("Tab / Enter", "aceptar sugerencia (popup abierto)"),
                    ("Enter", "aplicar el filtro (tabla o consulta)"),
                    ("Esc", "cerrar sugerencias / ir a los resultados"),
                ],
            ),
            Focus::WorkersLogFilter => HelpSection::new(
                "Filtro de logs",
                vec![
                    ("(texto)", "filtrar por método/URL/mensaje"),
                    ("Enter", "fijar el filtro"),
                    ("Esc", "limpiar el filtro"),
                ],
            ),
            Focus::D1Results => HelpSection::new(
                "Resultados",
                vec![
                    ("↑ ↓ ← → / k j h l", "navegar celda a celda"),
                    ("PageUp/PageDown", "desplazar filas de 10 en 10"),
                    ("Enter", "ver el valor completo de la celda"),
                    ("y", "copiar celda al portapapeles"),
                    ("Y", "copiar fila completa (TSV)"),
                ],
            ),
            Focus::R2Buckets => HelpSection::new(
                "Buckets R2",
                vec![
                    ("↑ ↓ / k j", "navegar buckets (uso/dominios)"),
                    ("n", "nuevo bucket"),
                    ("d", "borrar bucket (con confirmación)"),
                    ("c", "editar política CORS (JSON)"),
                    ("p", "dominio público r2.dev on/off"),
                    ("t", "dominios personalizados (añadir/quitar)"),
                    ("r", "recargar buckets"),
                ],
            ),
            Focus::R2Objects => HelpSection::new(
                "Objetos R2",
                vec![
                    ("↑ ↓ / k j", "navegar (↓ al final carga más si hay 500+)"),
                    ("Enter", "abrir carpeta / ver imagen"),
                    ("Backspace / h", "subir un nivel (o salir de la búsqueda)"),
                    ("/", "filtrar la carpeta actual"),
                    ("s", "buscar en todo el bucket"),
                    ("Espacio", "marcar/desmarcar (x borra los marcados)"),
                    ("u", "subir un archivo local"),
                    ("n", "nueva carpeta"),
                    ("d", "descargar a ~/Descargas y abrir"),
                    ("o", "abrir en el navegador (dominio público/personalizado)"),
                    ("y", "copiar URL del objeto"),
                    ("i", "metadatos del objeto"),
                    ("e", "renombrar objeto"),
                    ("m", "mover objeto (editar la clave completa)"),
                    ("p", "URL prefirmada (pide credenciales R2 una vez)"),
                    ("v", "previsualizar imagen en el terminal"),
                    ("x", "borrar objeto/marcados (con confirmación)"),
                    ("r", "recargar listado (o repetir la búsqueda)"),
                ],
            ),
            Focus::R2Filter => HelpSection::new(
                "Filtro de objetos",
                vec![
                    ("(texto)", "filtra por nombre mientras escribes"),
                    ("Enter", "fijar el filtro y volver al listado"),
                    ("Tab / ⇧Tab", "cambiar de panel (el filtro se mantiene)"),
                    ("Esc", "limpiar el filtro"),
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
        self.rect_tunnel_routes = None;
        self.rect_workers = None;
        self.rect_workers_detail = None;
        self.rect_queues = None;
        self.rect_queues_detail = None;
        self.rect_d1_dbs = None;
        self.rect_d1_tables = None;
        self.rect_d1_editor = None;
        self.rect_d1_where = None;
        self.rect_d1_results = None;
        self.rect_r2 = None;
        self.rect_r2_objects = None;
        self.rect_r2_filter = None;

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
                let (list_area, detail_area, routes_area) =
                    crate::components::tunnels::split(shell.main);
                self.tunnels
                    .draw_list(frame, list_area, self.focus == Focus::Tunnels);
                self.tunnels.draw_detail(frame, detail_area, false);
                self.tunnels
                    .draw_routes(frame, routes_area, self.focus == Focus::TunnelRoutes);
                self.rect_tunnels = Some(list_area);
                self.rect_tunnel_routes = Some(routes_area);
            }
            Module::Workers if main_active => {
                let (list_area, detail_area) =
                    crate::components::workers::split(shell.main);
                self.workers
                    .draw_list(frame, list_area, self.focus == Focus::Workers);
                let detail_focused =
                    matches!(self.focus, Focus::WorkersDetail | Focus::WorkersLogFilter);
                self.workers.draw_detail(
                    frame,
                    detail_area,
                    detail_focused,
                    self.focus == Focus::WorkersLogFilter,
                );
                self.rect_workers = Some(list_area);
                self.rect_workers_detail = Some(detail_area);
            }
            Module::Queues if main_active => {
                let (list_area, detail_area) = crate::components::workers::split(shell.main);
                self.queues
                    .draw_list(frame, list_area, self.focus == Focus::Queues);
                self.queues
                    .draw_detail(frame, detail_area, self.focus == Focus::QueuesDetail);
                self.rect_queues = Some(list_area);
                self.rect_queues_detail = Some(detail_area);
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
                // El área de resultados se parte en barra WHERE (3) + rejilla.
                let where_result = ratatui::layout::Layout::default()
                    .direction(ratatui::layout::Direction::Vertical)
                    .constraints([
                        ratatui::layout::Constraint::Length(3),
                        ratatui::layout::Constraint::Min(1),
                    ])
                    .split(result_area);
                let (where_area, grid_area) = (where_result[0], where_result[1]);
                self.d1
                    .draw_where(frame, where_area, self.focus == Focus::D1Where);
                self.d1
                    .draw_result(frame, grid_area, self.focus == Focus::D1Results);
                self.rect_d1_dbs = Some(dbs_area);
                self.rect_d1_tables = Some(tables_area);
                self.rect_d1_editor = Some(editor_area);
                self.rect_d1_where = Some(where_area);
                self.rect_d1_results = Some(grid_area);
            }
            Module::R2 if main_active => {
                let (buckets_area, info_area, objects_area) =
                    crate::components::r2::split(shell.main);
                self.r2
                    .draw_buckets(frame, buckets_area, self.focus == Focus::R2Buckets);
                self.r2.draw_info(frame, info_area);
                // Barra de filtro siempre visible sobre el listado.
                let rows = ratatui::layout::Layout::default()
                    .direction(ratatui::layout::Direction::Vertical)
                    .constraints([
                        ratatui::layout::Constraint::Length(3),
                        ratatui::layout::Constraint::Min(1),
                    ])
                    .split(objects_area);
                let (filter_area, obj_area) = (rows[0], rows[1]);
                self.r2
                    .draw_objects(frame, obj_area, self.focus == Focus::R2Objects);
                self.r2
                    .draw_filter(frame, filter_area, self.focus == Focus::R2Filter);
                self.rect_r2 = Some(buckets_area);
                self.rect_r2_objects = Some(obj_area);
                self.rect_r2_filter = Some(filter_area);
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
        // Con varios tokens, añade el sufijo del token para distinguir cuentas.
        let acc = if self.sessions.len() > 1 {
            let suffix = self
                .sessions
                .get(self.active_session)
                .map(|s| mask_token(&s.token))
                .unwrap_or_default();
            format!("{} {suffix}", self.active_account_name())
        } else {
            self.active_account_name().to_string()
        };
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
                Focus::Tunnels => {
                    "↑↓ túnel · Tab → rutas · n nuevo · a ruta · c limpiar · d borrar · ?".into()
                }
                Focus::TunnelRoutes => {
                    "↑↓ ruta · a añadir · e editar · d borrar · Tab → · ? ayuda".into()
                }
                Focus::Workers => {
                    "↑↓ worker · Tab → detalle · 1-5 pestaña · l logs · t probar · ?".into()
                }
                Focus::WorkersDetail => match self.workers.active_tab {
                    1 => "↑↓ deploy · Enter revertir · 1-5 pestaña · l logs · t · r · ?".into(),
                    3 => "↑↓ log · Enter detalle · / filtrar · E errores · y copiar · End sigue".into(),
                    _ => "↑↓ contenido · 1-5 pestaña · e editar · a secreto · l · t · ?".into(),
                },
                Focus::Queues => {
                    "↑↓ cola · 1-3 pestaña · s enviar · p pausa · P purgar · m peek · l logs · ?"
                        .into()
                }
                Focus::QueuesDetail => match self.queues.active_tab {
                    1 => "↑↓ consumer · e/Enter editar · s p P m l · r · ?".into(),
                    _ => "1-3 pestaña · s enviar · p pausa · P purgar · m peek · l logs · ?".into(),
                },
                Focus::D1Dbs => "↑↓ base · Tab → editor · r recargar · A · ? ayuda".into(),
                Focus::D1Tables => "↑↓ tabla · Enter SELECT * · Tab → editor · r · ?".into(),
                Focus::D1Editor => {
                    "escribe SQL · Ctrl+Espacio sugerir · F5 ejecutar · Tab → · ?".into()
                }
                Focus::D1Where => {
                    "escribe filtro WHERE · Ctrl+Espacio sugerir · Enter aplicar · ?".into()
                }
                Focus::D1Results => {
                    "↑↓←→ celda · Enter ver · y copiar · Y fila · PgUp/Dn · Tab →".into()
                }
                Focus::R2Buckets => {
                    "↑↓ bucket · n nuevo · c CORS · p público · t dominios · d borrar · ?".into()
                }
                Focus::R2Objects => {
                    if self.r2.is_searching() {
                        "Enter ir a carpeta · Esc volver · d y i x sobre el resultado · ?".into()
                    } else if self.r2.marks_len() > 0 {
                        format!(
                            "{} marcado(s) · Espacio marcar · x borrar marcados · ? ayuda",
                            self.r2.marks_len()
                        )
                    } else {
                        "Enter abrir · / filtrar · s buscar · Espacio marcar · u d e m i y x · ?"
                            .into()
                    }
                }
                Focus::R2Filter => {
                    "escribe para filtrar · Enter fijar · Esc limpiar · Tab panel".into()
                }
                Focus::WorkersLogFilter => {
                    "escribe para filtrar logs · Enter fijar · Esc limpiar · Tab panel".into()
                }
            }
        };
        (left, right)
    }
}

/// Entrecomilla un identificador SQL (tabla) escapando comillas dobles.
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Sufijo legible de un token para distinguirlo sin exponerlo (`····abcd`).
fn mask_token(token: &str) -> String {
    let tail: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("····{tail}")
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
