//! Métodos de `App` del módulo r2 (extraídos de `app/mod.rs`).

use super::*;

impl App {
    pub(crate) fn change_bucket(&mut self, delta: i32) {
        if self.r2.select(delta)
            && let Some(name) = self.r2.selected_name()
        {
            self.load_bucket_info(name);
            self.r2.reset_browser();
            self.load_objects();
        }
    }

    /// Lista los objetos del bucket seleccionado bajo el prefijo actual.
    pub(crate) fn load_objects(&mut self) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let prefix = self.r2.prefix.clone();
        self.r2.begin_objects();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client
                .list_objects(&account_id, &bucket, &prefix, true, None)
                .await
            {
                Ok(list) => Action::R2ObjectsLoaded {
                    bucket,
                    prefix,
                    list,
                },
                Err(e) => Action::R2ObjectsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// ↓ en la última fila con cursor pendiente: trae la página siguiente.
    pub(crate) fn load_more_objects(&mut self) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let Some(cursor) = self.r2.next_cursor_cloned() else {
            return;
        };
        let prefix = self.r2.prefix.clone();
        self.r2.begin_objects(); // "cargando…" en el hint
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client
                .list_objects(&account_id, &bucket, &prefix, true, Some(&cursor))
                .await
            {
                Ok(list) => Action::R2MoreObjectsLoaded {
                    bucket,
                    prefix,
                    list,
                },
                // No usa R2ObjectsError: su render sustituiría el listado
                // entero. El cursor se conserva, ↓ reintenta.
                Err(e) => Action::ObjectStatus(format!("✗ No se pudo cargar más: {e}")),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn navigate_to(&mut self, prefix: String) {
        self.r2.clear_filter();
        self.r2.clear_marks();
        self.r2.prefix = prefix;
        self.load_objects();
    }

    /// Enter sobre una fila del navegador: carpeta → entrar; imagen → preview.
    /// En modo búsqueda, navega a la carpeta contenedora del resultado.
    pub(crate) fn open_entry(&mut self) {
        if self.r2.is_searching() {
            if let Some(Entry::File(o)) = self.r2.selected_entry().cloned() {
                let parent = o
                    .key
                    .rsplit_once('/')
                    .map(|(d, _)| format!("{d}/"))
                    .unwrap_or_default();
                if let Some(c) = self.search_cancel.take() {
                    c.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.r2.exit_search();
                self.navigate_to(parent);
            }
            return;
        }
        match self.r2.selected_entry().cloned() {
            Some(Entry::Up) => {
                let parent = self.r2.parent_prefix();
                self.navigate_to(parent);
            }
            Some(Entry::Folder(prefix)) => self.navigate_to(prefix),
            Some(Entry::File(o)) if o.is_image() => self.spawn_preview(),
            Some(Entry::File(_)) => {
                self.status = "d descargar · p URL prefirmada · v ver (imágenes)".into();
            }
            None => {}
        }
    }

    /// `s`: pide el término de la búsqueda profunda.
    pub(crate) fn open_search(&mut self) {
        if self.r2.selected_name().is_none() {
            return;
        }
        self.popup = Some(Popup::TextPrompt(TextPrompt::new(PromptKind::Search)));
    }

    /// Pagina TODO el bucket (sin delimiter) filtrando por subcadena, con tope
    /// de páginas. Emite progreso por página y el resultado final.
    pub(crate) fn start_deep_search(&mut self, term: String) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        // Cancela la búsqueda anterior si seguía en vuelo.
        if let Some(c) = self.search_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        let generation = self.r2.begin_search(term.clone());
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());
        self.status = format!("Buscando «{term}»…");
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            const MAX_PAGES: usize = 20; // ~10k objetos
            let needle = term.to_lowercase();
            let mut files = Vec::new();
            let mut cursor: Option<String> = None;
            let mut pages = 0usize;
            let mut capped = false;
            let mut error = None;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return; // cancelada: ni parciales
                }
                match client
                    .list_objects(&account_id, &bucket, "", false, cursor.as_deref())
                    .await
                {
                    Ok(list) => {
                        pages += 1;
                        files.extend(
                            list.files
                                .into_iter()
                                .filter(|o| o.key.to_lowercase().contains(&needle)),
                        );
                        let _ = tx.send(Action::SearchProgress {
                            bucket: bucket.clone(),
                            generation,
                            page: pages,
                            hits: files.len(),
                        });
                        cursor = list.cursor;
                        if cursor.is_none() {
                            break;
                        }
                        if pages >= MAX_PAGES {
                            capped = true;
                            break;
                        }
                    }
                    Err(e) => {
                        error = Some(e.to_string());
                        break;
                    }
                }
            }
            let _ = tx.send(Action::SearchResults {
                bucket,
                generation,
                files,
                pages,
                capped,
                error,
            });
        });
    }

    /// Esc/h en modo búsqueda: cancela y recarga el browse (prefijo intacto).
    pub(crate) fn exit_search(&mut self) {
        if let Some(c) = self.search_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.r2.exit_search();
        self.load_objects();
    }

    /// Espacio: marca/desmarca el archivo y avanza a la fila siguiente.
    pub(crate) fn toggle_mark(&mut self) {
        if self.r2.toggle_mark() {
            self.r2.select_entry(1);
        } else {
            self.status = "Espacio marca archivos (no carpetas)".into();
        }
    }

    pub(crate) fn open_upload(&mut self) {
        let Some(bucket) = self.r2.selected_name() else {
            return;
        };
        self.popup = Some(Popup::Upload(UploadForm {
            dest: format!("{bucket}/{}", self.r2.prefix),
            ..Default::default()
        }));
    }

    pub(crate) fn spawn_upload(&mut self, path: String) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let prefix = self.r2.prefix.clone();
        let tx = self.action_tx.clone();
        self.status = "Subiendo…".into();
        tokio::spawn(async move {
            let action = match tokio::fs::read(&path).await {
                Ok(body) => {
                    let filename = std::path::Path::new(&path)
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_else(|| "archivo".into());
                    let key = format!("{prefix}{filename}");
                    let ct = mime_guess::from_path(&path)
                        .first_or_octet_stream()
                        .essence_str()
                        .to_string();
                    match client
                        .put_object(&account_id, &bucket, &key, body, &ct)
                        .await
                    {
                        Ok(()) => Action::ObjectMutated(format!("Subido {filename}")),
                        Err(e) => Action::ObjectError(e.to_string()),
                    }
                }
                Err(e) => Action::ObjectError(format!("leyendo {path}: {e}")),
            };
            let _ = tx.send(action);
        });
    }

    /// Descarga el archivo seleccionado a ~/Descargas (o el directorio actual).
    pub(crate) fn spawn_download(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let key = file.key.clone();
        let filename = file.filename().to_string();
        let dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let tx = self.action_tx.clone();
        self.status = format!("Descargando {filename}…");
        tokio::spawn(async move {
            let action = match client.get_object(&account_id, &bucket, &key).await {
                Ok(bytes) => {
                    let dest = dir.join(&filename);
                    match tokio::fs::write(&dest, bytes).await {
                        Ok(()) => {
                            // Abre el archivo con la app por defecto (detached:
                            // no bloquea la TUI ni hereda su terminal).
                            match open::that_detached(&dest) {
                                Ok(()) => Action::ObjectStatus(format!(
                                    "Guardado y abierto: {}",
                                    dest.display()
                                )),
                                Err(e) => Action::ObjectStatus(format!(
                                    "Guardado en {} (no se pudo abrir: {e})",
                                    dest.display()
                                )),
                            }
                        }
                        Err(e) => Action::ObjectError(format!("escribiendo {}: {e}", dest.display())),
                    }
                }
                Err(e) => Action::ObjectError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn confirm_delete_object(&mut self) {
        // Con marcas activas, `x` borra el lote (gana sobre el cursor).
        let marked = self.r2.marked_keys();
        if !marked.is_empty() {
            let n = marked.len();
            self.popup = Some(Popup::Confirm(Confirm {
                title: "Borrar objetos".into(),
                body: format!("¿Borrar {n} objeto(s) marcados? No se puede deshacer."),
                on_yes: Action::DeleteObjects { keys: marked },
            }));
            return;
        }
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        let key = file.key.clone();
        let name = file.filename().to_string();
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar objeto".into(),
            body: format!("¿Borrar {name}?"),
            on_yes: Action::DeleteObject { key },
        }));
    }

    /// Borra las claves marcadas en secuencia; para al primer error pero
    /// siempre recarga (algo pudo borrarse ya).
    pub(crate) fn spawn_delete_objects(&mut self, keys: Vec<String>) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let total = keys.len();
        self.status = format!("Borrando {total} objeto(s)…");
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let mut ok = 0usize;
            let mut fail: Option<String> = None;
            for key in keys {
                match client.delete_object(&account_id, &bucket, &key).await {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        fail = Some(format!("'{key}': {e}"));
                        break;
                    }
                }
            }
            let msg = match fail {
                None => format!("{ok} objeto(s) borrados"),
                Some(err) => format!("Borrados {ok}/{total} · ✗ {err}"),
            };
            let _ = tx.send(Action::ObjectMutated(msg));
        });
    }

    pub(crate) fn spawn_delete_object(&mut self, key: String) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Borrando objeto…".into();
        tokio::spawn(async move {
            let action = match client.delete_object(&account_id, &bucket, &key).await {
                Ok(()) => Action::ObjectMutated("Objeto borrado".into()),
                Err(e) => Action::ObjectError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// `e`: abre el formulario de renombrar (pre-rellenado con el nombre actual).
    pub(crate) fn open_rename(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        self.popup = Some(Popup::Rename(RenameForm {
            old_key: file.key.clone(),
            name: TextInput::new(file.filename().to_string()),
            move_mode: false,
            error: None,
            submitting: false,
        }));
    }

    /// `m`: como renombrar pero editando la clave completa (cambia de carpeta).
    pub(crate) fn open_move(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        self.popup = Some(Popup::Rename(RenameForm {
            old_key: file.key.clone(),
            name: TextInput::new(file.key.clone()),
            move_mode: true,
            error: None,
            submitting: false,
        }));
    }

    /// `n` (en Objetos): pide el nombre de la carpeta nueva.
    pub(crate) fn open_new_folder(&mut self) {
        if self.r2.selected_name().is_none() {
            return;
        }
        self.popup = Some(Popup::TextPrompt(TextPrompt::new(PromptKind::NewFolder)));
    }

    /// Crea el prefijo subiendo un objeto marcador vacío (como el dashboard).
    pub(crate) fn spawn_create_folder(&mut self, name: String) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let key = format!("{}{name}/", self.r2.prefix);
        let tx = self.action_tx.clone();
        self.status = "Creando carpeta…".into();
        tokio::spawn(async move {
            let action = match client
                .put_object(&account_id, &bucket, &key, Vec::new(), "application/x-directory")
                .await
            {
                Ok(()) => Action::ObjectMutated(format!("Carpeta {name}/ creada")),
                Err(e) => Action::ObjectError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// Renombrar = descargar + subir con la nueva clave + borrar la vieja
    /// (el API de R2 no ofrece copia server-side por este endpoint).
    pub(crate) fn spawn_rename_object(&mut self, old_key: String, new_key: String, content_type: Option<String>) {
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Renombrando…".into();
        tokio::spawn(async move {
            let result: color_eyre::eyre::Result<()> = async {
                let bytes = client.get_object(&account_id, &bucket, &old_key).await?;
                let ct = content_type.unwrap_or_else(|| {
                    mime_guess::from_path(&new_key)
                        .first_or_octet_stream()
                        .essence_str()
                        .to_string()
                });
                client
                    .put_object(&account_id, &bucket, &new_key, bytes, &ct)
                    .await?;
                client.delete_object(&account_id, &bucket, &old_key).await?;
                Ok(())
            }
            .await;
            let action = match result {
                Ok(()) => Action::ObjectMutated(format!("Renombrado a {new_key}")),
                Err(e) => Action::ObjectError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// `p`: URL prefirmada. Si no hay credenciales R2 guardadas, las pide antes.
    pub(crate) fn open_presign(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        let key = file.key.clone();
        match secrets::load_r2_credentials() {
            Ok(Some(_)) => {
                self.popup = Some(Popup::Presign(PresignForm {
                    key,
                    expires: TextInput::new("3600"),
                    error: None,
                }));
            }
            _ => {
                self.pending_presign = Some(key);
                self.popup = Some(Popup::R2Creds(R2CredsForm::default()));
            }
        }
    }

    /// Abre `url` con la aplicación por defecto del sistema (detached: no
    /// bloquea la TUI ni hereda su terminal).
    pub(crate) fn open_url_in_browser(&mut self, url: String) {
        match open::that_detached(&url) {
            Ok(()) => self.status = format!("Abierto en el navegador: {url}"),
            Err(e) => self.status = format!("No se pudo abrir el navegador: {e}"),
        }
    }

    /// Candidatos de dominio del bucket actual para servir objetos: público
    /// r2.dev (si está habilitado) + personalizados habilitados.
    pub(crate) fn object_domain_choices(&self) -> Vec<DomainChoice> {
        let Some(info) = self.r2.info() else {
            return Vec::new();
        };
        let mut choices = Vec::new();
        if info.public.enabled && !info.public.domain.is_empty() {
            choices.push(DomainChoice {
                label: format!("Público (r2.dev): {}", info.public.domain),
                domain: info.public.domain.clone(),
            });
        }
        for d in info.domains.iter().filter(|d| d.enabled) {
            choices.push(DomainChoice {
                label: format!("Personalizado: {}", d.domain),
                domain: d.domain.clone(),
            });
        }
        choices
    }

    /// `o`: abre el objeto seleccionado con el dominio público/personalizado
    /// del bucket. Sin fricción si solo hay un candidato; si hay varios,
    /// deja elegir con un popup.
    pub(crate) fn open_object_browser(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        let key = file.key.clone();
        if self.r2.info().is_none() {
            self.status = "Info del bucket no disponible todavía".into();
            return;
        }
        let choices = self.object_domain_choices();
        match choices.len() {
            0 => {
                self.status = "Sin dominio público ni personalizado en este bucket".into();
            }
            1 => {
                let url = crate::api::r2::object_url(&choices[0].domain, &key);
                self.open_url_in_browser(url);
            }
            _ => {
                self.popup = Some(Popup::ChooseDomain(ChooseDomain::new(
                    key,
                    choices,
                    ChoosePurpose::Abrir,
                )));
            }
        }
    }

    /// `y`: copia la URL pública del objeto al portapapeles (OSC 52).
    pub(crate) fn copy_object_url(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        let key = file.key.clone();
        if self.r2.info().is_none() {
            self.status = "Info del bucket no disponible todavía".into();
            return;
        }
        let choices = self.object_domain_choices();
        match choices.len() {
            0 => {
                self.status = "Sin dominio público ni personalizado en este bucket".into();
            }
            1 => {
                let url = crate::api::r2::object_url(&choices[0].domain, &key);
                crate::tui::osc52_copy(&url);
                self.status = format!("URL copiada: {url}");
            }
            _ => {
                self.popup = Some(Popup::ChooseDomain(ChooseDomain::new(
                    key,
                    choices,
                    ChoosePurpose::Copiar,
                )));
            }
        }
    }

    /// `i`: metadatos del objeto seleccionado — todo sale del listado + info
    /// del bucket (el API Bearer no ofrece HEAD de objeto).
    pub(crate) fn show_object_info(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        let title = file.filename().to_string();
        let mut body = format!(
            "Clave: {}\nTamaño: {} ({} bytes)\nModificado: {}\nContent-Type: {}",
            file.key,
            crate::ui::widgets::human_size(file.size),
            file.size,
            file.last_modified,
            file.http_metadata
                .as_ref()
                .and_then(|m| m.content_type.as_deref())
                .unwrap_or("—"),
        );
        let key = file.key.clone();
        let urls: Vec<String> = self
            .object_domain_choices()
            .iter()
            .map(|c| crate::api::r2::object_url(&c.domain, &key))
            .collect();
        if !urls.is_empty() {
            body.push_str("\n\nURLs:");
            for u in urls {
                body.push_str(&format!("\n  {u}"));
            }
        }
        self.popup = Some(Popup::Message(Message {
            title,
            body,
            is_error: false,
        }));
    }

    /// `p` (en Buckets): habilita/deshabilita el dominio público r2.dev con
    /// aviso explícito — habilitarlo hace el bucket legible desde internet.
    pub(crate) fn confirm_toggle_public(&mut self) {
        let (Some(bucket), Some(info)) = (self.r2.selected_name(), self.r2.info()) else {
            self.status = "Info del bucket no disponible todavía".into();
            return;
        };
        if info.public.domain.is_empty() {
            self.status = "Este bucket no tiene dominio r2.dev asignado".into();
            return;
        }
        let enabled = !info.public.enabled;
        let (title, body) = if enabled {
            (
                "Habilitar acceso público".to_string(),
                format!(
                    "Esto hará el bucket '{bucket}' PÚBLICO en internet:\n  https://{}\nCualquiera con la URL podrá leer los objetos. ¿Continuar?",
                    info.public.domain
                ),
            )
        } else {
            (
                "Deshabilitar acceso público".to_string(),
                format!(
                    "https://{} dejará de servir los objetos de '{bucket}'. ¿Continuar?",
                    info.public.domain
                ),
            )
        };
        self.popup = Some(Popup::Confirm(Confirm {
            title,
            body,
            on_yes: Action::SetPublicDomain { bucket, enabled },
        }));
    }

    pub(crate) fn spawn_set_public_domain(&mut self, bucket: String, enabled: bool) {
        self.status = "Actualizando dominio público…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.set_public_domain(&account_id, &bucket, enabled).await {
                Ok(()) => Action::DomainsMutated(if enabled {
                    "Dominio r2.dev habilitado (bucket público)".into()
                } else {
                    "Dominio r2.dev deshabilitado".into()
                }),
                Err(e) => Action::DomainError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// `t` (en Buckets): popup con los dominios personalizados del bucket.
    pub(crate) fn open_bucket_domains(&mut self) {
        let (Some(bucket), Some(info)) = (self.r2.selected_name(), self.r2.info()) else {
            self.status = "Info del bucket no disponible todavía".into();
            return;
        };
        self.popup = Some(Popup::BucketDomains(BucketDomains::new(
            bucket,
            info.domains.clone(),
        )));
    }

    pub(crate) fn spawn_add_domain(&mut self, bucket: String, domain: String, zone_id: String) {
        self.status = format!("Conectando {domain}…");
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .add_custom_domain(&account_id, &bucket, &domain, &zone_id)
                .await
            {
                Ok(()) => Action::DomainsMutated(format!(
                    "Dominio {domain} conectado (el certificado tarda unos minutos)"
                )),
                Err(e) => Action::DomainError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_remove_domain(&mut self, bucket: String, domain: String) {
        self.status = format!("Desconectando {domain}…");
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .remove_custom_domain(&account_id, &bucket, &domain)
                .await
            {
                Ok(()) => Action::DomainsMutated(format!("Dominio {domain} desconectado")),
                Err(e) => Action::DomainError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// `c` (en Buckets): abre el editor de la política CORS del bucket actual.
    pub(crate) fn open_cors_edit(&mut self) {
        let (Some(bucket), Some(info)) = (self.r2.selected_name(), self.r2.info()) else {
            self.status = "Selecciona un bucket (con info cargada)".into();
            return;
        };
        let json = serde_json::to_string_pretty(&serde_json::Value::Array(info.cors_rules.clone()))
            .unwrap_or_else(|_| "[]".into());
        self.popup = Some(Popup::CorsEdit(CorsEditForm {
            bucket,
            json: TextInput::new(json),
            error: None,
            submitting: false,
        }));
    }

    pub(crate) fn spawn_save_cors(&mut self, bucket: String, rules: serde_json::Value) {
        self.status = "Guardando CORS…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.set_bucket_cors(&account_id, &bucket, rules).await {
                Ok(()) => Action::CorsMutated("CORS actualizado".into()),
                Err(e) => Action::CorsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// Cálculo local de la URL prefirmada (SigV4); la copia vía OSC 52.
    pub(crate) fn generate_presign(&mut self, key: String, expires: u64) {
        let (Some(account_id), Some(bucket)) = (
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        match secrets::load_r2_credentials() {
            Ok(Some((ak, sk))) => {
                let url = crate::api::r2::presign_get(
                    &account_id,
                    &ak,
                    &sk,
                    &bucket,
                    &key,
                    expires,
                    Utc::now(),
                );
                crate::tui::osc52_copy(&url);
                self.popup = Some(Popup::Message(Message {
                    title: "URL prefirmada".into(),
                    body: format!(
                        "{url}\n\nVálida {expires}s · copiada al portapapeles (OSC 52)."
                    ),
                    is_error: false,
                }));
            }
            _ => self.status = "No hay credenciales R2 guardadas".into(),
        }
    }

    /// Descarga y decodifica la imagen seleccionada para verla en el terminal.
    pub(crate) fn spawn_preview(&mut self) {
        let Some(file) = self.r2.selected_file() else {
            self.status = "Selecciona un archivo".into();
            return;
        };
        if !file.is_image() {
            self.status = "Solo se pueden previsualizar imágenes".into();
            return;
        }
        let (Some(client), Some(account_id), Some(bucket)) = (
            self.client(),
            self.active_account_id().map(String::from),
            self.r2.selected_name(),
        ) else {
            return;
        };
        let key = file.key.clone();
        let tx = self.action_tx.clone();
        self.status = "Cargando imagen…".into();
        tokio::spawn(async move {
            let result = match client.get_object(&account_id, &bucket, &key).await {
                Ok(bytes) => crate::components::r2::decode_image(&bytes, 100, 40),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(Action::ImageDecoded { key, result });
        });
    }

    pub(crate) fn open_new_bucket(&mut self) {
        self.popup = Some(Popup::TextPrompt(TextPrompt::new(PromptKind::NewBucket)));
    }

    pub(crate) fn confirm_delete_bucket(&mut self) {
        let Some(name) = self.r2.selected_name() else {
            return;
        };
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar bucket".into(),
            body: format!("¿Borrar el bucket {name}? Debe estar vacío."),
            on_yes: Action::DeleteBucket(name),
        }));
    }

    pub(crate) fn load_buckets(&mut self) {
        self.r2.loading = true;
        self.r2.error = None;
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.list_buckets(&account_id).await {
                Ok(buckets) => Action::R2BucketsLoaded(buckets),
                Err(e) => Action::R2Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// Carga detalle + uso + dominios del bucket en una sola tarea.
    pub(crate) fn load_bucket_info(&mut self, name: String) {
        self.r2.begin_info(name.clone());
        self.spawn_api(move |client, account_id, tx| async move {
            let info = match client.bucket_detail(&account_id, &name).await {
                Ok(detail) => {
                    let (usage, domains, public, cors) = tokio::join!(
                        client.bucket_usage(&account_id, &name),
                        client.bucket_domains(&account_id, &name),
                        client.bucket_public_domain(&account_id, &name),
                        client.bucket_cors(&account_id, &name),
                    );
                    Some(Box::new(BucketInfo {
                        detail,
                        usage: usage.unwrap_or_default(),
                        domains: domains.unwrap_or_default(),
                        public: public.unwrap_or_default(),
                        cors_rules: cors.unwrap_or_default(),
                    }))
                }
                Err(e) => {
                    tracing::debug!("detalle bucket {name}: {e}");
                    None
                }
            };
            let _ = tx.send(Action::R2InfoLoaded { bucket: name, info });
        });
    }

    pub(crate) fn spawn_create_bucket(&mut self, name: String) {
        self.status = "Creando bucket…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.create_bucket(&account_id, &name).await {
                Ok(()) => Action::R2Mutated(format!("Bucket '{name}' creado")),
                Err(e) => Action::R2Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_delete_bucket(&mut self, name: String) {
        self.status = "Borrando bucket…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.delete_bucket(&account_id, &name).await {
                Ok(()) => Action::R2Mutated(format!("Bucket '{name}' borrado")),
                Err(e) => Action::R2Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }
}
