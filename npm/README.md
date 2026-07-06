# lazycf

TUI estilo **lazygit**, escrita en Rust + [ratatui](https://ratatui.rs), para administrar
[Cloudflare](https://cloudflare.com) desde la terminal sin abrir el dashboard web.

Módulos: **DNS/Dominios**, **Túneles** (Zero Trust), **Workers**, **Queues**, **D1**, **R2**.

## Instalación

```sh
npm install -g lazycf
```

El instalador descarga el binario nativo para tu sistema (Linux x64, macOS Intel/Apple Silicon,
Windows x64) desde las [releases de GitHub](https://github.com/PaulPPS632/lazycf/releases).

## Uso

```sh
lazycf
```

Necesitas un **API Token de Cloudflare** (nunca la Global Key): dashboard → *My Profile →
API Tokens → Create Token*. Permisos recomendados: **Account** (Workers, D1, Queues,
Cloudflare Tunnel, Analytics) + **Zone** (DNS, Cache Purge, Zone Read, Analytics).
El token se guarda en el keyring del sistema, nunca en texto plano.

## Capturas

### 🌐 DNS y Dominios
![Módulo DNS](https://raw.githubusercontent.com/PaulPPS632/lazycf/main/images/dns.png)

### ⚙ Workers (métricas, deploys, variables, logs en vivo)
![Módulo Workers](https://raw.githubusercontent.com/PaulPPS632/lazycf/main/images/workers.png)

### 🗄 D1 (cliente SQL en la terminal)
![Módulo D1](https://raw.githubusercontent.com/PaulPPS632/lazycf/main/images/d1.png)

### 📦 R2 (explorador de objetos, URLs prefirmadas, preview de imágenes)
![Módulo R2](https://raw.githubusercontent.com/PaulPPS632/lazycf/main/images/r2.png)

## Repositorio

Código, issues y documentación completa: [github.com/PaulPPS632/lazycf](https://github.com/PaulPPS632/lazycf)

## Licencia

GPL-3.0-only
