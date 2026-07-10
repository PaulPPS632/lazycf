//! Métodos de `App` del módulo queues (extraídos de `app/mod.rs`).

use super::*;

impl App {
    pub(crate) fn change_queue(&mut self, delta: i32) {
        if self.queues.select(delta) {
            self.queues.reset_tabs();
            self.load_active_queue_tab();
        }
    }

    /// Carga (perezosa) los datos de la pestaña activa de la cola seleccionada.
    pub(crate) fn load_active_queue_tab(&mut self) {
        let Some(queue_id) = self.queues.selected_id() else {
            return;
        };
        match self.queues.active_tab {
            1 if self.queues.consumers.is_idle() => self.load_queue_consumers(queue_id),
            2 if self.queues.metrics.is_idle() => self.load_queue_metrics(queue_id),
            _ => {}
        }
    }

    pub(crate) fn load_queues(&mut self) {
        self.queues.loading = true;
        self.queues.error = None;
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.list_queues(&account_id).await {
                Ok(qs) => Action::QueuesLoaded(qs),
                Err(e) => Action::QueueError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn load_queue_consumers(&mut self, queue_id: String) {
        self.queues.begin_consumers();
        self.spawn_api(move |client, account_id, tx| async move {
            let consumers = client.list_consumers(&account_id, &queue_id).await.ok();
            let _ = tx.send(Action::ConsumersLoaded {
                queue_id,
                consumers,
            });
        });
    }

    pub(crate) fn load_queue_metrics(&mut self, queue_id: String) {
        self.queues.begin_metrics();
        self.spawn_api(move |client, account_id, tx| async move {
            let end = Utc::now();
            let start = end - chrono::Duration::hours(24);
            let start_s = start.to_rfc3339_opts(SecondsFormat::Secs, true);
            let end_s = end.to_rfc3339_opts(SecondsFormat::Secs, true);
            let metrics = match client
                .queue_metrics(&account_id, &queue_id, &start_s, &end_s)
                .await
            {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::debug!("métricas de cola {queue_id}: {e}");
                    None
                }
            };
            let _ = tx.send(Action::QueueMetricsLoaded { queue_id, metrics });
        });
    }

    pub(crate) fn open_new_queue(&mut self) {
        self.popup = Some(Popup::TextPrompt(TextPrompt::new(PromptKind::NewQueue)));
    }

    pub(crate) fn spawn_create_queue(&mut self, name: String) {
        self.status = "Creando cola…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.create_queue(&account_id, &name).await {
                Ok(()) => Action::QueueMutated(format!("Cola '{name}' creada")),
                Err(e) => Action::QueueError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn confirm_delete_queue(&mut self) {
        let (Some(queue_id), Some(name)) = (self.queues.selected_id(), self.queues.selected_name())
        else {
            return;
        };
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar cola".into(),
            body: format!("¿Borrar la cola '{name}'? Se perderán sus mensajes pendientes."),
            on_yes: Action::DeleteQueue { queue_id },
        }));
    }

    pub(crate) fn spawn_delete_queue(&mut self, queue_id: String) {
        self.status = "Borrando cola…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.delete_queue(&account_id, &queue_id).await {
                Ok(()) => Action::QueueMutated("Cola borrada".into()),
                Err(e) => Action::QueueError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn confirm_pause_toggle(&mut self) {
        let Some(q) = self.queues.selected() else {
            return;
        };
        let (queue_id, queue_name, paused) = (
            q.queue_id.clone(),
            q.queue_name.clone(),
            !q.settings.delivery_paused,
        );
        let (title, body) = if paused {
            (
                "Pausar entrega".to_string(),
                format!(
                    "Los consumers de '{queue_name}' dejarán de recibir mensajes; los producers siguen encolando. ¿Continuar?"
                ),
            )
        } else {
            (
                "Reanudar entrega".to_string(),
                format!("¿Reanudar la entrega de mensajes de '{queue_name}'?"),
            )
        };
        self.popup = Some(Popup::Confirm(Confirm {
            title,
            body,
            on_yes: Action::PauseQueue {
                queue_id,
                queue_name,
                paused,
            },
        }));
    }

    pub(crate) fn spawn_pause_queue(&mut self, queue_id: String, queue_name: String, paused: bool) {
        self.status = if paused {
            "Pausando…".into()
        } else {
            "Reanudando…".into()
        };
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .set_delivery_paused(&account_id, &queue_id, &queue_name, paused)
                .await
            {
                Ok(()) => Action::QueueMutated(if paused {
                    "Entrega pausada".into()
                } else {
                    "Entrega reanudada".into()
                }),
                Err(e) => Action::QueueError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn confirm_purge_queue(&mut self) {
        let (Some(queue_id), Some(name)) = (self.queues.selected_id(), self.queues.selected_name())
        else {
            return;
        };
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Purgar cola".into(),
            body: format!("¿Borrar TODOS los mensajes de '{name}'? Esta acción es irreversible."),
            on_yes: Action::PurgeQueue { queue_id },
        }));
    }

    pub(crate) fn spawn_purge_queue(&mut self, queue_id: String) {
        self.status = "Purgando…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.purge_queue(&account_id, &queue_id).await {
                Ok(()) => Action::QueueMutated("Cola purgada".into()),
                Err(e) => Action::QueueError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn open_send_message(&mut self) {
        let (Some(queue_id), Some(queue_name)) =
            (self.queues.selected_id(), self.queues.selected_name())
        else {
            return;
        };
        self.popup = Some(Popup::SendMessage(SendMessageForm::new(
            queue_id, queue_name,
        )));
    }

    pub(crate) fn spawn_send_message(
        &mut self,
        queue_id: String,
        body: String,
        content_type: String,
        delay_seconds: Option<u64>,
    ) {
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .push_message(&account_id, &queue_id, &body, &content_type, delay_seconds)
                .await
            {
                Ok(()) => Action::MessageSent("Mensaje enviado".into()),
                Err(e) => Action::SendMessageError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// `e`/Enter en la pestaña Consumers: edita batch/retries/DLQ/etc.
    pub(crate) fn open_edit_consumer(&mut self) {
        let Some(queue_id) = self.queues.selected_id() else {
            return;
        };
        let Some(c) = self.queues.selected_consumer() else {
            self.status = "Sin consumers cargados (pulsa la pestaña Consumers)".into();
            return;
        };
        if c.consumer_id.is_empty() {
            self.status = "Este consumer no tiene id: no se puede editar".into();
            return;
        }
        self.popup = Some(Popup::ConsumerEdit(ConsumerEditForm::edit(queue_id, c)));
    }

    pub(crate) fn spawn_update_consumer(
        &mut self,
        queue_id: String,
        consumer_id: String,
        body: serde_json::Value,
    ) {
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .update_consumer(&account_id, &queue_id, &consumer_id, &body)
                .await
            {
                Ok(()) => Action::ConsumerSaved {
                    queue_id,
                    msg: "Consumer actualizado".into(),
                },
                Err(e) => Action::ConsumerError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    /// `m`: espía mensajes (peek, sin ack) — solo tiene sentido en colas con
    /// consumer http_pull; si hay un consumer worker, el API lo rechaza.
    pub(crate) fn open_peek(&mut self) {
        let Some(queue_id) = self.queues.selected_id() else {
            return;
        };
        if self
            .queues
            .effective_consumers()
            .iter()
            .any(|c| c.is_worker())
        {
            self.status =
                "Esta cola tiene un consumer worker: no se pueden espiar mensajes (usa 'l' para ver logs)"
                    .into();
            return;
        }
        self.spawn_pull_messages(queue_id);
    }

    pub(crate) fn spawn_pull_messages(&mut self, queue_id: String) {
        self.status = "Espiando mensajes…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let outcome = client
                .pull_messages(&account_id, &queue_id, 20, 30_000)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Action::MessagesPulled { queue_id, outcome });
        });
    }

    /// `l`: salta al módulo Workers con el consumer worker de la cola
    /// tailando en vivo (reusa toda la infra de logs de Workers).
    pub(crate) fn open_consumer_logs(&mut self) {
        match self.queues.consumer_script() {
            Some(script) => self.jump_to_consumer_logs(script),
            None => self.status = "Esta cola no tiene consumer Worker".into(),
        }
    }

    pub(crate) fn jump_to_consumer_logs(&mut self, script: String) {
        self.sidebar.set_module(Module::Workers);
        if self.workers.is_empty() {
            self.pending_tail = Some(script);
            self.focus = Focus::Workers;
            if !self.workers.loading {
                self.load_workers();
            }
            self.status = "Cargando workers para el tail…".into();
            return;
        }
        if self.workers.select_by_name(&script) {
            self.workers.reset_tabs();
            self.focus = Focus::WorkersDetail;
            self.dispatch(Action::StartTail(script));
        } else {
            self.status = format!("Worker '{script}' no está en la lista");
        }
    }
}
