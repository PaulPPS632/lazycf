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
API Tokens → Create Token → Create Custom Token*.
El token se guarda en el keyring del sistema, nunca en texto plano.

## Scopes (permisos) del token

Cada módulo de lazycf mapea a un permiso del token. Agrega solo los que uses;
si falta uno, ese módulo devuelve `403` pero el resto sigue funcionando.

### A nivel de cuenta (*Account*)

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

### A nivel de zona (*Zone*)

| Scope | Por qué |
| --- | --- |
| **Zone** · Read | Módulo DNS: listar tus zonas/dominios (`/zones`). |
| **DNS** · Edit | Módulo DNS: listar, crear, editar, borrar registros y toggle de proxy. |
| **Workers Routes** · Read | Pestaña Rutas del módulo Workers: rutas de Worker por zona. |
| **Cache Purge** · Purge | Purga de caché de una zona (`/zones/{id}/purge_cache`). |

> **R2 (URLs prefirmadas / SigV4):** el explorador de objetos usa la API de Cloudflare
> (token Bearer), pero las URLs prefirmadas requieren además credenciales **S3 de R2**
> (Access Key ID + Secret) generadas aparte en *R2 → Manage R2 API Tokens*.

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

MIT
