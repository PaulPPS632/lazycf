//! Métodos de `App` del módulo dns (extraídos de `app/mod.rs`).

use super::*;

impl App {
    pub(crate) fn change_zone(&mut self, delta: i32) {
        if self.dns.select_zone(delta)
            && let Some(zone_id) = self.dns.selected_zone_id()
        {
            self.load_records(zone_id);
        }
    }

    pub(crate) fn reload_records(&mut self) {
        if let Some(zone_id) = self.dns.selected_zone_id() {
            self.load_records(zone_id);
        }
    }

    pub(crate) fn confirm_purge(&mut self) {
        let Some(zone) = self.dns.selected_zone() else {
            return;
        };
        let (zone_id, zone_name) = (zone.id.clone(), zone.name.clone());
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Purgar caché".into(),
            body: format!("¿Purgar TODA la caché de {zone_name}?"),
            on_yes: Action::PurgeCache { zone_id },
        }));
    }

    pub(crate) fn confirm_delete(&mut self) {
        let (Some(zone), Some(record)) = (self.dns.selected_zone(), self.dns.selected_record())
        else {
            return;
        };
        let zone_id = zone.id.clone();
        let record_id = record.id.clone();
        let label = format!("{} {}", record.record_type, record.name);
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar registro".into(),
            body: format!("¿Borrar el registro {label}?"),
            on_yes: Action::DeleteRecord { zone_id, record_id },
        }));
    }

    pub(crate) fn confirm_toggle_proxy(&mut self) {
        let Some(record) = self.dns.selected_record() else {
            return;
        };
        if !record.is_proxiable() {
            self.status = "Este tipo de registro no es proxiable".into();
            return;
        }
        let turning_on = record.proxied != Some(true);
        let name = record.name.clone();
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Cambiar proxy".into(),
            body: format!(
                "¿{} el proxy de {name}?",
                if turning_on { "Activar" } else { "Desactivar" }
            ),
            on_yes: Action::ToggleProxy,
        }));
    }

    pub(crate) fn open_add_record(&mut self) {
        let Some(zone_id) = self.dns.selected_zone_id() else {
            return;
        };
        self.popup = Some(Popup::RecordForm(RecordForm::create(zone_id)));
    }

    pub(crate) fn open_edit_record(&mut self) {
        let Some(zone_id) = self.dns.selected_zone_id() else {
            return;
        };
        let Some(record) = self.dns.selected_record() else {
            return;
        };
        self.popup = Some(Popup::RecordForm(RecordForm::edit(zone_id, record)));
    }

    pub(crate) fn load_zones(&mut self) {
        let Some(client) = self.client() else {
            return;
        };
        self.dns.loading_zones = true;
        self.dns.error = None;
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client.list_zones().await {
                Ok(zones) => Action::ZonesLoaded(zones),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn load_records(&mut self, zone_id: String) {
        let Some(client) = self.client() else {
            return;
        };
        self.dns.begin_loading_records();
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let action = match client.list_dns_records(&zone_id).await {
                Ok(records) => Action::RecordsLoaded { zone_id, records },
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn toggle_proxy(&mut self) {
        let (Some(client), Some(zone_id)) = (self.client(), self.dns.selected_zone_id()) else {
            return;
        };
        let Some(record) = self.dns.selected_record() else {
            return;
        };
        if !record.is_proxiable() {
            self.status = "Este tipo de registro no es proxiable".into();
            return;
        }
        let record_id = record.id.clone();
        let new_val = record.proxied != Some(true);
        let tx = self.action_tx.clone();
        self.status = "Cambiando proxy…".into();
        tokio::spawn(async move {
            let action = match client.set_dns_proxied(&zone_id, &record_id, new_val).await {
                Ok(_) => Action::DnsMutated(
                    if new_val {
                        "Proxy activado"
                    } else {
                        "Proxy desactivado"
                    }
                    .into(),
                ),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_delete(&mut self, zone_id: String, record_id: String) {
        let Some(client) = self.client() else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Borrando registro…".into();
        tokio::spawn(async move {
            let action = match client.delete_dns_record(&zone_id, &record_id).await {
                Ok(_) => Action::DnsMutated("Registro borrado".into()),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_submit_record(
        &mut self,
        zone_id: String,
        editing_id: Option<String>,
        rtype: String,
        name: String,
        content: String,
        ttl: String,
        proxied: bool,
        priority: String,
    ) {
        let Some(client) = self.client() else {
            return;
        };
        let tx = self.action_tx.clone();
        let editing = editing_id.is_some();
        self.status = if editing {
            "Guardando registro…"
        } else {
            "Creando registro…"
        }
        .into();

        let rtype_up = rtype.trim().to_uppercase();
        let ttl_num: u32 = ttl.trim().parse().unwrap_or(1);
        let mut body = serde_json::json!({
            "type": rtype_up,
            "name": name.trim(),
            "content": content.trim(),
            "ttl": ttl_num,
        });
        if matches!(rtype_up.as_str(), "A" | "AAAA" | "CNAME") {
            body["proxied"] = serde_json::Value::Bool(proxied);
        }
        if rtype_up == "MX" {
            body["priority"] = serde_json::json!(priority.trim().parse::<u32>().unwrap_or(10));
        }

        tokio::spawn(async move {
            let result = match &editing_id {
                Some(id) => client.update_dns_record(&zone_id, id, &body).await,
                None => client.create_dns_record(&zone_id, &body).await,
            };
            let action = match result {
                Ok(_) => Action::DnsMutated(
                    if editing {
                        "Registro actualizado"
                    } else {
                        "Registro creado"
                    }
                    .into(),
                ),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_purge(&mut self, zone_id: String) {
        let Some(client) = self.client() else {
            return;
        };
        let tx = self.action_tx.clone();
        self.status = "Purgando caché…".into();
        tokio::spawn(async move {
            let action = match client.purge_everything(&zone_id).await {
                Ok(_) => Action::DnsStatus("Caché purgada".into()),
                Err(e) => Action::DnsError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }
}
