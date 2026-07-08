```text
██╗      █████╗ ███████╗██╗   ██╗ ██████╗███████╗
██║     ██╔══██╗╚══███╔╝╚██╗ ██╔╝██╔════╝██╔════╝
██║     ███████║  ███╔╝  ╚████╔╝ ██║     █████╗
██║     ██╔══██║ ███╔╝    ╚██╔╝  ██║     ██╔══╝
███████╗██║  ██║███████╗   ██║   ╚██████╗██║
╚══════╝╚═╝  ╚═╝╚══════╝   ╚═╝    ╚═════╝╚═╝
```

# lazycf

TUI estilo **lazygit**, escrita en Rust + [ratatui](https://ratatui.rs), para administrar
[Cloudflare](https://cloudflare.com) desde la terminal sin abrir el dashboard web.

Módulos: **DNS/Dominios**, **Túneles** (Zero Trust), **Workers**, **Queues**, **D1**, **R2**.

Navegación estilo lazygit (paneles, atajos de teclado, sidebar de recursos), llamadas a la API
de Cloudflare 100% async (no bloquea la UI), y soporte multi-cuenta con selector de cuenta activa.

## Módulos

### 🌐 DNS y Dominios
- Zonas de la cuenta activa (izquierda) + tabla de registros de la zona seleccionada (derecha).
- Crear y editar registros con **formulario dinámico por tipo** (A, AAAA, CNAME, TXT, MX):
  los campos cambian según el tipo (prioridad en MX, proxy solo en proxiables, TTL con `auto`).
- **Toggle de proxy** (nube naranja) con confirmación, borrado de registros con confirmación.
- **Purga de caché** de la zona completa (con confirmación).

### 🚇 Túneles (Zero Trust)
- Lista de túneles `cloudflared` con **estado en vivo** (healthy / degraded / down / inactive)
  y conexiones activas por datacenter.
- Crear túnel (muestra el token para el connector) y borrarlo; limpiar conexiones colgadas.
- **Rutas públicas (ingress)** estilo dashboard: "agregar aplicación publicada" con
  subdominio + zona de la cuenta — **crea el CNAME automáticamente**. Editar servicio/ruta
  y borrar rutas sin romper el resto del config.

### ⚙ Workers
- Lista de scripts + detalle con **5 pestañas**:
  - **Métricas** — requests, errores, CPU p50/p99 y sparkline 24 h (GraphQL), tasa de error coloreada.
  - **Implementaciones** — historial de deployments; **rollback** al deployment seleccionado (con confirmación).
  - **Variables** — variables y secretos; editar valores y añadir secretos nuevos (endpoint seguro).
  - **Logs** — **live-tail por WebSocket** (protocolo `trace-v1`): filtro en vivo (`/`),
    solo-errores (`E`), seguir el final (`End`), Enter abre el **detalle del evento**
    (request, headers, logs, excepciones) y `y` copia el JSON crudo.
  - **Rutas** — rutas de Worker por zona + custom domains apuntando al script.
- **Probar una ruta** (`t`): GET con código de estado y latencia; sugiere la URL `workers.dev`.

### 📨 Queues
- Lista de colas (⏸ marca las pausadas) + detalle con **3 pestañas**:
  - **Resumen** — settings (delay, retención, estado de entrega), producers y consumers.
  - **Consumers** — configuración completa; **editar** batch, retries, delay, DLQ,
    concurrencia/wait (worker) o visibility timeout (HTTP pull).
  - **Métricas** — backlog actual + sparklines de backlog y mensajes ingeridos (24 h, GraphQL).
- Crear y borrar colas; **enviar mensajes** (texto o JSON, con delay opcional);
  **pausar/reanudar** la entrega y **purgar** mensajes (todo con confirmación).
- **Peek de mensajes** en colas HTTP pull (`m`, sin ack — reaparecen tras el visibility timeout).
- `l` salta al módulo Workers con el **live-tail del consumer** ya arrancado.

### 🗄 D1
- Bases y tablas (vía `sqlite_master`); ↑↓ sobre una tabla muestra sus columnas (PRAGMA).
- **Editor SQL multilínea** con **autocompletado contextual** (keywords, tablas, columnas,
  y columnas por alias — `t.` sugiere las de esa tabla/subquery). F5 / Ctrl+Enter ejecuta.
- **LIMIT automático** en consultas sin límite propio + tope de 2 000 filas con aviso de
  truncado — la rejilla vuela aunque la tabla tenga 100 000 registros.
- Resultados en **rejilla estilo hoja de cálculo**: navegación por celda, ver el valor
  completo (Enter), copiar celda (`y`) o fila TSV (`Y`).
- Barra **WHERE** con autocompletado (columnas del resultado): filtra la tabla actual
  **o la última consulta libre** (se envuelve como subquery).

### 📦 R2
- Buckets (crear/borrar) + panel de **uso** (peso, nº de objetos, ubicación, clase).
- **Navegador de objetos** por carpetas con paginación, filtro instantáneo (`/`) y
  **búsqueda profunda** en todo el bucket (`s`).
- Subir, descargar (a ~/Descargas), renombrar, **mover** (editando la clave), nueva carpeta,
  metadatos (`i`), marcar múltiples (`Espacio`) y **borrado masivo** (con confirmación).
- **Preview de imágenes en la terminal** (`v`, medias celdas RGB).
- URLs: abrir/copiar con dominio público `r2.dev` o **dominios personalizados**
  (conectar/quitar desde la TUI), y **URLs prefirmadas SigV4** (credenciales S3 en el keyring).
- Editor de **política CORS** (JSON) y toggle del dominio público `r2.dev`.

### Transversal
- **Multi-cuenta y multi-token**: selector de cuenta activa (`A`), tokens en el keyring del sistema.
- **Mouse completo**: click enfoca y selecciona, scroll navega cualquier panel.
- Ayuda contextual (`?`) con los atajos del panel activo; barra de estado con hints.
- Render bajo demanda: CPU ≈ 0 en reposo.

## Capturas

### 🌐 DNS y Dominios
Zonas y registros: crear/editar con formulario dinámico por tipo, toggle de proxy
(nube naranja) con confirmación, borrado y purga de caché.

![Módulo DNS](images/dns.png)

### ⚙ Workers
Detalle con pestañas: métricas 24h con sparkline (GraphQL), implementaciones,
variables/secretos (editables) y logs en vivo (live-tail por WebSocket).

![Módulo Workers](images/workers.png)

### 🗄 D1
Cliente SQL: bases y tablas a la izquierda, editor SQL (F5 / Ctrl+Enter ejecuta)
y resultados como tabla con scroll a la derecha.

![Módulo D1](images/d1.png)

### 📦 R2
Explorador de objetos: buckets + info de uso (peso, nº de objetos), navegación por
carpetas, subida/descarga, URLs prefirmadas (SigV4) y preview de imágenes en terminal.

![Módulo R2](images/r2.png)

## Requisitos

- Rust (edition 2024) y Cargo.
- **Un API Token de Cloudflare** — la app no funciona sin él. Se autentica contra
  `Authorization: Bearer <token>`, nunca con la Global Key.

Cómo obtenerlo: dashboard de Cloudflare → *My Profile → API Tokens → Create Token*
→ *Create Custom Token*.

### Scopes (permisos) necesarios

Cada módulo de lazycf mapea a un permiso del token. Agrega solo los que uses;
si falta uno, ese módulo devuelve `403` pero el resto sigue funcionando.

#### A nivel de cuenta (*Account*)

| Scope | Por qué |
| --- | --- |
| **Account Settings** · Read | Listar tus cuentas y verificar el token (`/accounts`, `/accounts/{id}/tokens/verify`) para el selector de cuenta activa. |
| **Workers Scripts** · Edit | Módulo Workers: listar scripts, deployments, subdominio, dominios, ver/editar variables y secretos, rollback de deployments. |
| **Workers Tail** · Read | Logs en vivo de Workers (live-tail por WebSocket). |
| **D1** · Edit | Módulo D1: listar bases, y ejecutar SQL (incluye escrituras) en el editor. |
| **Workers R2 Storage** · Edit | Módulo R2: buckets, uso, objetos (subir/descargar/borrar/renombrar), CORS y dominios. |
| **Queues** · Edit | Módulo Queues: listar/crear/borrar colas, consumidores, publicar y purgar mensajes. |
| **Cloudflare Tunnel** · Edit | Módulo Túneles (Zero Trust): listar, crear, editar ingress, limpiar conexiones y borrar túneles. |
| **Account Analytics** · Read | Métricas 24h de Workers y Queues (sparklines) vía GraphQL. |

#### A nivel de zona (*Zone*)

| Scope | Por qué |
| --- | --- |
| **Zone** · Read | Módulo DNS: listar tus zonas/dominios (`/zones`). |
| **DNS** · Edit | Módulo DNS: listar, crear, editar, borrar registros y toggle de proxy. |
| **Workers Routes** · Read | Pestaña Rutas del módulo Workers: rutas de Worker por zona. |
| **Cache Purge** · Purge | Purga de caché de una zona (`/zones/{id}/purge_cache`). |

> **R2 (URLs prefirmadas / SigV4):** el explorador de objetos usa la API de Cloudflare
> (token Bearer), pero las URLs prefirmadas requieren además credenciales **S3 de R2**
> (Access Key ID + Secret) generadas aparte en *R2 → Manage R2 API Tokens*.


```sh
export CLOUDFLARE_API_TOKEN="<tu-token>"
```

## Uso

```sh
cargo run
```

```sh
cargo run -- --log lazycf.log   # escribe logs a archivo (por defecto no loggea)
```

