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

Cómo obtenerlo: dashboard de Cloudflare → *My Profile → API Tokens → Create Token*.
Token scoped recomendado: **Account** (Workers, D1, Queues, Cloudflare Tunnel, Analytics) +
**Zone** (DNS, Cache Purge, Zone Read, Analytics). R2 (objetos) requiere un token R2 aparte.


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

