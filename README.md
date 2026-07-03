```
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

## Requisitos

- Rust (edition 2024) y Cargo.
- **Un API Token de Cloudflare** — la app no funciona sin él. Se autentica contra
  `Authorization: Bearer <token>`, nunca con la Global Key.

Cómo obtenerlo: dashboard de Cloudflare → *My Profile → API Tokens → Create Token*.
Token scoped recomendado: **Account** (Workers, D1, Queues, Cloudflare Tunnel, Analytics) +
**Zone** (DNS, Cache Purge, Zone Read, Analytics). R2 (objetos) requiere un token R2 aparte.

El token se guarda en el **keyring del OS**, nunca en texto plano. Como alternativa (útil en
CI/headless) se puede exportar como variable de entorno:

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

