//! Overlays modales: entrada de token, confirmaciones, selector de cuenta y
//! mensajes. El estado lo posee `app.rs`; aquí van los datos y el render.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::action::Action;
use crate::components::input::TextInput;
use crate::model::{Binding, CustomDomain, DnsRecord};
use crate::ui::{layout, theme};

/// Popup activo.
pub enum Popup {
    /// Entrada del API token (pantalla de auth).
    Token(TokenEntry),
    /// Confirmación de una acción destructiva.
    Confirm(Confirm),
    /// Selector de cuenta activa.
    AccountPicker(AccountPicker),
    /// Ayuda: atajos del contexto actual (los construye `app.rs`).
    Help(Help),
    /// Entrada del nombre para crear un túnel.
    NewTunnel(NewTunnel),
    /// Entrada del nombre para crear un bucket R2.
    NewBucket(NewBucket),
    /// Subir un archivo a R2.
    Upload(UploadForm),
    /// Renombrar un objeto R2.
    Rename(RenameForm),
    /// Pedir credenciales R2 (para URLs prefirmadas).
    R2Creds(R2CredsForm),
    /// Pedir expiración de la URL prefirmada.
    Presign(PresignForm),
    /// Previsualización de imagen.
    ImageView(ImageView),
    /// Formulario de crear/editar registro DNS.
    RecordForm(RecordForm),
    /// Formulario para añadir una ruta pública (ingress) a un túnel.
    RouteForm(RouteForm),
    /// Prueba HTTP de una ruta de Worker.
    HttpTest(HttpTest),
    /// Editar/añadir una variable o secreto de un Worker.
    BindingEdit(BindingEdit),
    /// Editor de la política CORS de un bucket R2 (JSON crudo).
    CorsEdit(CorsEditForm),
    /// Elegir con qué dominio abrir/copiar la URL de un objeto R2.
    ChooseDomain(ChooseDomain),
    /// Término para la búsqueda profunda en todo el bucket.
    SearchInput(SearchInput),
    /// Nombre de la carpeta nueva (objeto marcador vacío).
    NewFolder(NewFolder),
    /// Dominios personalizados del bucket: lista + añadir/quitar.
    BucketDomains(BucketDomains),
    /// Conectar un dominio personalizado (subdominio + zona de la cuenta).
    DomainAdd(DomainAddForm),
    /// Detalle scrollable de un evento del live-tail de Workers.
    LogDetail(LogDetail),
    /// Mensaje informativo o de error.
    Message(Message),
}

/// Detalle de un evento de tail (request/cf/logs/excepciones), scrollable.
pub struct LogDetail {
    pub title: String,
    pub lines: Vec<String>,
    /// JSON crudo del evento (para copiar con `y`).
    pub raw: String,
    pub scroll: u16,
}

/// Editor de variable (plain_text) o secreto (secret_text) de un Worker.
/// Al editar solo cambia el valor; al añadir (`adding`) se escribe nombre + valor
/// y el tipo es secreto (endpoint aislado y seguro).
pub struct BindingEdit {
    pub script: String,
    pub name: TextInput,
    pub is_secret: bool,
    pub value: TextInput,
    pub adding: bool,
    /// 0 = nombre (solo al añadir), luego valor.
    pub field: usize,
    pub error: Option<String>,
    pub submitting: bool,
}

impl BindingEdit {
    pub fn edit(script: String, b: &Binding) -> Self {
        let is_secret = b.is_secret();
        Self {
            script,
            name: TextInput::new(b.name.clone()),
            is_secret,
            // Los secretos no se pueden leer: se parte de vacío.
            value: if is_secret {
                TextInput::default()
            } else {
                TextInput::new(b.text.clone().unwrap_or_default())
            },
            adding: false,
            field: 0,
            error: None,
            submitting: false,
        }
    }

    pub fn add_secret(script: String) -> Self {
        Self {
            script,
            name: TextInput::default(),
            is_secret: true,
            value: TextInput::default(),
            adding: true,
            field: 0,
            error: None,
            submitting: false,
        }
    }

    fn field_count(&self) -> usize {
        if self.adding { 2 } else { 1 }
    }

    pub fn move_field(&mut self, delta: i32) {
        let n = self.field_count() as i32;
        self.field = ((((self.field as i32 + delta) % n) + n) % n) as usize;
    }

    /// `true` si el campo activo es el nombre (solo al añadir).
    pub fn on_name(&self) -> bool {
        self.adding && self.field == 0
    }

    pub fn active_text_mut(&mut self) -> &mut TextInput {
        if self.on_name() {
            &mut self.name
        } else {
            &mut self.value
        }
    }
}

/// Entrada de una sola línea (nombre del túnel).
#[derive(Default)]
pub struct NewTunnel {
    pub name: TextInput,
    pub error: Option<String>,
}

/// Entrada de una sola línea (nombre del bucket R2).
#[derive(Default)]
pub struct NewBucket {
    pub name: TextInput,
    pub error: Option<String>,
}

/// Renombrar un objeto R2 (dentro de la misma carpeta): solo cambia el
/// nombre de archivo, el prefijo/carpeta se mantiene.
pub struct RenameForm {
    pub old_key: String,
    pub name: TextInput,
    /// `true` = mover: el input es la clave completa (permite cambiar de carpeta).
    pub move_mode: bool,
    pub error: Option<String>,
    pub submitting: bool,
}

/// Término de búsqueda profunda (subcadena sobre las claves de todo el bucket).
#[derive(Default)]
pub struct SearchInput {
    pub term: TextInput,
    pub error: Option<String>,
}

/// Nombre de la carpeta nueva (crea el objeto marcador `prefijo/nombre/`).
#[derive(Default)]
pub struct NewFolder {
    pub name: TextInput,
    pub error: Option<String>,
}

/// Dominios personalizados de un bucket (snapshot de `BucketInfo.domains`).
pub struct BucketDomains {
    pub bucket: String,
    pub rows: Vec<CustomDomain>,
    pub state: ListState,
}

impl BucketDomains {
    pub fn new(bucket: String, rows: Vec<CustomDomain>) -> Self {
        let mut state = ListState::default();
        state.select((!rows.is_empty()).then_some(0));
        Self { bucket, rows, state }
    }

    pub fn move_by(&mut self, delta: i32) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let n = len as i32;
        self.state.select(Some(((((cur + delta) % n) + n) % n) as usize));
    }

    pub fn selected(&self) -> Option<&CustomDomain> {
        self.state.selected().and_then(|i| self.rows.get(i))
    }
}

/// Conectar un dominio personalizado al bucket: subdominio + zona de la cuenta.
pub struct DomainAddForm {
    pub bucket: String,
    pub subdomain: TextInput,
    pub zones: Vec<ZoneRef>,
    pub zone_idx: usize,
    /// 0 = subdominio, 1 = zona (select con ←→).
    pub field: usize,
    pub error: Option<String>,
    pub submitting: bool,
}

impl DomainAddForm {
    pub fn new(bucket: String, zones: Vec<ZoneRef>) -> Self {
        Self {
            bucket,
            subdomain: TextInput::default(),
            zones,
            zone_idx: 0,
            field: 0,
            error: None,
            submitting: false,
        }
    }

    pub fn move_field(&mut self, delta: i32) {
        self.field = ((((self.field as i32 + delta) % 2) + 2) % 2) as usize;
    }

    /// Rellena las zonas si aún estaban vacías (llegaron tras abrir el form).
    pub fn set_zones(&mut self, zones: Vec<ZoneRef>) {
        if self.zones.is_empty() {
            self.zones = zones;
            self.zone_idx = 0;
        }
    }

    pub fn cycle_zone(&mut self, delta: i32) {
        let n = self.zones.len() as i32;
        if n == 0 {
            return;
        }
        self.zone_idx = ((((self.zone_idx as i32 + delta) % n) + n) % n) as usize;
    }

    pub fn zone(&self) -> Option<&ZoneRef> {
        self.zones.get(self.zone_idx)
    }

    /// Dominio completo: `sub.zona`, o el apex si el subdominio está vacío.
    pub fn full_domain(&self) -> Option<String> {
        let zone = &self.zone()?.name;
        let sub = self.subdomain.value().trim();
        Some(if sub.is_empty() {
            zone.clone()
        } else {
            format!("{sub}.{zone}")
        })
    }
}

/// Subir un archivo local al prefijo actual del bucket.
#[derive(Default)]
pub struct UploadForm {
    /// Destino visible (bucket/prefijo) — informativo.
    pub dest: String,
    pub path: TextInput,
    pub error: Option<String>,
    pub submitting: bool,
}

/// Credenciales R2 (Access Key + Secret) para URLs prefirmadas S3.
#[derive(Default)]
pub struct R2CredsForm {
    pub access_key: TextInput,
    pub secret: TextInput,
    /// 0 = access key, 1 = secret.
    pub field: usize,
    pub error: Option<String>,
}

/// Expiración (segundos) para la URL prefirmada del objeto `key`.
pub struct PresignForm {
    pub key: String,
    pub expires: TextInput,
    pub error: Option<String>,
}

/// Visor de imagen en terminal (medias celdas ▀ RGB).
pub struct ImageView {
    pub title: String,
    pub lines: Vec<Line<'static>>,
}

/// Editor de la política CORS de un bucket (JSON crudo, multilínea).
pub struct CorsEditForm {
    pub bucket: String,
    pub json: TextInput,
    pub error: Option<String>,
    pub submitting: bool,
}

/// Una URL candidata para abrir un objeto (dominio público o personalizado).
pub struct DomainChoice {
    pub label: String,
    pub domain: String,
}

/// Qué hacer con la URL del objeto al elegir dominio.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChoosePurpose {
    /// Abrir en el navegador.
    Abrir,
    /// Copiar al portapapeles (OSC 52).
    Copiar,
}

/// Selección de dominio para abrir/copiar la URL de un objeto R2.
pub struct ChooseDomain {
    /// Clave del objeto (para construir la URL al confirmar).
    pub key: String,
    pub purpose: ChoosePurpose,
    pub choices: Vec<DomainChoice>,
    pub state: ListState,
}

impl ChooseDomain {
    pub fn new(key: String, choices: Vec<DomainChoice>, purpose: ChoosePurpose) -> Self {
        let mut state = ListState::default();
        state.select((!choices.is_empty()).then_some(0));
        Self {
            key,
            purpose,
            choices,
            state,
        }
    }

    pub fn move_by(&mut self, delta: i32) {
        let len = self.choices.len();
        if len == 0 {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let n = len as i32;
        self.state.select(Some(((((cur + delta) % n) + n) % n) as usize));
    }

    pub fn selected(&self) -> Option<&DomainChoice> {
        self.state.selected().and_then(|i| self.choices.get(i))
    }
}

/// Prueba HTTP: URL a golpear.
#[derive(Default)]
pub struct HttpTest {
    pub url: TextInput,
    pub error: Option<String>,
    pub sending: bool,
}

/// Campos posibles del formulario DNS (los visibles dependen del tipo).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RField {
    Type,
    Name,
    Content,
    Priority,
    Proxy,
    Ttl,
}

/// Formulario de registro DNS (crear si `editing_id` es `None`).
/// El campo "Tipo" es un select; los demás campos cambian según el tipo.
pub struct RecordForm {
    pub zone_id: String,
    pub editing_id: Option<String>,
    pub types: Vec<String>,
    pub type_idx: usize,
    pub name: TextInput,
    pub content: TextInput,
    pub priority: TextInput,
    pub proxied: bool,
    pub ttl: TextInput,
    /// Índice dentro de `visible()`.
    pub field: usize,
    pub error: Option<String>,
    /// Petición en vuelo: bloquea input y muestra "Guardando…".
    pub submitting: bool,
}

/// Tipos ofrecidos en el select.
const DEFAULT_TYPES: [&str; 5] = ["A", "AAAA", "CNAME", "TXT", "MX"];

impl RecordForm {
    pub fn create(zone_id: String) -> Self {
        Self {
            zone_id,
            editing_id: None,
            types: DEFAULT_TYPES.iter().map(|s| s.to_string()).collect(),
            type_idx: 0,
            name: TextInput::default(),
            content: TextInput::default(),
            priority: TextInput::new("10"),
            proxied: false,
            ttl: TextInput::new("1"),
            field: 0,
            error: None,
            submitting: false,
        }
    }

    pub fn edit(zone_id: String, r: &DnsRecord) -> Self {
        let mut types: Vec<String> = DEFAULT_TYPES.iter().map(|s| s.to_string()).collect();
        // Preserva el tipo aunque no esté en la lista por defecto (p. ej. NS).
        if !types.iter().any(|t| t == &r.record_type) {
            types.insert(0, r.record_type.clone());
        }
        let type_idx = types.iter().position(|t| t == &r.record_type).unwrap_or(0);
        Self {
            zone_id,
            editing_id: Some(r.id.clone()),
            types,
            type_idx,
            name: TextInput::new(r.name.clone()),
            content: TextInput::new(r.content.clone()),
            priority: TextInput::new(r.priority.map(|p| p.to_string()).unwrap_or_else(|| "10".into())),
            proxied: r.proxied == Some(true),
            ttl: TextInput::new(r.ttl.to_string()),
            field: 0,
            error: None,
            submitting: false,
        }
    }

    pub fn rtype(&self) -> &str {
        &self.types[self.type_idx]
    }

    pub fn proxiable(&self) -> bool {
        matches!(self.rtype(), "A" | "AAAA" | "CNAME")
    }

    pub fn is_mx(&self) -> bool {
        self.rtype() == "MX"
    }

    /// Etiqueta del campo de contenido según el tipo.
    pub fn content_label(&self) -> &'static str {
        match self.rtype() {
            "A" => "Dirección IPv4",
            "AAAA" => "Dirección IPv6",
            "CNAME" => "Destino",
            "MX" => "Servidor",
            _ => "Contenido",
        }
    }

    /// Campos visibles, en orden, para el tipo actual.
    pub fn visible(&self) -> Vec<RField> {
        let mut v = vec![RField::Type, RField::Name, RField::Content];
        if self.is_mx() {
            v.push(RField::Priority);
        }
        if self.proxiable() {
            v.push(RField::Proxy);
        }
        v.push(RField::Ttl);
        v
    }

    pub fn current(&self) -> RField {
        let vis = self.visible();
        vis[self.field.min(vis.len() - 1)]
    }

    pub fn move_field(&mut self, delta: i32) {
        let n = self.visible().len() as i32;
        self.field = ((((self.field as i32 + delta) % n) + n) % n) as usize;
    }

    /// Cambia el tipo (select); ajusta proxy y re-encaja el campo activo.
    pub fn cycle_type(&mut self, delta: i32) {
        let n = self.types.len() as i32;
        self.type_idx = ((((self.type_idx as i32 + delta) % n) + n) % n) as usize;
        if !self.proxiable() {
            self.proxied = false;
        }
        let len = self.visible().len();
        if self.field >= len {
            self.field = len - 1;
        }
    }

    /// Campo de texto activo (None para Type/Proxy).
    pub fn active_text_mut(&mut self) -> Option<&mut TextInput> {
        match self.current() {
            RField::Name => Some(&mut self.name),
            RField::Content => Some(&mut self.content),
            RField::Priority => Some(&mut self.priority),
            RField::Ttl => Some(&mut self.ttl),
            _ => None,
        }
    }
}

/// Campos del formulario de ruta pública, en orden.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RouteField {
    Subdomain,
    Domain,
    Path,
    Service,
}

/// Zona candidata para el dominio de la ruta (nombre visible + id para el CNAME).
#[derive(Clone)]
pub struct ZoneRef {
    pub name: String,
    pub id: String,
}

/// Formulario para rutas públicas (ingress) de un túnel.
/// - Crear (estilo dashboard "Agregar aplicación publicada"): subdominio +
///   dominio de las zonas de la cuenta → el CNAME se crea automáticamente.
/// - Editar (`editing = Some(hostname)`): el hostname es fijo (solo servicio y
///   ruta), así el CNAME sigue siendo válido.
pub struct RouteForm {
    pub tunnel_id: String,
    pub tunnel_name: String,
    /// `Some(hostname)` = editando una ruta existente; `None` = creando.
    pub editing: Option<String>,
    pub subdomain: TextInput,
    pub zones: Vec<ZoneRef>,
    pub zone_idx: usize,
    pub path: TextInput,
    pub service: TextInput,
    /// Índice dentro de `visible()`.
    pub field: usize,
    pub error: Option<String>,
    pub submitting: bool,
}

impl RouteForm {
    pub fn new(tunnel_id: String, tunnel_name: String, zones: Vec<ZoneRef>) -> Self {
        Self {
            tunnel_id,
            tunnel_name,
            editing: None,
            subdomain: TextInput::default(),
            zones,
            zone_idx: 0,
            path: TextInput::default(),
            service: TextInput::new("https://localhost:8080"),
            field: 0,
            error: None,
            submitting: false,
        }
    }

    /// Editar una ruta existente: hostname fijo, se editan servicio y ruta.
    pub fn edit(
        tunnel_id: String,
        tunnel_name: String,
        hostname: String,
        service: String,
        path: String,
    ) -> Self {
        Self {
            tunnel_id,
            tunnel_name,
            editing: Some(hostname),
            subdomain: TextInput::default(),
            zones: Vec::new(),
            zone_idx: 0,
            path: TextInput::new(path),
            service: TextInput::new(service),
            field: 0,
            error: None,
            submitting: false,
        }
    }

    /// Campos visibles según el modo (al editar, el hostname no se toca).
    pub fn visible(&self) -> &'static [RouteField] {
        if self.editing.is_some() {
            &[RouteField::Path, RouteField::Service]
        } else {
            &[
                RouteField::Subdomain,
                RouteField::Domain,
                RouteField::Path,
                RouteField::Service,
            ]
        }
    }

    pub fn current(&self) -> RouteField {
        let vis = self.visible();
        vis[self.field.min(vis.len() - 1)]
    }

    pub fn move_field(&mut self, delta: i32) {
        let n = self.visible().len() as i32;
        self.field = ((((self.field as i32 + delta) % n) + n) % n) as usize;
    }

    /// Rellena las zonas si aún estaban vacías (llegaron tras abrir el form).
    pub fn set_zones(&mut self, zones: Vec<ZoneRef>) {
        if self.editing.is_none() && self.zones.is_empty() {
            self.zones = zones;
            self.zone_idx = 0;
        }
    }

    /// Cambia el dominio seleccionado (select con ←→).
    pub fn cycle_zone(&mut self, delta: i32) {
        let n = self.zones.len() as i32;
        if n == 0 {
            return;
        }
        self.zone_idx = ((((self.zone_idx as i32 + delta) % n) + n) % n) as usize;
    }

    pub fn zone(&self) -> Option<&ZoneRef> {
        self.zones.get(self.zone_idx)
    }

    /// Nombre de host completo. Al editar es el hostname fijo; al crear se
    /// compone de subdominio + dominio (o solo dominio en el apex).
    pub fn full_hostname(&self) -> Option<String> {
        if let Some(h) = &self.editing {
            return Some(h.clone());
        }
        let domain = &self.zone()?.name;
        let sub = self.subdomain.value().trim();
        Some(if sub.is_empty() {
            domain.clone()
        } else {
            format!("{sub}.{domain}")
        })
    }

    /// Campo de texto activo (`None` en el select Domain).
    pub fn active_text_mut(&mut self) -> Option<&mut TextInput> {
        match self.current() {
            RouteField::Subdomain => Some(&mut self.subdomain),
            RouteField::Path => Some(&mut self.path),
            RouteField::Service => Some(&mut self.service),
            RouteField::Domain => None,
        }
    }
}

/// Ayuda contextual: secciones de atajos aplicables al foco actual.
pub struct Help {
    pub sections: Vec<HelpSection>,
}

pub struct HelpSection {
    pub title: String,
    pub items: Vec<(String, String)>,
}

impl HelpSection {
    pub fn new(title: &str, items: Vec<(&str, &str)>) -> Self {
        Self {
            title: title.into(),
            items: items
                .into_iter()
                .map(|(k, d)| (k.into(), d.into()))
                .collect(),
        }
    }
}

/// Estado del campo de entrada del token.
#[derive(Default)]
pub struct TokenEntry {
    pub input: TextInput,
    pub error: Option<String>,
    pub verifying: bool,
}

/// Diálogo de confirmación: `on_yes` se despacha al aceptar.
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub on_yes: Action,
}

/// Fila del selector: una cuenta de una sesión (token) concreta.
pub struct AccountRow {
    pub label: String,
    /// Índice de la sesión (token) dueña de la cuenta.
    pub session: usize,
    /// Índice de la cuenta dentro de la sesión.
    pub account: usize,
    /// `true` si es la cuenta activa actual.
    pub active: bool,
}

/// Selector de cuenta activa (todas las cuentas de todos los tokens).
pub struct AccountPicker {
    pub rows: Vec<AccountRow>,
    pub state: ListState,
}

impl AccountPicker {
    pub fn new(rows: Vec<AccountRow>) -> Self {
        let mut state = ListState::default();
        let active = rows.iter().position(|r| r.active).unwrap_or(0);
        state.select((!rows.is_empty()).then_some(active));
        Self { rows, state }
    }
    pub fn move_by(&mut self, delta: i32) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let n = len as i32;
        self.state.select(Some(((((cur + delta) % n) + n) % n) as usize));
    }
    pub fn selected_row(&self) -> Option<&AccountRow> {
        self.state.selected().and_then(|i| self.rows.get(i))
    }
}

pub struct Message {
    pub title: String,
    pub body: String,
    pub is_error: bool,
}

/// Dibuja el popup activo centrado sobre `area`.
pub fn draw(frame: &mut Frame, area: Rect, popup: &mut Popup) {
    match popup {
        Popup::Token(entry) => draw_token(frame, area, entry),
        Popup::Confirm(c) => draw_confirm(frame, area, c),
        Popup::AccountPicker(p) => draw_account_picker(frame, area, p),
        Popup::Help(h) => draw_help(frame, area, h),
        Popup::NewTunnel(t) => draw_new_tunnel(frame, area, t),
        Popup::NewBucket(b) => draw_new_bucket(frame, area, b),
        Popup::Upload(u) => draw_upload(frame, area, u),
        Popup::Rename(r) => draw_rename(frame, area, r),
        Popup::R2Creds(c) => draw_r2_creds(frame, area, c),
        Popup::Presign(p) => draw_presign(frame, area, p),
        Popup::ImageView(v) => draw_image_view(frame, area, v),
        Popup::RecordForm(f) => draw_record_form(frame, area, f),
        Popup::RouteForm(f) => draw_route_form(frame, area, f),
        Popup::HttpTest(t) => draw_http_test(frame, area, t),
        Popup::BindingEdit(b) => draw_binding_edit(frame, area, b),
        Popup::CorsEdit(c) => draw_cors_edit(frame, area, c),
        Popup::ChooseDomain(c) => draw_choose_domain(frame, area, c),
        Popup::SearchInput(s) => draw_search_input(frame, area, s),
        Popup::NewFolder(f) => draw_new_folder(frame, area, f),
        Popup::BucketDomains(d) => draw_bucket_domains(frame, area, d),
        Popup::DomainAdd(f) => draw_domain_add(frame, area, f),
        Popup::LogDetail(d) => draw_log_detail(frame, area, d),
        Popup::Message(msg) => draw_message(frame, area, msg),
    }
}

fn draw_log_detail(frame: &mut Frame, area: Rect, d: &LogDetail) {
    let width = (area.width.saturating_mul(4) / 5).max(50);
    let height = (area.height.saturating_mul(4) / 5).max(10);
    let rect = layout::centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(format!(" {} ", d.title))
        .title_bottom(" ↑↓/PgUp/PgDn desplazar · y copiar JSON · Esc cerrar ")
        .border_style(theme::border(true))
        .title_style(theme::title(true));
    let lines: Vec<Line> = d
        .lines
        .iter()
        .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme::FG))))
        .collect();
    let body = Paragraph::new(lines).block(block).scroll((d.scroll, 0));
    frame.render_widget(body, rect);
}

fn draw_binding_edit(frame: &mut Frame, area: Rect, b: &BindingEdit) {
    let title = if b.adding {
        " ＋ Nuevo secreto "
    } else if b.is_secret {
        " 🔒 Editar secreto "
    } else {
        " ✎ Editar variable "
    };

    let mut lines: Vec<Line> = Vec::new();
    // Campo nombre (editable solo al añadir; si no, informativo).
    if b.adding {
        lines.push(field_line("Nombre", &b.name, b.field == 0));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<10}", "Nombre"), Style::default().fg(theme::DIM)),
            Span::styled(b.name.value().to_string(), Style::default().fg(theme::FG)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<10}", "Tipo"), Style::default().fg(theme::DIM)),
            Span::styled(
                if b.is_secret { "secret_text" } else { "plain_text" }.to_string(),
                Style::default().fg(theme::FG),
            ),
        ]));
    }
    let value_active = b.field == b.field_count() - 1;
    lines.push(field_line("Valor", &b.value, value_active));

    lines.push(Line::from(""));
    let hint = if b.submitting {
        Span::styled("Guardando…", Style::default().fg(theme::ACCENT))
    } else if let Some(e) = &b.error {
        Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))
    } else if b.is_secret {
        Span::styled(
            "Enter guardar · Esc cancelar · el valor no se muestra tras guardar",
            Style::default().fg(theme::DIM),
        )
    } else {
        Span::styled(
            "↑↓ campo · Enter guardar · Esc cancelar",
            Style::default().fg(theme::DIM),
        )
    };
    lines.push(Line::from(hint));

    let height = (lines.len() as u16 + 2).clamp(8, area.height);
    let rect = layout::centered(area, 66, height);
    frame.render_widget(Clear, rect);
    let body = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(title)
                .border_style(theme::border(true))
                .title_style(theme::title(true)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

/// Línea de campo editable con marcador de foco y cursor.
fn field_line(label: &str, input: &TextInput, active: bool) -> Line<'static> {
    let marker = if active { "▶ " } else { "  " };
    let label_style = if active {
        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::DIM)
    };
    let mut spans = vec![Span::styled(format!("{marker}{label:<10}"), label_style)];
    spans.extend(input.spans(active));
    Line::from(spans)
}

fn draw_http_test(frame: &mut Frame, area: Rect, t: &HttpTest) {
    let rect = layout::centered(area, 74, 8);
    frame.render_widget(Clear, rect);
    let status: Line = if t.sending {
        Line::from(Span::styled("Enviando…", Style::default().fg(theme::ACCENT)))
    } else if let Some(e) = &t.error {
        Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR)))
    } else {
        Line::from(Span::styled(
            "Enter enviar GET · Esc cancelar",
            Style::default().fg(theme::DIM),
        ))
    };
    let body = Paragraph::new(vec![
        Line::from("URL a probar (GET):"),
        Line::from(""),
        Line::from(t.url.spans(!t.sending)),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 🧪 Probar ruta ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_upload(frame: &mut Frame, area: Rect, u: &UploadForm) {
    let rect = layout::centered(area, 76, 9);
    frame.render_widget(Clear, rect);
    let status: Line = if u.submitting {
        Line::from(Span::styled("Subiendo…", Style::default().fg(theme::ACCENT)))
    } else if let Some(e) = &u.error {
        Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR)))
    } else {
        Line::from(Span::styled(
            "Enter subir · Esc cancelar",
            Style::default().fg(theme::DIM),
        ))
    };
    let body = Paragraph::new(vec![
        Line::from(Span::styled(
            format!("Destino: {}", u.dest),
            Style::default().fg(theme::DIM),
        )),
        Line::from(""),
        Line::from("Ruta del archivo local:"),
        Line::from(u.path.spans(!u.submitting)),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(" ⬆ Subir objeto ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_rename(frame: &mut Frame, area: Rect, r: &RenameForm) {
    let rect = layout::centered(area, if r.move_mode { 76 } else { 70 }, 10);
    frame.render_widget(Clear, rect);
    // Renombrar muestra solo el nombre de archivo; mover, la clave completa.
    let shown_old = if r.move_mode {
        r.old_key.as_str()
    } else {
        r.old_key.rsplit('/').next().unwrap_or(&r.old_key)
    };
    let verb = if r.move_mode { "Moviendo…" } else { "Renombrando…" };
    let status: Line = if r.submitting {
        Line::from(Span::styled(verb, Style::default().fg(theme::ACCENT)))
    } else if let Some(e) = &r.error {
        Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR)))
    } else if r.move_mode {
        Line::from(Span::styled(
            "Enter mover · Esc cancelar · sobrescribe si el destino ya existe fuera de lo listado",
            Style::default().fg(theme::DIM),
        ))
    } else {
        Line::from(Span::styled(
            "Enter renombrar · Esc cancelar",
            Style::default().fg(theme::DIM),
        ))
    };
    let label = if r.move_mode {
        "Clave destino (con carpeta):"
    } else {
        "Nuevo nombre:"
    };
    let body = Paragraph::new(vec![
        Line::from(Span::styled(
            format!("Actual: {shown_old}"),
            Style::default().fg(theme::DIM),
        )),
        Line::from(""),
        Line::from(label),
        Line::from(r.name.spans(!r.submitting)),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(if r.move_mode {
                " ⇢ Mover objeto "
            } else {
                " ✎ Renombrar objeto "
            })
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_r2_creds(frame: &mut Frame, area: Rect, c: &R2CredsForm) {
    let rect = layout::centered(area, 76, 11);
    frame.render_widget(Clear, rect);
    let status: Line = match &c.error {
        Some(e) => Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))),
        None => Line::from(Span::styled(
            "↑↓ campo · Enter guardar · Esc cancelar",
            Style::default().fg(theme::DIM),
        )),
    };
    let field = |label: &str, input: &TextInput, active: bool, mask: bool| -> Line<'static> {
        let marker = if active { "▶ " } else { "  " };
        let style = if active {
            Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::DIM)
        };
        let mut spans = vec![Span::styled(format!("{marker}{label:<12}"), style)];
        if mask {
            spans.push(Span::styled(
                "•".repeat(input.value().chars().count()),
                Style::default().fg(theme::FG),
            ));
            if active {
                spans.push(Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)));
            }
        } else {
            spans.extend(input.spans(active));
        }
        Line::from(spans)
    };
    let body = Paragraph::new(vec![
        Line::from(Span::styled(
            "Credenciales R2 (API Token S3 → se guardan en el keyring):",
            Style::default().fg(theme::DIM),
        )),
        Line::from(""),
        field("Access Key", &c.access_key, c.field == 0, false),
        field("Secret", &c.secret, c.field == 1, true),
        Line::from(""),
        Line::from(Span::styled(
            "Créalas en dash.cloudflare.com → R2 → Manage API Tokens",
            Style::default().fg(theme::DIM),
        )),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 🔑 Credenciales R2 ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_presign(frame: &mut Frame, area: Rect, p: &PresignForm) {
    let rect = layout::centered(area, 70, 9);
    frame.render_widget(Clear, rect);
    let status: Line = match &p.error {
        Some(e) => Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))),
        None => Line::from(Span::styled(
            "Enter generar · Esc cancelar",
            Style::default().fg(theme::DIM),
        )),
    };
    let name = p.key.rsplit('/').next().unwrap_or(&p.key);
    let body = Paragraph::new(vec![
        Line::from(Span::styled(
            format!("Objeto: {name}"),
            Style::default().fg(theme::DIM),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Expira en (segundos, máx 604800): ", Style::default().fg(theme::FG)),
        ]),
        Line::from(p.expires.spans(true)),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 🔗 URL prefirmada ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_image_view(frame: &mut Frame, area: Rect, v: &ImageView) {
    // Tamaño del popup = imagen + borde, recortado a la pantalla.
    let img_w = v.lines.first().map(|l| l.width()).unwrap_or(0) as u16;
    let img_h = v.lines.len() as u16;
    let rect = layout::centered(area, img_w + 2, img_h + 2);
    frame.render_widget(Clear, rect);
    let body = Paragraph::new(v.lines.clone()).block(
        Block::bordered()
            .title(format!(" 🖼 {} · cualquier tecla cierra ", v.title))
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    );
    frame.render_widget(body, rect);
}

fn draw_new_bucket(frame: &mut Frame, area: Rect, b: &NewBucket) {
    let rect = layout::centered(area, 56, 8);
    frame.render_widget(Clear, rect);
    let status: Line = match &b.error {
        Some(e) => Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))),
        None => Line::from(Span::styled(
            "Enter crear · Esc cancelar",
            Style::default().fg(theme::DIM),
        )),
    };
    let body = Paragraph::new(vec![
        Line::from("Nombre del nuevo bucket:"),
        Line::from(""),
        Line::from(b.name.spans(true)),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 📦 Nuevo bucket ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    );
    frame.render_widget(body, rect);
}

fn draw_new_tunnel(frame: &mut Frame, area: Rect, t: &NewTunnel) {
    let rect = layout::centered(area, 56, 8);
    frame.render_widget(Clear, rect);
    let status: Line = match &t.error {
        Some(e) => Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))),
        None => Line::from(Span::styled(
            "Enter crear · Esc cancelar",
            Style::default().fg(theme::DIM),
        )),
    };
    let body = Paragraph::new(vec![
        Line::from("Nombre del nuevo túnel:"),
        Line::from(""),
        Line::from(t.name.spans(true)),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 🚇 Nuevo túnel ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    );
    frame.render_widget(body, rect);
}

fn draw_record_form(frame: &mut Frame, area: Rect, f: &RecordForm) {
    let visible = f.visible();
    let title = if f.editing_id.is_some() {
        " ✎ Editar registro "
    } else {
        " ＋ Nuevo registro "
    };

    let mut lines: Vec<Line> = Vec::new();
    for field in &visible {
        let active = f.current() == *field;
        let marker = if active { "▶ " } else { "  " };
        let label_style = if active {
            Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::DIM)
        };

        let (label, value): (&str, Vec<Span>) = match field {
            RField::Type => (
                "Tipo",
                if active {
                    vec![Span::styled(
                        format!("‹ {} ›", f.rtype()),
                        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                    )]
                } else {
                    vec![Span::styled(f.rtype().to_string(), Style::default().fg(theme::FG))]
                },
            ),
            RField::Name => ("Nombre", f.name.spans(active)),
            RField::Content => (f.content_label(), f.content.spans(active)),
            RField::Priority => ("Prioridad", f.priority.spans(active)),
            RField::Proxy => (
                "Proxy",
                if f.proxied {
                    vec![Span::styled("● on", Style::default().fg(theme::ACCENT))]
                } else {
                    vec![Span::styled("○ off", Style::default().fg(theme::DIM))]
                },
            ),
            // TTL: si no está activo y vale "1", muestra "1 (auto)"; si no, editable.
            RField::Ttl => (
                "TTL",
                if !active && f.ttl.value() == "1" {
                    vec![Span::styled("1 (auto)", Style::default().fg(theme::FG))]
                } else {
                    f.ttl.spans(active)
                },
            ),
        };

        let mut spans = vec![Span::styled(format!("{marker}{label:<12}"), label_style)];
        spans.extend(value);
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    let hint = if f.submitting {
        Span::styled("Guardando…", Style::default().fg(theme::ACCENT))
    } else if let Some(e) = &f.error {
        Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))
    } else {
        Span::styled(
            "↑↓ campo · ←→ tipo · Espacio proxy · Enter guardar · Esc",
            Style::default().fg(theme::DIM),
        )
    };
    lines.push(Line::from(hint));

    let height = (lines.len() as u16 + 2).clamp(8, area.height);
    let rect = layout::centered(area, 66, height);
    frame.render_widget(Clear, rect);
    let body = Paragraph::new(lines).block(
        Block::bordered()
            .title(title)
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    );
    frame.render_widget(body, rect);
}

fn draw_route_form(frame: &mut Frame, area: Rect, f: &RouteForm) {
    let cur = f.current();
    let row = |label: &str, value: Vec<Span<'static>>, active: bool| -> Line<'static> {
        let marker = if active { "▶ " } else { "  " };
        let label_style = if active {
            Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::DIM)
        };
        let mut spans = vec![Span::styled(format!("{marker}{label:<11}"), label_style)];
        spans.extend(value);
        Line::from(spans)
    };
    // Campo de texto con placeholder tenue si está vacío y sin foco.
    let text_field = |label: &str, input: &TextInput, ph: &str, fld: RouteField| -> Line<'static> {
        let active = cur == fld;
        let value = if input.value().is_empty() && !active {
            vec![Span::styled(ph.to_string(), Style::default().fg(theme::DIM))]
        } else {
            input.spans(active)
        };
        row(label, value, active)
    };

    let editing = f.editing.is_some();
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Túnel: {}", f.tunnel_name),
        Style::default().fg(theme::DIM),
    )));

    if let Some(host) = &f.editing {
        // Edición: hostname fijo (solo servicio y ruta cambian).
        lines.push(Line::from(vec![
            Span::styled("  Host        ", Style::default().fg(theme::DIM)),
            Span::styled(host.clone(), Style::default().fg(theme::ACCENT)),
        ]));
    } else {
        lines.push(text_field(
            "Subdominio",
            &f.subdomain,
            "www, blog, api (opcional)",
            RouteField::Subdomain,
        ));
        // Dominio: select entre las zonas de la cuenta.
        let domain_active = cur == RouteField::Domain;
        let domain_val = match f.zone() {
            Some(z) if domain_active => vec![Span::styled(
                format!("‹ {} ›", z.name),
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            )],
            Some(z) => vec![Span::styled(z.name.clone(), Style::default().fg(theme::FG))],
            None => vec![Span::styled(
                "(sin zonas en la cuenta)",
                Style::default().fg(theme::ERROR),
            )],
        };
        lines.push(row("Dominio", domain_val, domain_active));
        // Nombre de host completo (calculado).
        lines.push(Line::from(vec![
            Span::styled("  Host completo ", Style::default().fg(theme::DIM)),
            Span::styled(
                f.full_hostname().unwrap_or_else(|| "—".into()),
                Style::default().fg(theme::ACCENT),
            ),
        ]));
    }
    lines.push(text_field("Ruta", &f.path, "^/blog (opcional)", RouteField::Path));
    lines.push(text_field(
        "Servicio",
        &f.service,
        "https://localhost:8080",
        RouteField::Service,
    ));

    lines.push(Line::from(""));
    let hint = if f.submitting {
        Span::styled("Guardando…", Style::default().fg(theme::ACCENT))
    } else if let Some(e) = &f.error {
        Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))
    } else if editing {
        Span::styled(
            "↑↓ campo · Enter guardar · Esc",
            Style::default().fg(theme::DIM),
        )
    } else {
        Span::styled(
            "↑↓ campo · ←→ dominio · Enter guardar · Esc",
            Style::default().fg(theme::DIM),
        )
    };
    lines.push(Line::from(hint));

    let title = if editing {
        " ✎ Editar ruta "
    } else {
        " ＋ Agregar aplicación publicada "
    };
    let height = (lines.len() as u16 + 2).clamp(9, area.height);
    let rect = layout::centered(area, 70, height);
    frame.render_widget(Clear, rect);
    let body = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(title)
                .border_style(theme::border(true))
                .title_style(theme::title(true)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

/// Modal con los atajos del contexto actual (secciones dadas por `app.rs`).
fn draw_help(frame: &mut Frame, area: Rect, help: &Help) {
    let mut lines: Vec<Line> = Vec::new();
    for (i, sec) in help.sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(sec.title.clone(), theme::title(true))));
        for (keys, desc) in &sec.items {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {keys:<14}"),
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc.clone(), Style::default().fg(theme::FG)),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Cualquier tecla para cerrar",
        Style::default().fg(theme::DIM),
    )));

    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = layout::centered(area, 54, height);
    frame.render_widget(Clear, rect);
    let body = Paragraph::new(lines).block(
        Block::bordered()
            .title(" ⌨ Atajos ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    );
    frame.render_widget(body, rect);
}

fn draw_token(frame: &mut Frame, area: Rect, entry: &TokenEntry) {
    let rect = layout::centered(area, 72, 16);
    frame.render_widget(Clear, rect);

    // Token enmascarado con cursor de bloque en su posición.
    let n = entry.input.value().chars().count();
    let cur = entry.input.cursor().min(n);
    let bold = Style::default().fg(theme::FG).add_modifier(Modifier::BOLD);
    let (at, after) = if cur < n {
        ("•".to_string(), "•".repeat(n - cur - 1))
    } else {
        (" ".to_string(), String::new())
    };
    let masked_line = Line::from(vec![
        Span::styled("•".repeat(cur), bold),
        Span::styled(at, Style::default().add_modifier(Modifier::REVERSED)),
        Span::styled(after, bold),
    ]);
    let status: Line = if entry.verifying {
        Line::from(Span::styled("Verificando…", Style::default().fg(theme::ACCENT)))
    } else if let Some(err) = &entry.error {
        Line::from(Span::styled(
            format!("✗ {err}"),
            Style::default().fg(theme::ERROR),
        ))
    } else {
        Line::from(Span::styled(
            "Enter verificar · Ctrl-O abrir dashboard · Ctrl-C salir",
            Style::default().fg(theme::DIM),
        ))
    };

    let dim = Style::default().fg(theme::DIM);
    let body = Paragraph::new(vec![
        Line::from("Pega tu API Token de Cloudflare:"),
        Line::from(""),
        masked_line,
        Line::from(""),
        Line::from(Span::styled("Crea un Custom Token con estos permisos:", dim)),
        Line::from(Span::styled(
            "  Zone:  DNS·Edit · Cache Purge · Zone·Read · Analytics·Read",
            dim,
        )),
        Line::from(Span::styled(
            "  Account:  Workers · D1 · Queues · Tunnel · R2·Edit",
            dim,
        )),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 🔑 Autenticación ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_confirm(frame: &mut Frame, area: Rect, c: &Confirm) {
    let rect = layout::centered(area, 60, 8);
    frame.render_widget(Clear, rect);
    let body = Paragraph::new(vec![
        Line::from(Span::styled(c.body.clone(), Style::default().fg(theme::FG))),
        Line::from(""),
        Line::from(Span::styled(
            "s/Enter confirmar · n/Esc cancelar",
            Style::default().fg(theme::DIM),
        )),
    ])
    .block(
        Block::bordered()
            .title(format!(" {} ", c.title))
            .border_style(Style::default().fg(theme::ERROR))
            .title_style(Style::default().fg(theme::ERROR).add_modifier(Modifier::BOLD)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_cors_edit(frame: &mut Frame, area: Rect, c: &CorsEditForm) {
    // Popup grande (editor JSON multilínea), como el editor SQL de D1.
    let width = (area.width.saturating_mul(4) / 5).max(40);
    let height = (area.height.saturating_mul(4) / 5).max(10);
    let rect = layout::centered(area, width, height);
    frame.render_widget(Clear, rect);

    let block = Block::bordered()
        .title(format!(" ⚙ CORS · {} ", c.bucket))
        .border_style(theme::border(true))
        .title_style(theme::title(true));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let rows = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([ratatui::layout::Constraint::Min(1), ratatui::layout::Constraint::Length(1)])
        .split(inner);
    frame.render_widget(Paragraph::new(c.json.lines(true)), rows[0]);

    let hint = if c.submitting {
        Span::styled("Guardando…", Style::default().fg(theme::ACCENT))
    } else if let Some(e) = &c.error {
        Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))
    } else {
        Span::styled(
            "Ctrl+Enter / F5 guardar · Esc cancelar",
            Style::default().fg(theme::DIM),
        )
    };
    frame.render_widget(Paragraph::new(Line::from(hint)), rows[1]);
}

fn draw_choose_domain(frame: &mut Frame, area: Rect, c: &mut ChooseDomain) {
    let h = (c.choices.len() as u16 + 4).clamp(6, 14);
    let rect = layout::centered(area, 66, h);
    frame.render_widget(Clear, rect);
    let (title, bottom) = match c.purpose {
        ChoosePurpose::Abrir => (" Abrir objeto — elige dominio ", " Enter abrir · Esc cancelar "),
        ChoosePurpose::Copiar => (" Copiar URL — elige dominio ", " Enter copiar · Esc cancelar "),
    };
    let items: Vec<ListItem> = c
        .choices
        .iter()
        .map(|d| ListItem::new(Line::from(Span::styled(d.label.clone(), Style::default().fg(theme::FG)))))
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(title)
                .title_bottom(bottom)
                .border_style(theme::border(true))
                .title_style(theme::title(true)),
        )
        .highlight_style(theme::selection())
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, rect, &mut c.state);
}

fn draw_search_input(frame: &mut Frame, area: Rect, s: &SearchInput) {
    let rect = layout::centered(area, 62, 8);
    frame.render_widget(Clear, rect);
    let status: Line = match &s.error {
        Some(e) => Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))),
        None => Line::from(Span::styled(
            "Enter buscar · Esc cancelar",
            Style::default().fg(theme::DIM),
        )),
    };
    let body = Paragraph::new(vec![
        Line::from("Término (subcadena, sin distinguir mayúsculas):"),
        Line::from(""),
        Line::from(s.term.spans(true)),
        Line::from(""),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 🔎 Buscar en el bucket ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    );
    frame.render_widget(body, rect);
}

fn draw_new_folder(frame: &mut Frame, area: Rect, f: &NewFolder) {
    let rect = layout::centered(area, 62, 9);
    frame.render_widget(Clear, rect);
    let status: Line = match &f.error {
        Some(e) => Line::from(Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))),
        None => Line::from(Span::styled(
            "Enter crear · Esc cancelar",
            Style::default().fg(theme::DIM),
        )),
    };
    let body = Paragraph::new(vec![
        Line::from("Nombre de la carpeta:"),
        Line::from(""),
        Line::from(f.name.spans(true)),
        Line::from(""),
        Line::from(Span::styled(
            "Se crea un objeto marcador vacío (como en el dashboard)",
            Style::default().fg(theme::DIM),
        )),
        status,
    ])
    .block(
        Block::bordered()
            .title(" 📁 Nueva carpeta ")
            .border_style(theme::border(true))
            .title_style(theme::title(true)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_bucket_domains(frame: &mut Frame, area: Rect, d: &mut BucketDomains) {
    let h = (d.rows.len() as u16 + 4).clamp(6, 16);
    let rect = layout::centered(area, 66, h);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(format!(" 🌐 Dominios · {} ", d.bucket))
        .title_bottom(" a añadir · d quitar · Esc ")
        .border_style(theme::border(true))
        .title_style(theme::title(true));
    if d.rows.is_empty() {
        let body = Paragraph::new("Sin dominios personalizados · pulsa 'a' para conectar uno")
            .block(block)
            .style(Style::default().fg(theme::DIM))
            .wrap(Wrap { trim: true });
        frame.render_widget(body, rect);
        return;
    }
    let items: Vec<ListItem> = d
        .rows
        .iter()
        .map(|dom| {
            let (text, color) = if dom.enabled {
                (dom.domain.clone(), theme::FG)
            } else {
                (format!("{} (deshabilitado)", dom.domain), theme::DIM)
            };
            ListItem::new(Line::from(Span::styled(text, Style::default().fg(color))))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selection())
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, rect, &mut d.state);
}

fn draw_domain_add(frame: &mut Frame, area: Rect, f: &DomainAddForm) {
    let row = |label: &str, value: Vec<Span<'static>>, active: bool| -> Line<'static> {
        let marker = if active { "▶ " } else { "  " };
        let label_style = if active {
            Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::DIM)
        };
        let mut spans = vec![Span::styled(format!("{marker}{label:<11}"), label_style)];
        spans.extend(value);
        Line::from(spans)
    };

    let sub_active = f.field == 0;
    let sub_val = if f.subdomain.value().is_empty() && !sub_active {
        vec![Span::styled(
            "assets, cdn (vacío = apex)".to_string(),
            Style::default().fg(theme::DIM),
        )]
    } else {
        f.subdomain.spans(sub_active)
    };
    let zone_active = f.field == 1;
    let zone_val = match f.zone() {
        Some(z) if zone_active => vec![Span::styled(
            format!("‹ {} ›", z.name),
            Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
        )],
        Some(z) => vec![Span::styled(z.name.clone(), Style::default().fg(theme::FG))],
        None => vec![Span::styled(
            "(sin zonas en la cuenta)".to_string(),
            Style::default().fg(theme::ERROR),
        )],
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Bucket: {}", f.bucket),
            Style::default().fg(theme::DIM),
        )),
        row("Subdominio", sub_val, sub_active),
        row("Dominio", zone_val, zone_active),
        Line::from(vec![
            Span::styled("  Host completo ", Style::default().fg(theme::DIM)),
            Span::styled(
                f.full_domain().unwrap_or_else(|| "—".into()),
                Style::default().fg(theme::ACCENT),
            ),
        ]),
        Line::from(""),
    ];
    let hint = if f.submitting {
        Span::styled("Conectando…", Style::default().fg(theme::ACCENT))
    } else if let Some(e) = &f.error {
        Span::styled(format!("✗ {e}"), Style::default().fg(theme::ERROR))
    } else {
        Span::styled(
            "↑↓ campo · ←→ dominio · Enter conectar · Esc",
            Style::default().fg(theme::DIM),
        )
    };
    lines.push(Line::from(hint));

    let height = (lines.len() as u16 + 2).clamp(9, area.height);
    let rect = layout::centered(area, 70, height);
    frame.render_widget(Clear, rect);
    let body = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(" ＋ Conectar dominio ")
                .border_style(theme::border(true))
                .title_style(theme::title(true)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}

fn draw_account_picker(frame: &mut Frame, area: Rect, p: &mut AccountPicker) {
    let h = (p.rows.len() as u16 + 5).clamp(7, 20);
    let rect = layout::centered(area, 68, h);
    frame.render_widget(Clear, rect);
    let items: Vec<ListItem> = p
        .rows
        .iter()
        .map(|r| {
            let marker = if r.active { "● " } else { "  " };
            let style = if r.active {
                Style::default().fg(theme::ACCENT)
            } else {
                Style::default().fg(theme::FG)
            };
            ListItem::new(Line::from(Span::styled(
                format!("{marker}{}", r.label),
                style,
            )))
        })
        .collect();
    let block = Block::bordered()
        .title(" Cuentas ")
        .title_bottom(" Enter cambiar · a añadir token · d borrar token · Esc ")
        .border_style(theme::border(true))
        .title_style(theme::title(true));
    if p.rows.is_empty() {
        let body = Paragraph::new("Sin cuentas · pulsa 'a' para añadir un token")
            .block(block)
            .style(Style::default().fg(theme::DIM));
        frame.render_widget(body, rect);
        return;
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selection())
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, rect, &mut p.state);
}

fn draw_message(frame: &mut Frame, area: Rect, msg: &Message) {
    // Alto dinámico: cuerpos largos (p. ej. URLs prefirmadas) necesitan espacio.
    let width: u16 = 76;
    let inner = width.saturating_sub(2).max(1) as usize;
    let body_rows: usize = msg
        .body
        .lines()
        .map(|l| l.chars().count().div_ceil(inner).max(1))
        .sum();
    let rect = layout::centered(area, width, (body_rows as u16 + 5).clamp(8, area.height));
    frame.render_widget(Clear, rect);
    let color = if msg.is_error { theme::ERROR } else { theme::ACCENT };
    let body = Paragraph::new(vec![
        Line::from(Span::styled(msg.body.clone(), Style::default().fg(theme::FG))),
        Line::from(""),
        Line::from(Span::styled(
            "Enter/Esc para cerrar",
            Style::default().fg(theme::DIM),
        )),
    ])
    .block(
        Block::bordered()
            .title(format!(" {} ", msg.title))
            .border_style(Style::default().fg(color))
            .title_style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(body, rect);
}
