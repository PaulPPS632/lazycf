//! Overlays modales: entrada de token, confirmaciones, selector de cuenta y
//! mensajes. El estado lo posee `app.rs`; aquí van los datos y el render.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::action::Action;
use crate::components::input::TextInput;
use crate::model::{Account, Binding, DnsRecord};
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
    /// Formulario de crear/editar registro DNS.
    RecordForm(RecordForm),
    /// Prueba HTTP de una ruta de Worker.
    HttpTest(HttpTest),
    /// Editar/añadir una variable o secreto de un Worker.
    BindingEdit(BindingEdit),
    /// Mensaje informativo o de error.
    Message(Message),
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

/// Selector de cuenta activa.
pub struct AccountPicker {
    pub accounts: Vec<Account>,
    pub state: ListState,
}

impl AccountPicker {
    pub fn new(accounts: Vec<Account>, active: usize) -> Self {
        let mut state = ListState::default();
        state.select(Some(active.min(accounts.len().saturating_sub(1))));
        Self { accounts, state }
    }
    pub fn move_by(&mut self, delta: i32) {
        let len = self.accounts.len();
        if len == 0 {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let n = len as i32;
        self.state.select(Some(((((cur + delta) % n) + n) % n) as usize));
    }
    pub fn selected(&self) -> usize {
        self.state.selected().unwrap_or(0)
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
        Popup::RecordForm(f) => draw_record_form(frame, area, f),
        Popup::HttpTest(t) => draw_http_test(frame, area, t),
        Popup::BindingEdit(b) => draw_binding_edit(frame, area, b),
        Popup::Message(msg) => draw_message(frame, area, msg),
    }
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

fn draw_account_picker(frame: &mut Frame, area: Rect, p: &mut AccountPicker) {
    let h = (p.accounts.len() as u16 + 4).clamp(6, 18);
    let rect = layout::centered(area, 60, h);
    frame.render_widget(Clear, rect);
    let items: Vec<ListItem> = p
        .accounts
        .iter()
        .map(|a| ListItem::new(a.name.clone()))
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Cuentas · Enter selecciona · Esc cierra ")
                .border_style(theme::border(true))
                .title_style(theme::title(true)),
        )
        .highlight_style(theme::selection())
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, rect, &mut p.state);
}

fn draw_message(frame: &mut Frame, area: Rect, msg: &Message) {
    let rect = layout::centered(area, 60, 8);
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
