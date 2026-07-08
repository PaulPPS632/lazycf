//! Métodos de `App` del módulo workers (extraídos de `app/mod.rs`).

use super::*;

impl App {
    pub(crate) fn change_worker(&mut self, delta: i32) {
        if self.workers.select(delta) {
            // Cambiar de worker detiene el tail y limpia sus logs.
            self.stop_tail();
            self.workers.clear_logs();
            self.workers.reset_tabs();
            self.load_active_tab();
        }
    }

    /// ↑↓ en la columna 3: navega el contenido de la pestaña activa
    /// (implementación / binding / log). Métricas y Rutas no navegan.
    pub(crate) fn workers_detail_nav(&mut self, delta: i32) {
        match self.workers.active_tab {
            1 => {
                self.workers.select_deploy(delta);
            }
            2 => {
                self.workers.select_binding(delta);
            }
            3 => {
                self.workers.log_scroll(delta);
            }
            _ => {}
        }
    }

    /// Enter en Workers: revertir (Implementaciones) o ver detalle (Logs).
    pub(crate) fn workers_enter(&mut self) {
        match self.workers.active_tab {
            1 => self.confirm_rollback(),
            3 => self.open_log_detail(),
            _ => {}
        }
    }

    /// Enter en la pestaña Logs: abre el popup de detalle del evento (si tiene).
    pub(crate) fn open_log_detail(&mut self) {
        let Some(ev) = self.workers.selected_log_event() else {
            return;
        };
        if ev.detail.is_empty() {
            return; // evento sintético (conectado/finalizado): nada que expandir
        }
        self.popup = Some(Popup::LogDetail(LogDetail {
            title: ev.summary.clone(),
            lines: ev.detail.clone(),
            raw: ev.raw.clone(),
            scroll: 0,
        }));
    }

    /// `y` en la pestaña Logs: copia el JSON crudo del evento seleccionado.
    pub(crate) fn copy_log_event(&mut self) {
        match self.workers.selected_log_event() {
            Some(ev) if !ev.raw.is_empty() => {
                crate::tui::osc52_copy(&ev.raw);
                self.status = "Evento copiado al portapapeles".into();
            }
            _ => self.status = "Nada que copiar".into(),
        }
    }

    /// Enter en Implementaciones: revierte al deployment seleccionado (Confirm).
    pub(crate) fn confirm_rollback(&mut self) {
        let Some(script) = self.workers.selected_name() else {
            return;
        };
        let Some(idx) = self.workers.selected_deploy_index() else {
            return;
        };
        if idx == 0 {
            self.status = "Ese despliegue ya es el activo".into();
            return;
        }
        let Some(dep) = self.workers.selected_deploy() else {
            return;
        };
        if dep.versions.is_empty() {
            self.status = "El despliegue no tiene versiones para revertir".into();
            return;
        }
        let versions = dep.versions.clone();
        let fecha = crate::ui::widgets::short_date(&dep.created_on, 16);
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Revertir despliegue".into(),
            body: format!(
                "¿Revertir al despliegue de {fecha}?\nSe volverá a desplegar esa(s) versión(es)."
            ),
            on_yes: Action::RollbackDeployment { script, versions },
        }));
    }

    pub(crate) fn open_edit_binding(&mut self) {
        let Some(script) = self.workers.selected_name() else {
            return;
        };
        let Some(b) = self.workers.selected_binding() else {
            return;
        };
        if !(b.btype == "plain_text" || b.btype == "secret_text") {
            self.status = "Solo se pueden editar variables y secretos".into();
            return;
        }
        self.popup = Some(Popup::BindingEdit(BindingEdit::edit(script, b)));
    }

    pub(crate) fn open_add_secret(&mut self) {
        let Some(script) = self.workers.selected_name() else {
            return;
        };
        self.popup = Some(Popup::BindingEdit(BindingEdit::add_secret(script)));
    }

    /// `l`: inicia el live-tail si no hay uno; si ya está activo, lo detiene.
    pub(crate) fn toggle_tail(&mut self) {
        if self.workers.tailing {
            self.dispatch(Action::StopTail);
        } else if let Some(script) = self.workers.selected_name() {
            self.dispatch(Action::StartTail(script));
        }
    }

    /// Señala el cierre del tail activo (el task borra la sesión al terminar).
    pub(crate) fn stop_tail(&mut self) {
        if let Some(tx) = self.tail_stop.take() {
            let _ = tx.send(());
        }
        self.workers.set_tailing(false);
    }

    /// Carga (perezosa) los datos de la pestaña activa del worker seleccionado.
    pub(crate) fn load_active_tab(&mut self) {
        let Some(script) = self.workers.selected_name() else {
            return;
        };
        match self.workers.active_tab {
            0 if self.workers.metrics.is_idle() => self.load_metrics(script),
            1 if self.workers.deployments.is_idle() => self.load_deployments(script),
            2 if self.workers.bindings.is_idle() => self.load_bindings(script),
            4 if self.workers.routing.is_idle() => self.load_routing(script),
            _ => {}
        }
    }

    pub(crate) fn open_http_test(&mut self) {
        let url = self
            .workers
            .suggested_url()
            .unwrap_or_else(|| "https://".into());
        self.popup = Some(Popup::HttpTest(HttpTest {
            url: TextInput::new(url),
            error: None,
            sending: false,
        }));
    }

    pub(crate) fn load_workers(&mut self) {
        self.workers.loading = true;
        self.workers.error = None;
        self.spawn_api(move |client, account_id, tx| async move {
            let sub = client.workers_subdomain(&account_id).await.ok().flatten();
            let _ = tx.send(Action::SubdomainLoaded(sub));
            let action = match client.list_scripts(&account_id).await {
                Ok(scripts) => Action::WorkersLoaded(scripts),
                Err(e) => Action::WorkersError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn load_metrics(&mut self, script: String) {
        self.workers.begin_metrics();
        self.spawn_api(move |client, account_id, tx| async move {
            let end = Utc::now();
            let start = end - chrono::Duration::hours(24);
            let start_s = start.to_rfc3339_opts(SecondsFormat::Secs, true);
            let end_s = end.to_rfc3339_opts(SecondsFormat::Secs, true);
            let metrics = match client
                .worker_metrics(&account_id, &script, &start_s, &end_s)
                .await
            {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::debug!("métricas {script}: {e}");
                    None
                }
            };
            let _ = tx.send(Action::MetricsLoaded { script, metrics });
        });
    }

    pub(crate) fn load_deployments(&mut self, script: String) {
        self.workers.begin_deployments();
        self.spawn_api(move |client, account_id, tx| async move {
            let deployments = client.list_deployments(&account_id, &script).await.ok();
            let _ = tx.send(Action::DeploymentsLoaded { script, deployments });
        });
    }

    pub(crate) fn load_bindings(&mut self, script: String) {
        self.workers.begin_bindings();
        self.spawn_api(move |client, account_id, tx| async move {
            let bindings = client.worker_bindings(&account_id, &script).await.ok();
            let _ = tx.send(Action::BindingsLoaded { script, bindings });
        });
    }

    /// Rutas de zona (fan-out) + custom domains del worker. Necesita las zonas
    /// de la cuenta; si aún no están, las carga y reintenta desde `ZonesLoaded`.
    pub(crate) fn load_routing(&mut self, script: String) {
        if self.all_zones.is_empty() && !self.dns.loading_zones {
            self.load_zones();
        }
        let zones: Vec<(String, String)> = self
            .account_zone_refs()
            .into_iter()
            .map(|z| (z.id, z.name))
            .collect();
        self.workers.begin_routing(zones.len());
        self.spawn_api(move |client, account_id, tx| async move {
            let routing = client
                .worker_routes_for(&account_id, &zones, &script)
                .await
                .ok()
                .map(|(routes, domains)| crate::components::workers::RoutingInfo {
                    routes: routes.into_iter().map(|(zn, r)| (r.pattern, zn)).collect(),
                    domains: domains.into_iter().map(|d| d.hostname).collect(),
                });
            let _ = tx.send(Action::RoutingLoaded { script, routing });
        });
    }

    pub(crate) fn spawn_rollback(&mut self, script: String, versions: Vec<crate::model::DeployVersion>) {
        self.status = "Revirtiendo…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .rollback_deployment(&account_id, &script, &versions)
                .await
            {
                Ok(()) => Action::DeploymentRolledBack {
                    script,
                    msg: "Rollback aplicado".into(),
                },
                Err(e) => Action::RollbackError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_probe(&mut self, url: String) {
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let r = crate::api::workers::http_probe(url).await;
            let _ = tx.send(Action::HttpResult {
                status: r.status,
                millis: r.millis,
                info: r.info,
            });
        });
    }

    /// Inicia el live-tail: crea la sesión, conecta el WS y transmite líneas.
    /// Un `oneshot` corta el bucle; al salir cierra el WS y borra la sesión.
    pub(crate) fn spawn_tail(&mut self, script: String) {
        // Detén cualquier tail previo antes de abrir otro.
        self.stop_tail();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        self.tail_stop = Some(stop_tx);
        self.workers.set_tab(3);
        self.workers.clear_logs();
        self.workers.set_tailing(true);
        self.workers
            .push_event(crate::api::workers::TailEvent::info("· conectando…"));
        self.status = "Iniciando tail…".into();

        self.spawn_api(move |client, account_id, tx| async move {
            use futures::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;

            let (tail_id, url) = match client.create_tail(&account_id, &script).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(Action::TailError {
                        script: script.clone(),
                        msg: e.to_string(),
                    });
                    let _ = tx.send(Action::TailEnded { script });
                    return;
                }
            };
            let mut ws = match crate::api::workers::connect_tail(&url).await {
                Ok(w) => w,
                Err(e) => {
                    let _ = tx.send(Action::TailError {
                        script: script.clone(),
                        msg: e.to_string(),
                    });
                    client.delete_tail(&account_id, &script, &tail_id).await.ok();
                    let _ = tx.send(Action::TailEnded { script });
                    return;
                }
            };
            // Mensaje de apertura del protocolo trace-v1 (como wrangler);
            // los filtros ya fueron en el POST de creación.
            let _ = ws
                .send(Message::Text("{\"debug\":false}".into()))
                .await;
            let _ = tx.send(Action::TailStarted {
                script: script.clone(),
            });

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    msg = ws.next() => match msg {
                        // El servidor puede emitir los eventos como frame de
                        // texto o binario (JSON en ambos casos).
                        Some(Ok(Message::Text(t))) => {
                            if let Some(event) = crate::api::workers::parse_tail(t.as_str()) {
                                let _ = tx.send(Action::TailPush { script: script.clone(), event });
                            }
                        }
                        Some(Ok(Message::Binary(b))) => {
                            let raw = String::from_utf8_lossy(&b);
                            if let Some(event) = crate::api::workers::parse_tail(&raw) {
                                let _ = tx.send(Action::TailPush { script: script.clone(), event });
                            }
                        }
                        Some(Ok(Message::Ping(p))) => {
                            let _ = ws.send(Message::Pong(p)).await;
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            let _ = tx.send(Action::TailError {
                                script: script.clone(),
                                msg: e.to_string(),
                            });
                            break;
                        }
                    }
                }
            }
            let _ = ws.close(None).await;
            client.delete_tail(&account_id, &script, &tail_id).await.ok();
            let _ = tx.send(Action::TailEnded { script });
        });
    }

    /// Guarda una variable/secreto. Los secretos usan `PUT .../secrets` (aislado);
    /// las vars planas usan `PATCH .../settings` conservando el resto con `inherit`.
    pub(crate) fn spawn_save_binding(
        &mut self,
        script: String,
        name: String,
        is_secret: bool,
        value: String,
        _adding: bool,
    ) {
        // Nombres de los demás bindings (para preservarlos con `inherit`).
        let others: Vec<String> = match &self.workers.bindings {
            Loadable::Ready(bs) => bs
                .iter()
                .map(|b| b.name.clone())
                .filter(|n| *n != name)
                .collect(),
            _ => Vec::new(),
        };
        self.status = "Guardando…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let result = if is_secret {
                client.put_secret(&account_id, &script, &name, &value).await
            } else {
                let mut arr: Vec<serde_json::Value> =
                    vec![serde_json::json!({ "type": "plain_text", "name": name, "text": value })];
                for n in &others {
                    arr.push(serde_json::json!({ "type": "inherit", "name": n }));
                }
                client
                    .update_worker_bindings(&account_id, &script, serde_json::Value::Array(arr))
                    .await
            };
            let action = match result {
                Ok(()) => Action::BindingSaved {
                    script,
                    msg: if is_secret {
                        "Secreto guardado".into()
                    } else {
                        "Variable guardada".into()
                    },
                },
                Err(e) => Action::BindingError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }
}
