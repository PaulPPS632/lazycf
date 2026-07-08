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
| **Account Analytics** · Read | Métricas 24h de Workers (sparkline) vía GraphQL. |

#### A nivel de zona (*Zone*)

| Scope | Por qué |
| --- | --- |
| **Zone** · Read | Módulo DNS: listar tus zonas/dominios (`/zones`). |
| **DNS** · Edit | Módulo DNS: listar, crear, editar, borrar registros y toggle de proxy. |
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

