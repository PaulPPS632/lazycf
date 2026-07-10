# Plan de implementación — OAuth de Cloudflare en lazycf

Objetivo: permitir login vía **OAuth (Authorization Code + PKCE)** además del API
token actual, para reducir la fricción de onboarding. El API token sigue siendo
válido (auth híbrida). El token OAuth es un Bearer, así que encaja en el
`CfClient` existente sin reescribir la capa HTTP.

---

## 1. Restricciones de Cloudflare (recordatorio)

- Solo se soporta **Authorization Code flow**. No hay Device Flow, ni Implicit,
  ni Client Credentials. `response_type = code`.
- Cliente **público** (CLI/TUI): PKCE **obligatorio**, método **S256**, sin
  `client_secret`.
- El `redirect_uri` debe hacer **match exacto** con uno de los registrados.
- `http://` solo permitido en loopback (`localhost` / `127.0.0.1`).
- El access token es **corto**; hay `refresh_token` (scope `offline_access`).

## 2. Cliente registrado (rellenar)

- `CLIENT_ID`: `6a69f4b7c41d52eec65810258e36c79a` — es público, va como constante en el binario.
- `redirect_uris` registrados:
  - `http://localhost:8976/oauth/callback`
  - `http://localhost:8977/oauth/callback`
  - `http://localhost:8978/oauth/callback`
- Scopes solicitados (nombres VERIFICADOS contra `GET /oauth/scopes`, jul 2026 —
  formato self-managed kebab-case, no el estilo wrangler):
  `account-settings.read user-details.read zone.read dns.write cache.purge
  analytics.read account-analytics.read argotunnel.write workers-scripts.write
  workers-routes.read workers-tail.read d1.write queues.write workers-r2.write
  workers-r2-bucket-item.write offline_access`
  - Tunnel = `argotunnel.*`; R2 objetos vía API = `workers-r2-bucket-item.*`.

## 3. Endpoints OAuth (VERIFICAR antes de codear — §12)

Valores que usa wrangler (host `dash.cloudflare.com`):

| Uso        | Método | URL                                            |
|------------|--------|------------------------------------------------|
| Authorize  | GET    | `https://dash.cloudflare.com/oauth2/auth`      |
| Token      | POST   | `https://dash.cloudflare.com/oauth2/token`     |
| Revoke     | POST   | `https://dash.cloudflare.com/oauth2/revoke`    |

`POST /oauth2/token` responde JSON:
`{ access_token, token_type, expires_in, refresh_token, scope }`.

## 4. Arquitectura

Módulo nuevo **`src/oauth.rs`** (aislado, sin dependencias de la TUI):

- `generate_pkce() -> (verifier, challenge)` — verifier = 43–128 chars
  base64url aleatorios; challenge = `base64url(sha256(verifier))` (S256).
  (`sha2` y `base64` ya están en `Cargo.toml`.)
- `build_authorize_url(client_id, redirect_uri, scopes, challenge, state)`.
- `run_loopback(candidatos: &[u16]) -> (listener, redirect_uri)` — bindea el
  primer puerto libre de `[8976, 8977, 8978]` con `tokio::net::TcpListener`.
- `wait_for_code(listener, expected_state, timeout) -> Result<String>` —
  acepta conexiones **en bucle** hasta recibir un `GET /oauth/callback` válido
  (ignora `/favicon.ico`, prefetch y probes; NO tomar la primera conexión a
  ciegas). Parsea `?code=…&state=…`, valida `state` (CSRF), responde una página
  HTML "puedes cerrar esta pestaña", cierra.
  - Si llega `?error=…` (p. ej. `access_denied` cuando el usuario cancela el
    consent) → responder HTML de error y devolver `Err` inmediato, sin esperar
    al timeout.
- `exchange_code(client_id, code, verifier, redirect_uri) -> OAuthTokens`.
- `refresh(client_id, refresh_token) -> OAuthTokens`.
- `revoke(client_id, refresh_token) -> Result<()>` — contra `/oauth2/revoke`;
  se llama al eliminar una sesión OAuth desde la UI.
- `login(client_id) -> OAuthTokens` — orquesta: PKCE → listener → `open`
  navegador → esperar code → intercambiar → devolver tokens.

El navegador se abre con el crate **`open`** (ya presente).

## 5. Dependencias nuevas / ajustes

- `rand = "0.9"` (o `getrandom`) — entropía para `code_verifier` y `state`.
- **`chrono`: falta la feature `serde`** (hoy: `default-features = false,
  features = ["clock"]`). Sin ella, `DateTime<Utc>` dentro del JSON del keyring
  **no compila**. Alternativa más simple: guardar `expires_at` como unix
  timestamp `i64` y evitar el cambio de feature.
  El resto (`sha2`, `base64`, `open`, `reqwest` con `form`/`json`, `tokio net`,
  `keyring`) ya está.

## 6. Modelo de datos

```rust
// src/oauth.rs
#[derive(Serialize, Deserialize, Clone)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64, // unix timestamp: now + expires_in (evita feature serde de chrono)
    pub scopes: String,
}
```

Generalizar la credencial de una sesión. En `secrets.rs` / `app`:

```rust
// OJO serde: internally-tagged (`#[serde(tag = "kind")]`) NO soporta newtype
// variants de primitivos (`ApiToken(String)` falla al serializar en runtime).
// Usar struct variants:
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "kind")]
pub enum Credential {
    ApiToken { token: String },
    OAuth { tokens: OAuthTokens },
}
```

La credencial del env `CLOUDFLARE_API_TOKEN` se marca como **no persistible**
(hoy `persist_tokens` guarda también la del env en el keyring — quirk
preexistente; no arrastrarlo a `persist_credentials`).

## 7. Cambios archivo por archivo

**`src/oauth.rs`** (nuevo): todo lo de §4.

**`src/secrets.rs`**:
- Guardar credenciales mixtas. Cambiar la entrada del keyring de
  `Vec<String>` a `Vec<Credential>` serializado como JSON con tag
  (`serde(tag = "kind")`).
- Migración: al cargar, si el JSON es la lista antigua de strings, mapear cada
  string a `Credential::ApiToken`. Mantener la migración legacy existente.
- Añadir `load_credentials()` / `save_credentials(&[Credential])`. Conservar
  `load_tokens`/`save_tokens` como wrappers o eliminarlos tras migrar callers.
- **Dedup**: el `tokens.contains(&t)` actual (igualdad de string) no aplica a
  OAuth (los tokens rotan). Dedupear API tokens por string; credenciales OAuth
  no se dedupean (o por `access_token` si hiciera falta).

**`src/api/client.rs`**:
- `CfClient` pasa de `token: String` a un origen de credencial refrescable
  **compartido**: `Arc<CredentialSource>` + `client_id` para refrescar in situ.

  ```rust
  pub struct CredentialSource {
      cred: RwLock<Credential>,
      refresh_lock: tokio::sync::Mutex<()>, // single-flight
      on_refresh: UnboundedSender<Action>,  // notifica al App para persistir
  }
  ```
- **Single-flight de refresh (crítico)**: `CfClient` es `Clone` y la TUI lanza
  requests paralelas (DNS + tunnels + workers). Si el access token expiró, N
  tareas reciben 401 a la vez; si cada una refresca y Cloudflare **rota el
  refresh_token en cada uso**, el segundo refresh usa el token viejo → sesión
  muerta. Protocolo:
  1. 401 con credencial OAuth → `refresh_lock.lock().await`.
  2. Tras adquirir el lock, **re-leer** la credencial: si el `access_token`
     cambió (otro task ya refrescó) → soltar lock y reintentar con el nuevo.
  3. Si no cambió → `oauth::refresh`, escribir el resultado en el `RwLock`,
     notificar `on_refresh`, soltar lock, reintentar la petición.
  4. Si el reintento sigue en 401 → error de auth (no loop).
- **Refresco proactivo**: antes de cada request, si `now >= expires_at - 60 s`
  (margen por clock skew y requests en vuelo) → mismo protocolo single-flight.
- `bearer_auth` sigue igual (access token OAuth es Bearer).

**`src/api/mod.rs`** (`authenticate`):
- El token OAuth **no** valida contra `/user/tokens/verify`. Para OAuth, usar
  `GET /accounts` (scope `account:read`) como prueba de vida: si devuelve
  cuentas → activo. Bifurcar según tipo de credencial.

**`src/app/mod.rs`**:
- **Fuente única de verdad**: `Session` NO guarda una copia de la credencial;
  guarda el **mismo** `Arc<CredentialSource>` que su `CfClient`. Si `Session`
  tuviera copia propia, tras un refresh interno del client quedaría stale y
  `persist_credentials` guardaría el refresh_token viejo (ya rotado) →
  al reiniciar lazycf, credencial inválida.
  `Session { source: Arc<CredentialSource>, client: CfClient, accounts }`.
- `persist_tokens` → `persist_credentials`: lee `source.cred` de cada sesión
  (siempre fresco) y guarda la lista, **excluyendo** la credencial del env.
- Nueva acción `Action::CredentialRefreshed`: el client la emite tras cada
  refresh (vía `on_refresh`); el App responde llamando `persist_credentials`.
  `client.rs` no toca `secrets.rs` directamente — la persistencia queda en el
  App, que es quien conoce la lista completa.
- `spawn_verify(token)` → aceptar `Credential`.
- Nueva acción `Action::StartOAuthLogin` y `Action::OAuthCompleted(OAuthTokens)` /
  `Action::OAuthFailed(String)`.
- `mask_token` reutilizable para mostrar credenciales OAuth enmascaradas.

**UI (pantalla de tokens)**:
- Añadir opción **"Iniciar sesión con Cloudflare (OAuth)"** junto a "Añadir API
  token". Al elegirla: estado "Abriendo navegador… esperando autorización",
  lanzar `oauth::login` en `tokio::spawn`, resolver con las acciones nuevas.
- Permitir cancelar (Esc) → abortar el listener.
- **"Eliminar token"** sobre una sesión OAuth: llamar `oauth::revoke` con el
  refresh_token (best-effort: si falla, eliminar localmente igual) antes de
  quitarla del keyring.

**Config / constantes**:
- `CLIENT_ID` como `const` con override por env `LAZYCF_OAUTH_CLIENT_ID`.

## 8. Flujo de login (secuencia)

1. Usuario elige "Login con Cloudflare".
2. `generate_pkce()` → verifier + challenge; `state` aleatorio.
3. Bindear loopback en 8976 → 8977 → 8978 (primer libre) → `redirect_uri`.
4. Construir authorize URL y abrir navegador (`open`).
5. Usuario aprueba consent → Cloudflare redirige a `redirect_uri?code&state`.
6. Listener captura `code`, valida `state`, responde HTML de cierre.
7. `POST /oauth2/token` (`grant_type=authorization_code`, `code`, `client_id`,
   `redirect_uri`, `code_verifier`) → `OAuthTokens`.
8. Calcular `expires_at = now + expires_in`. Guardar en keyring.
9. `authenticate()` (vía `/accounts`) → crear `Session` → cargar cuentas.

## 9. Cobertura por módulo (según scopes)

| Módulo   | Scope necesario                          | Estado con OAuth |
|----------|------------------------------------------|------------------|
| Workers  | `workers_scripts:write` + relacionados   | ✅ completo      |
| D1       | `d1:write`                               | ✅ completo      |
| Queues   | `queues:write`                           | ✅ completo      |
| DNS      | `dns_records:edit` + `zone:read` + `cache_purge:edit` (purge) | ✅ si se pidió el scope |
| Tunnels  | "Cloudflare Tunnel:Edit" (`cfd_tunnel`)  | ⚠️ solo si el scope existe (§12) |
| R2       | bucket mgmt: R2 Storage:Edit             | ⚠️ **parcial**   |

**R2 — importante:** el manejo de *objetos* (`r2.rs`) usa **URLs prefirmadas
SigV4** contra `*.r2.cloudflarestorage.com`, que requieren **R2 Access Key +
Secret** (creds S3, ya guardadas aparte en `secrets.rs`). OAuth **no** las
reemplaza. OAuth solo cubre la gestión de buckets vía API (`/accounts/{}/r2/...`).

## 10. Seguridad y casos borde

- **CSRF**: `state` aleatorio, verificar en el callback; rechazar si no coincide.
- **PKCE**: `code_verifier` solo en memoria durante el flujo; nunca a disco.
- **Timeout** del listener (p. ej. 120 s) → mensaje claro y cancelar.
- **Puerto ocupado**: fallback 8976→8977→8978; si los 3 fallan, error explícito.
- **Refresh fallido** (refresh_token revocado/expirado) → marcar sesión como
  caducada y pedir re-login; no crashear.
- **Refresh concurrente**: cubierto por el single-flight de §7 — sin él, la
  rotación del refresh_token mata la sesión bajo carga paralela.
- **Consent cancelado** (`?error=access_denied`): cubierto en §4 — error
  inmediato, no esperar timeout.
- **Nunca loguear** tokens (usar `mask_token`). Refresh + access en keyring, no
  en TOML.
- El navegador podría abrir en otra máquina (SSH): imprimir también la URL para
  copiar/pegar como fallback.

## 11. Fases de entrega

- [ ] **F0** — Verificar endpoints y nombres de scope reales (§12).
- [x] **F1** — `src/oauth.rs`: PKCE + authorize URL + tests unitarios del PKCE.
- [x] **F2** — Loopback listener + `wait_for_code` (test con request simulada).
- [x] **F3** — `exchange_code` + `refresh` contra `/oauth2/token`.
- [x] **F4** — `Credential` enum + migración en `secrets.rs`.
- [x] **F5** — `CfClient` refrescable: single-flight + reintento en 401 +
  refresco proactivo con margen + `Action::CredentialRefreshed`.
- [x] **F6** — `authenticate()` OAuth-aware (`/accounts`).
- [x] **F7** — Wiring en `app` + opción de UI "Login con Cloudflare" (Ctrl-L)
  + revoke al eliminar sesión OAuth.
- [ ] **F8** — Prueba E2E manual con la cuenta real; documentar en README
  (README ya documentado; falta la prueba E2E con cuenta real).

## 12. Pendiente de verificar

> **Verificado en pruebas (jul 2026):** los clientes *self-managed* NO aceptan
> los scopes estilo wrangler (`account:read` → `invalid_scope`). Usan IDs
> kebab-case con `.read`/`.edit` (p. ej. `account-settings.read`, `zone.read`,
> `workers-r2.read` — confirmados en la demo oficial `login-with-cloudflare`).
> Además el authorize solo acepta un **subconjunto de los scopes registrados
> en el cliente** (dashboard → *Manage Account → OAuth clients*). Nombres
> válidos completos: `GET /client/v4/oauth/scopes` (con API token). Override
> en runtime: `LAZYCF_OAUTH_SCOPES`.

- URLs exactas de authorize/token/revoke para clientes **self-managed**.
  Verificado: **no hay** discovery `.well-known/oauth-authorization-server` en
  `dash.cloudflare.com` ni `api.cloudflare.com`. Mejor fuente: constantes de
  wrangler en `workers-sdk` (`user.ts`), o probar el `client_id` propio directo
  contra `https://dash.cloudflare.com/oauth2/auth`.
- Nombre exacto del scope **DNS** (`dns_records:edit` en §2 es el nombre del
  permiso de API token; sin confirmar como scope OAuth) y de **Tunnel**
  (`cfd_tunnel`/`argo`) y **R2**:
  ```
  curl https://api.cloudflare.com/client/v4/oauth/scopes \
    -H "Authorization: Bearer <token>" | jq -r '.result[].name' \
    | grep -Ei 'tunnel|argo|r2|dns|cache'
  ```
- Valor de `expires_in` que devuelve el token endpoint (para el cálculo de
  `expires_at` y la política de refresh proactivo).

## 13. Pruebas

- Unit: `generate_pkce` (longitud, charset, challenge = S256 del verifier),
  parseo de la query del callback (incl. `?error=access_denied` y requests
  basura tipo `/favicon.ico`), validación de `state`.
- Unit: single-flight de refresh — N tasks concurrentes contra un mock 401 →
  exactamente **un** refresh ejecutado, el resto reutiliza el token nuevo.
- Integración: migración de keyring (lista de strings antigua → `Credential`)
  y roundtrip serde del enum tagged.
- Manual E2E: login OAuth → listar cuentas → probar DNS (write), Workers, D1,
  Queues; confirmar comportamiento de Tunnels/R2 según scopes; forzar expiración
  para validar el refresh + reintento en 401.
