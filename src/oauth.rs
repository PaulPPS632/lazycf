//! Login OAuth de Cloudflare (Authorization Code + PKCE, cliente público).
//!
//! Cloudflare solo soporta Authorization Code flow; PKCE S256 es obligatorio
//! para clientes públicos (sin `client_secret`). El listener de loopback
//! captura el redirect en `http://localhost:<puerto>/oauth/callback`.
//! Aislado de la TUI: la orquestación (`login`) solo abre el navegador y
//! devuelve los tokens; el wiring de acciones vive en `app`.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Client ID público registrado para lazycf (override: `LAZYCF_OAUTH_CLIENT_ID`).
const DEFAULT_CLIENT_ID: &str = "6a69f4b7c41d52eec65810258e36c79a";
const AUTHORIZE_URL: &str = "https://dash.cloudflare.com/oauth2/auth";
const TOKEN_URL: &str = "https://dash.cloudflare.com/oauth2/token";
const REVOKE_URL: &str = "https://dash.cloudflare.com/oauth2/revoke";
/// Puertos de loopback registrados como `redirect_uri` (match exacto).
const CALLBACK_PORTS: [u16; 3] = [8976, 8977, 8978];
const CALLBACK_PATH: &str = "/oauth/callback";
/// Tiempo máximo esperando la autorización en el navegador.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(120);

/// Scopes solicitados por defecto (override: `LAZYCF_OAUTH_SCOPES`).
///
/// Los clientes OAuth *self-managed* usan IDs de scope en kebab-case con
/// sufijo `.read`/`.write` (NO el formato `account:read` del cliente interno
/// de wrangler). Nombres verificados contra `GET /client/v4/oauth/scopes`
/// (jul 2026). El authorize solo acepta un subconjunto de los scopes
/// registrados en el cliente (*Manage Account → OAuth clients*): si falla
/// con `invalid_scope`, alinear el registro con esta lista.
///
/// Mapeo por módulo: DNS = `dns.write` + `cache.purge` + `analytics.read`;
/// Túneles = `argotunnel.write`; Workers = `workers-scripts.write` +
/// `workers-routes.read` + `workers-tail.read`; métricas GraphQL =
/// `account-analytics.read`; R2 = `workers-r2.write` +
/// `workers-r2-bucket-item.write` (objetos vía API).
const SCOPES: &str = "account-settings.read user-details.read zone.read \
dns.write cache.purge analytics.read account-analytics.read \
argotunnel.write workers-scripts.write workers-routes.read workers-tail.read \
d1.write queues.write workers-r2.write workers-r2-bucket-item.write \
offline_access";

/// Scopes efectivos: env `LAZYCF_OAUTH_SCOPES` o la constante por defecto.
fn scopes() -> String {
    std::env::var("LAZYCF_OAUTH_SCOPES")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| SCOPES.to_string())
}

/// Client ID efectivo: env `LAZYCF_OAUTH_CLIENT_ID` o la constante del binario.
pub fn client_id() -> String {
    std::env::var("LAZYCF_OAUTH_CLIENT_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string())
}

/// Tokens OAuth de una sesión. `expires_at` es unix timestamp (`now +
/// expires_in`) para no requerir la feature `serde` de chrono en el keyring.
#[derive(Serialize, Deserialize, Clone)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub scopes: String,
}

impl std::fmt::Debug for OAuthTokens {
    // Nunca volcar tokens en logs/trazas.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &"<redactado>")
            .field("refresh_token", &"<redactado>")
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl OAuthTokens {
    /// `true` si el access token expiró o expira en menos de `margin` segundos
    /// (margen por clock skew y requests en vuelo).
    pub fn expires_within(&self, margin: i64) -> bool {
        chrono::Utc::now().timestamp() >= self.expires_at - margin
    }
}

/// Respuesta cruda de `POST /oauth2/token`.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    #[serde(default)]
    scope: String,
}

impl From<TokenResponse> for OAuthTokens {
    fn from(r: TokenResponse) -> Self {
        Self {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            expires_at: chrono::Utc::now().timestamp() + r.expires_in,
            scopes: r.scope,
        }
    }
}

// --- PKCE ---

/// `n` bytes aleatorios como base64url sin padding.
fn random_urlsafe(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::rng().fill(&mut bytes[..]);
    URL_SAFE_NO_PAD.encode(&bytes)
}

/// Genera el par PKCE `(code_verifier, code_challenge)` con método S256.
/// El verifier son 43 chars base64url (32 bytes de entropía; rango válido
/// 43–128); el challenge es `base64url(sha256(verifier))`.
pub fn generate_pkce() -> (String, String) {
    let verifier = random_urlsafe(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// URL de autorización con PKCE y `state` anti-CSRF.
pub fn build_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    challenge: &str,
    state: &str,
) -> String {
    let enc = |v: &str| utf8_percent_encode(v, NON_ALPHANUMERIC).to_string();
    format!(
        "{AUTHORIZE_URL}?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &state={}&code_challenge={}&code_challenge_method=S256",
        enc(client_id),
        enc(redirect_uri),
        enc(scopes),
        enc(state),
        enc(challenge),
    )
}

// --- Listener de loopback ---

/// Bindea el primer puerto libre de `CALLBACK_PORTS` y devuelve el listener
/// junto con el `redirect_uri` exacto registrado para ese puerto.
async fn run_loopback() -> Result<(TcpListener, String)> {
    for port in CALLBACK_PORTS {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Ok((listener, format!("http://localhost:{port}{CALLBACK_PATH}")));
        }
    }
    bail!(
        "no hay puerto libre para el callback OAuth (probados {})",
        CALLBACK_PORTS.map(|p| p.to_string()).join(", ")
    )
}

/// Resultado de parsear la query del callback.
enum Callback {
    /// `?code=…&state=…` válidos.
    Code(String),
    /// `?error=…` (p. ej. `access_denied` si el usuario cancela el consent).
    Denied(String),
    /// `state` ausente o distinto del esperado (posible CSRF).
    BadState,
    /// No es el callback (favicon, probes, prefetch…): seguir escuchando.
    Ignore,
}

/// Clasifica la primera línea de la request HTTP del navegador.
fn parse_callback(request_line: &str, expected_state: &str) -> Callback {
    // "GET /oauth/callback?code=…&state=… HTTP/1.1"
    let Some(target) = request_line
        .strip_prefix("GET ")
        .and_then(|r| r.split(' ').next())
    else {
        return Callback::Ignore;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Callback::Ignore;
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_desc = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        // El navegador manda '+' por espacio en la query (form-urlencoded).
        let v = percent_decode_str(&v.replace('+', " "))
            .decode_utf8_lossy()
            .into_owned();
        match k {
            "code" => code = Some(v),
            "state" => state = Some(v),
            "error" => error = Some(v),
            "error_description" => error_desc = Some(v),
            _ => {}
        }
    }

    if let Some(e) = error {
        return Callback::Denied(match error_desc {
            Some(d) if !d.is_empty() => format!("{e}: {d}"),
            _ => e,
        });
    }
    if state.as_deref() != Some(expected_state) {
        return Callback::BadState;
    }
    match code {
        Some(c) if !c.is_empty() => Callback::Code(c),
        _ => Callback::Ignore,
    }
}

/// Responde una página HTML mínima y cierra la conexión.
async fn respond_html(stream: &mut TcpStream, status: &str, body: &str) {
    let page = format!(
        "<!doctype html><html lang=\"es\"><meta charset=\"utf-8\">\
         <title>lazycf</title>\
         <body style=\"font-family:sans-serif;margin:4em auto;max-width:32em;text-align:center\">\
         {body}</body></html>"
    );
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Acepta conexiones en bucle hasta recibir el callback válido (ignora
/// favicon/probes; NO toma la primera conexión a ciegas). Valida `state`
/// (CSRF) y devuelve el `code`. `?error=…` corta de inmediato sin esperar
/// al timeout.
async fn wait_for_code(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String> {
    let accept_loop = async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            // La primera línea basta; el resto de la request no importa.
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]);
            let line = head.lines().next().unwrap_or_default();

            match parse_callback(line, expected_state) {
                Callback::Code(code) => {
                    respond_html(
                        &mut stream,
                        "200 OK",
                        "<h2>✓ Autorización completada</h2>\
                         <p>Ya puedes cerrar esta pestaña y volver a lazycf.</p>",
                    )
                    .await;
                    return Ok(code);
                }
                Callback::Denied(e) => {
                    respond_html(
                        &mut stream,
                        "200 OK",
                        "<h2>✗ Autorización cancelada</h2>\
                         <p>Puedes cerrar esta pestaña.</p>",
                    )
                    .await;
                    bail!("autorización denegada: {e}");
                }
                Callback::BadState => {
                    respond_html(
                        &mut stream,
                        "400 Bad Request",
                        "<h2>✗ Petición inválida</h2><p>El parámetro state no coincide.</p>",
                    )
                    .await;
                    bail!("el `state` del callback no coincide (posible CSRF)");
                }
                Callback::Ignore => {
                    respond_html(&mut stream, "404 Not Found", "").await;
                }
            }
        }
    };

    tokio::time::timeout(timeout, accept_loop)
        .await
        .map_err(|_| {
            eyre!(
                "tiempo de espera agotado ({}s) sin autorización",
                timeout.as_secs()
            )
        })?
}

// --- Token endpoint ---

fn oauth_http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("lazycf/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .wrap_err("creando cliente HTTP para OAuth")
}

/// POST al token endpoint con `params` y decodifica `TokenResponse`.
async fn token_request(token_url: &str, params: &[(&str, &str)]) -> Result<OAuthTokens> {
    let resp = oauth_http()?
        .post(token_url)
        .form(params)
        .send()
        .await
        .wrap_err("llamando al token endpoint")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // Error OAuth estándar: { error, error_description }.
        #[derive(Deserialize)]
        struct OAuthErr {
            error: String,
            #[serde(default)]
            error_description: String,
        }
        if let Ok(e) = serde_json::from_str::<OAuthErr>(&text) {
            bail!(
                "{}{}",
                e.error,
                if e.error_description.is_empty() {
                    String::new()
                } else {
                    format!(": {}", e.error_description)
                }
            );
        }
        bail!("token endpoint devolvió HTTP {status}");
    }
    let parsed: TokenResponse =
        serde_json::from_str(&text).wrap_err("decodificando la respuesta del token endpoint")?;
    Ok(parsed.into())
}

/// Intercambia el `code` del callback por tokens (PKCE: envía el verifier).
pub async fn exchange_code(
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokens> {
    token_request(
        TOKEN_URL,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ],
    )
    .await
}

/// Refresca los tokens. Cloudflare rota el `refresh_token` en cada uso: el
/// resultado SIEMPRE debe sustituir a la credencial anterior.
pub async fn refresh(client_id: &str, refresh_token: &str) -> Result<OAuthTokens> {
    refresh_at(TOKEN_URL, client_id, refresh_token).await
}

/// Como [`refresh`] pero con URL inyectable (tests con servidor local).
pub async fn refresh_at(
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<OAuthTokens> {
    token_request(
        token_url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
        ],
    )
    .await
}

/// Revoca el `refresh_token` (al eliminar una sesión OAuth). Best-effort en
/// el caller: si falla, la sesión se elimina localmente igual.
pub async fn revoke(client_id: &str, refresh_token: &str) -> Result<()> {
    let resp = oauth_http()?
        .post(REVOKE_URL)
        .form(&[
            ("client_id", client_id),
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
        ])
        .send()
        .await
        .wrap_err("llamando al revoke endpoint")?;
    let status = resp.status();
    if !status.is_success() {
        bail!("revoke devolvió HTTP {status}");
    }
    Ok(())
}

/// Orquesta el login completo: PKCE → listener → navegador → code → tokens.
/// `notify_url` recibe la URL de autorización para mostrarla como fallback
/// (p. ej. sesión SSH donde el navegador abre en otra máquina).
pub async fn login(client_id: &str, notify_url: impl FnOnce(String) + Send) -> Result<OAuthTokens> {
    let (verifier, challenge) = generate_pkce();
    let state = random_urlsafe(24);
    let (listener, redirect_uri) = run_loopback().await?;
    let url = build_authorize_url(client_id, &redirect_uri, &scopes(), &challenge, &state);

    notify_url(url.clone());
    if let Err(e) = crate::browser::open(&url) {
        tracing::warn!("no se pudo abrir el navegador: {e}");
    }

    let code = wait_for_code(listener, &state, LOGIN_TIMEOUT).await?;
    exchange_code(client_id, &code, &verifier, &redirect_uri).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_valido_y_challenge_s256() {
        let (verifier, challenge) = generate_pkce();
        // Rango RFC 7636: 43–128 chars, charset base64url.
        assert!(
            (43..=128).contains(&verifier.len()),
            "len: {}",
            verifier.len()
        );
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "charset: {verifier}"
        );
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn pkce_es_aleatorio() {
        assert_ne!(generate_pkce().0, generate_pkce().0);
    }

    #[test]
    fn authorize_url_contiene_parametros_codificados() {
        let url = build_authorize_url(
            "cid",
            "http://localhost:8976/oauth/callback",
            "zone:read offline_access",
            "chall",
            "st",
        );
        assert!(url.starts_with("https://dash.cloudflare.com/oauth2/auth?response_type=code"));
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8976%2Foauth%2Fcallback"));
        assert!(url.contains("zone%3Aread%20offline%5Faccess"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn callback_valido_devuelve_el_code() {
        let line = "GET /oauth/callback?code=abc%2F123&state=xyz HTTP/1.1";
        match parse_callback(line, "xyz") {
            Callback::Code(c) => assert_eq!(c, "abc/123"),
            _ => panic!("debió devolver Code"),
        }
    }

    #[test]
    fn callback_con_error_es_denied() {
        let line = "GET /oauth/callback?error=access_denied&state=xyz HTTP/1.1";
        assert!(matches!(
            parse_callback(line, "xyz"),
            Callback::Denied(e) if e == "access_denied"
        ));
    }

    #[test]
    fn callback_con_state_incorrecto_se_rechaza() {
        let line = "GET /oauth/callback?code=abc&state=otro HTTP/1.1";
        assert!(matches!(parse_callback(line, "xyz"), Callback::BadState));
        let sin_state = "GET /oauth/callback?code=abc HTTP/1.1";
        assert!(matches!(
            parse_callback(sin_state, "xyz"),
            Callback::BadState
        ));
    }

    #[test]
    fn requests_basura_se_ignoran() {
        for line in [
            "GET /favicon.ico HTTP/1.1",
            "GET / HTTP/1.1",
            "POST /oauth/callback HTTP/1.1",
            "",
        ] {
            assert!(
                matches!(parse_callback(line, "xyz"), Callback::Ignore),
                "{line}"
            );
        }
    }

    #[tokio::test]
    async fn wait_for_code_ignora_probes_y_acepta_el_callback() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            // 1ª conexión: probe basura → debe ignorarse y seguir escuchando.
            let mut s1 = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s1.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut sink = Vec::new();
            let _ = s1.read_to_end(&mut sink).await;
            // 2ª conexión: callback real.
            let mut s2 = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s2.write_all(b"GET /oauth/callback?code=c0de&state=st HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut sink = Vec::new();
            let _ = s2.read_to_end(&mut sink).await;
        });

        let code = wait_for_code(listener, "st", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(code, "c0de");
    }

    #[tokio::test]
    async fn wait_for_code_corta_inmediato_con_access_denied() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /oauth/callback?error=access_denied&state=st HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut sink = Vec::new();
            let _ = s.read_to_end(&mut sink).await;
        });

        let err = wait_for_code(listener, "st", Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("access_denied"), "{err}");
    }
}
