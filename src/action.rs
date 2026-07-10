//! Intenciones de alto nivel. Los componentes y las tareas async (resultados
//! de red) emiten `Action`; `app.rs` las enruta en `dispatch`.

use crate::api::r2::ObjectList;
use crate::components::r2::BucketInfo;
use crate::model::{
    Account, Binding, D1Database, Deployment, DnsRecord, IngressRule, PulledMessage,
    QueryOutcome, Queue, QueueConsumer, QueueMetrics, R2Bucket, R2Object, Tunnel, WorkerMetrics,
    WorkerScript, Zone,
};

#[derive(Debug, Clone)]
pub enum Action {
    /// Salir de la aplicación.
    Quit,
    /// Mover el foco al siguiente panel (Tab) o al anterior (`back = true`).
    CycleFocus { back: bool },

    // --- Auth ---
    /// El usuario envió un token para verificar.
    SubmitToken(String),
    /// Verificación OK: credencial válida + cuentas visibles.
    CredentialVerified {
        credential: crate::secrets::Credential,
        accounts: Vec<Account>,
        /// Viene de `CLOUDFLARE_API_TOKEN`: no se persiste en el keyring.
        from_env: bool,
    },
    /// Fallo de autenticación (token inválido, red, keyring, etc.).
    AuthFailed(String),
    /// Abrir en el navegador la página de creación de API tokens.
    OpenTokenPage,
    /// Abrir el modal de ayuda con todos los atajos.
    OpenHelp,

    // --- Auth: OAuth ---
    /// Iniciar el flujo de login OAuth (abre navegador + listener local).
    StartOAuthLogin,
    /// Cancelar el flujo OAuth en vuelo (Esc en el popup).
    CancelOAuthLogin,
    /// URL de autorización lista (fallback para copiar/pegar, p. ej. SSH).
    OAuthUrl(String),
    /// El flujo OAuth terminó con tokens: verificar y crear la sesión.
    OAuthCompleted(crate::oauth::OAuthTokens),
    /// El flujo OAuth falló (denegado, timeout, red…).
    OAuthFailed(String),
    /// El client refrescó una credencial OAuth: persistir la lista actualizada.
    CredentialRefreshed,

    // --- Cuentas / sesiones (multi-token) ---
    /// Abrir el selector de cuenta.
    OpenAccountPicker,
    /// Cambiar a la cuenta `account` de la sesión (token) `session`.
    SwitchTo { session: usize, account: usize },
    /// Eliminar el token de la sesión `session` (tras confirmación).
    DeleteToken(usize),

    // --- DNS ---
    /// Zonas cargadas.
    ZonesLoaded(Vec<Zone>),
    /// Registros cargados para una zona concreta.
    RecordsLoaded {
        zone_id: String,
        records: Vec<DnsRecord>,
    },
    /// Alternar el proxy del registro seleccionado (barra espaciadora).
    ToggleProxy,
    /// Confirmar borrado de un registro.
    DeleteRecord {
        zone_id: String,
        record_id: String,
    },
    /// Crear o editar un registro (desde el formulario).
    SubmitRecord {
        zone_id: String,
        editing_id: Option<String>,
        rtype: String,
        name: String,
        content: String,
        ttl: String,
        proxied: bool,
        priority: String,
    },
    /// Confirmar purga de caché de una zona.
    PurgeCache {
        zone_id: String,
    },
    /// Error en una operación DNS.
    DnsError(String),
    /// Mutación DNS OK: fija estado y recarga los registros de la zona.
    DnsMutated(String),
    /// Mensaje de estado sin recarga (p. ej. purga de caché).
    DnsStatus(String),

    // --- Túneles ---
    /// Túneles cargados.
    TunnelsLoaded(Vec<Tunnel>),
    /// Ingress cargado para un túnel concreto.
    IngressLoaded {
        tunnel_id: String,
        rules: Vec<IngressRule>,
    },
    /// Crear un túnel nuevo con este nombre.
    CreateTunnel(String),
    /// Túnel creado: muestra el token del conector y recarga.
    TunnelCreated {
        name: String,
        token: String,
    },
    /// Limpiar las conexiones de un túnel.
    CleanupConnections {
        tunnel_id: String,
    },
    /// Borrar un túnel.
    DeleteTunnel {
        tunnel_id: String,
    },
    /// Añadir una ruta pública (regla de ingress) a un túnel; opcionalmente crea
    /// el CNAME proxied en la zona `dns_zone` (`None` = no crear DNS).
    AddTunnelRoute {
        tunnel_id: String,
        hostname: String,
        service: String,
        path: String,
        dns_zone: Option<String>,
    },
    /// Editar una ruta existente (servicio/ruta; el hostname no cambia).
    EditTunnelRoute {
        tunnel_id: String,
        hostname: String,
        service: String,
        path: String,
    },
    /// Borrar una ruta (regla de ingress) por hostname; NO borra el CNAME.
    DeleteTunnelRoute {
        tunnel_id: String,
        hostname: String,
    },
    /// Mutación de ruta OK: fija estado, cierra el form y recarga las rutas del
    /// túnel actual (sin recargar toda la lista, que perdería la selección).
    TunnelRouteMutated(String),
    /// Error al añadir/editar una ruta (mantiene el formulario abierto).
    TunnelRouteError(String),
    /// Error en una operación de túneles.
    TunnelError(String),
    /// Mutación de túnel OK: fija estado y recarga la lista.
    TunnelMutated(String),

    // --- Workers ---
    /// Scripts cargados.
    WorkersLoaded(Vec<WorkerScript>),
    /// Subdominio `*.workers.dev` de la cuenta.
    SubdomainLoaded(Option<String>),
    /// Métricas cargadas para un script concreto (`None` = no disponibles).
    MetricsLoaded {
        script: String,
        metrics: Option<WorkerMetrics>,
    },
    /// Implementaciones cargadas (`None` = error).
    DeploymentsLoaded {
        script: String,
        deployments: Option<Vec<Deployment>>,
    },
    /// Bindings (vars/secretos) cargados (`None` = error).
    BindingsLoaded {
        script: String,
        bindings: Option<Vec<Binding>>,
    },
    /// Error en una operación de Workers.
    WorkersError(String),
    /// Lanzar una prueba HTTP GET a esta URL.
    HttpProbe(String),
    /// Resultado de la prueba HTTP.
    HttpResult {
        status: Option<u16>,
        millis: u128,
        info: String,
    },

    // --- Workers: live-tail (Fase 7) ---
    /// Iniciar el tail de logs de un script.
    StartTail(String),
    /// Detener el tail activo.
    StopTail,
    /// El WebSocket de tail se conectó.
    TailStarted { script: String },
    /// Un evento de tail recibido por el WebSocket.
    TailPush {
        script: String,
        event: crate::api::workers::TailEvent,
    },
    /// Error en el tail (creación/WS).
    TailError { script: String, msg: String },
    /// El tail terminó (parado, cerrado o expirado).
    TailEnded { script: String },

    // --- Workers: rutas / rollback ---
    /// Rutas de zona + custom domains del worker (`None` = error).
    RoutingLoaded {
        script: String,
        routing: Option<crate::components::workers::RoutingInfo>,
    },
    /// Revertir a un deployment previo (re-desplegar sus versiones).
    RollbackDeployment {
        script: String,
        versions: Vec<crate::model::DeployVersion>,
    },
    /// Rollback OK: fija estado y recarga las implementaciones.
    DeploymentRolledBack { script: String, msg: String },
    /// Error al revertir.
    RollbackError(String),

    // --- Queues (Fase 4) ---
    /// Colas cargadas.
    QueuesLoaded(Vec<Queue>),
    /// Error en una operación de Queues.
    QueueError(String),
    /// Mutación de cola OK: fija estado y recarga la lista.
    QueueMutated(String),
    /// Crear una cola con este nombre.
    CreateQueue(String),
    /// Borrar una cola (tras confirmación).
    DeleteQueue { queue_id: String },
    /// Pausar/reanudar la entrega (settings.delivery_paused). `queue_name`
    /// viaja para el body del PATCH (algunas versiones del API lo exigen).
    PauseQueue {
        queue_id: String,
        queue_name: String,
        paused: bool,
    },
    /// Purgar la cola (borra TODOS los mensajes; tras confirmación).
    PurgeQueue { queue_id: String },
    /// Publicar un mensaje (desde el formulario).
    SendMessage {
        queue_id: String,
        body: String,
        content_type: String,
        delay_seconds: Option<u64>,
    },
    /// Mensaje publicado OK: estado + cierra el formulario.
    MessageSent(String),
    /// Error al publicar (mantiene el formulario abierto).
    SendMessageError(String),
    /// Consumers cargados (`None` = error).
    ConsumersLoaded {
        queue_id: String,
        consumers: Option<Vec<QueueConsumer>>,
    },
    /// Guardar la edición de un consumer (PUT; body con el shape del GET).
    UpdateConsumer {
        queue_id: String,
        consumer_id: String,
        body: serde_json::Value,
    },
    /// Consumer guardado: estado, cierra el form y recarga la pestaña.
    ConsumerSaved { queue_id: String, msg: String },
    /// Error al guardar el consumer (mantiene el formulario abierto).
    ConsumerError(String),
    /// Métricas de la cola (`None` = no disponibles).
    QueueMetricsLoaded {
        queue_id: String,
        metrics: Option<QueueMetrics>,
    },
    /// Resultado del peek (pull sin ack). `Err` = mensaje legible del API
    /// (p. ej. cola con consumer worker).
    MessagesPulled {
        queue_id: String,
        outcome: Result<Vec<PulledMessage>, String>,
    },

    // --- Workers: variables / secretos ---
    /// Guardar una variable (plain_text) o secreto (secret_text).
    SaveBinding {
        script: String,
        name: String,
        is_secret: bool,
        value: String,
        adding: bool,
    },
    /// Binding guardado: fija estado y recarga la pestaña de variables.
    BindingSaved { script: String, msg: String },
    /// Error al guardar un binding (mantiene el formulario abierto).
    BindingError(String),

    // --- D1 (Fase 5) ---
    /// Bases de datos D1 cargadas.
    D1DatabasesLoaded(Vec<D1Database>),
    /// Tablas de una base concreta cargadas, con su esquema (tabla → columnas)
    /// para el autocompletado del editor SQL.
    D1TablesLoaded {
        db_id: String,
        tables: Vec<String>,
        schema: std::collections::HashMap<String, Vec<String>>,
    },
    /// Error al listar tablas (se muestra en el panel de tablas).
    D1TablesError(String),
    /// Resultado de una consulta (título + tabla o error).
    D1ResultLoaded {
        db_id: String,
        title: String,
        outcome: Result<QueryOutcome, String>,
    },
    /// Error al listar bases D1.
    D1Error(String),

    // --- R2 (Fase 6) ---
    /// Buckets cargados.
    R2BucketsLoaded(Vec<R2Bucket>),
    /// Detalle + uso + dominios de un bucket (`None` = error).
    R2InfoLoaded {
        bucket: String,
        info: Option<Box<BucketInfo>>,
    },
    /// Crear un bucket con este nombre.
    CreateBucket(String),
    /// Borrar un bucket.
    DeleteBucket(String),
    /// Mutación de bucket OK: fija estado y recarga la lista.
    R2Mutated(String),
    /// Error en una operación de R2.
    R2Error(String),

    // --- R2: objetos ---
    /// Listado de objetos (carpetas + archivos) para un bucket/prefijo.
    R2ObjectsLoaded {
        bucket: String,
        prefix: String,
        list: ObjectList,
    },
    /// Error listando objetos.
    R2ObjectsError(String),
    /// Subir el archivo local `path` al prefijo actual.
    UploadObject { path: String },
    /// Borrar el objeto `key` (tras confirmación).
    DeleteObject { key: String },
    /// Renombrar (copiar + borrar) un objeto dentro de la misma carpeta.
    RenameObject {
        old_key: String,
        new_key: String,
        content_type: Option<String>,
    },
    /// Página siguiente del listado actual (se añade al final, no reemplaza).
    R2MoreObjectsLoaded {
        bucket: String,
        prefix: String,
        list: ObjectList,
    },
    /// Lanzar la búsqueda profunda del término en todo el bucket (paginada).
    SearchObjects { term: String },
    /// Progreso de la búsqueda (una página recorrida). `generation` descarta
    /// respuestas de búsquedas obsoletas.
    SearchProgress {
        bucket: String,
        generation: u64,
        page: usize,
        hits: usize,
    },
    /// Resultado final de la búsqueda (parcial si `error` es `Some`).
    SearchResults {
        bucket: String,
        generation: u64,
        files: Vec<R2Object>,
        pages: usize,
        capped: bool,
        error: Option<String>,
    },
    /// Borrar las claves marcadas (tras confirmación). Secuencial; para al
    /// primer error pero siempre recarga el listado.
    DeleteObjects { keys: Vec<String> },
    /// Crear el marcador de carpeta `prefijo + nombre + "/"`.
    CreateFolder { name: String },
    /// Mutación de objeto OK: fija estado y recarga el listado actual.
    ObjectMutated(String),
    /// Descarga completada (ruta local) o estado de objeto sin recarga.
    ObjectStatus(String),
    /// Error en una operación de objeto (se muestra en form si hay uno).
    ObjectError(String),
    /// Guardar credenciales R2 (Access Key + Secret) para URLs prefirmadas.
    SaveR2Creds { access_key: String, secret: String },
    /// Generar la URL prefirmada de `key` (cálculo local con las credenciales R2).
    GeneratePresign { key: String, expires: u64 },
    /// Imagen descargada y decodificada (o error) para previsualizar.
    ImageDecoded {
        key: String,
        result: Result<(u32, u32, Vec<u8>), String>,
    },
    /// Guardar la política CORS de un bucket (JSON ya validado).
    SaveCors {
        bucket: String,
        rules: serde_json::Value,
    },
    /// CORS guardado OK: fija estado, cierra el popup y recarga solo ese bucket.
    CorsMutated(String),
    /// Error al guardar CORS (mantiene el popup abierto para corregir).
    CorsError(String),

    // --- R2: dominios ---
    /// Habilitar/deshabilitar el dominio público r2.dev del bucket.
    SetPublicDomain { bucket: String, enabled: bool },
    /// Conectar un dominio personalizado (POST domains/custom, enabled=true).
    AddCustomDomain {
        bucket: String,
        domain: String,
        zone_id: String,
    },
    /// Desconectar un dominio personalizado del bucket.
    RemoveCustomDomain { bucket: String, domain: String },
    /// Mutación de dominios OK: estado, cierra popups de dominios y recarga info.
    DomainsMutated(String),
    /// Error de dominios (mantiene el form de añadir abierto si procede).
    DomainError(String),
}
