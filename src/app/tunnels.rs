//! Métodos de `App` del módulo tunnels (extraídos de `app/mod.rs`).

use super::*;

impl App {
    pub(crate) fn open_new_tunnel(&mut self) {
        self.popup = Some(Popup::TextPrompt(TextPrompt::new(PromptKind::NewTunnel)));
    }

    pub(crate) fn open_new_route(&mut self) {
        let Some(tunnel) = self.tunnels.selected() else {
            return;
        };
        let (id, name) = (tunnel.id.clone(), tunnel.name.clone());
        // Zonas de la cuenta para el select de dominio. Si aún no se han cargado
        // (no se entró a DNS), se lanza la carga y se rellenan al llegar.
        if self.all_zones.is_empty() {
            self.load_zones();
        }
        let zones = self.account_zone_refs();
        self.popup = Some(Popup::RouteForm(RouteForm::new(id, name, zones)));
    }

    pub(crate) fn open_edit_route(&mut self) {
        let (Some(tunnel), Some(route)) = (self.tunnels.selected(), self.tunnels.selected_route())
        else {
            return;
        };
        let (id, name) = (tunnel.id.clone(), tunnel.name.clone());
        self.popup = Some(Popup::RouteForm(RouteForm::edit(
            id,
            name,
            route.hostname.clone(),
            route.service.clone(),
            route.path.clone().unwrap_or_default(),
        )));
    }

    pub(crate) fn confirm_delete_route(&mut self) {
        let (Some(tunnel), Some(route)) = (self.tunnels.selected(), self.tunnels.selected_route())
        else {
            return;
        };
        let tunnel_id = tunnel.id.clone();
        let hostname = route.hostname.clone();
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar ruta".into(),
            body: format!(
                "¿Borrar la ruta {hostname}?\n\nQuita solo la regla del túnel; el registro DNS \
                 (CNAME) se conserva — bórralo aparte en el módulo DNS si ya no lo necesitas."
            ),
            on_yes: Action::DeleteTunnelRoute {
                tunnel_id,
                hostname,
            },
        }));
    }

    pub(crate) fn spawn_add_route(
        &mut self,
        tunnel_id: String,
        hostname: String,
        service: String,
        path: String,
        dns_zone: Option<String>,
    ) {
        self.status = "Añadiendo ruta…".into();
        let hostname = hostname.trim().to_string();
        let service = service.trim().to_string();
        let path_opt = {
            let p = path.trim().to_string();
            (!p.is_empty()).then_some(p)
        };
        let target = format!("{tunnel_id}.cfargotunnel.com");
        self.spawn_api(move |client, account_id, tx| async move {
            // 1. Regla de ingress.
            if let Err(e) = client
                .add_tunnel_route(
                    &account_id,
                    &tunnel_id,
                    &hostname,
                    &service,
                    path_opt.as_deref(),
                )
                .await
            {
                let _ = tx.send(Action::TunnelRouteError(e.to_string()));
                return;
            }
            // 2. CNAME proxied (si se pidió y hay zona). El fallo del DNS no anula
            //    la ruta ya creada: se reporta como aviso.
            let msg = match dns_zone {
                Some(zone_id) => {
                    let body = serde_json::json!({
                        "type": "CNAME",
                        "name": hostname,
                        "content": target,
                        "proxied": true,
                        "ttl": 1,
                    });
                    match client.create_dns_record(&zone_id, &body).await {
                        Ok(_) => format!("Ruta {hostname} añadida (+ DNS)"),
                        Err(e) => format!("Ruta {hostname} añadida, pero el DNS falló: {e}"),
                    }
                }
                None => format!("Ruta {hostname} añadida (crea el DNS manualmente)"),
            };
            let _ = tx.send(Action::TunnelRouteMutated(msg));
        });
    }

    pub(crate) fn spawn_edit_route(
        &mut self,
        tunnel_id: String,
        hostname: String,
        service: String,
        path: String,
    ) {
        self.status = "Guardando ruta…".into();
        let service = service.trim().to_string();
        let path_opt = {
            let p = path.trim().to_string();
            (!p.is_empty()).then_some(p)
        };
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .update_tunnel_route(
                    &account_id,
                    &tunnel_id,
                    &hostname,
                    &service,
                    path_opt.as_deref(),
                )
                .await
            {
                Ok(()) => Action::TunnelRouteMutated(format!("Ruta {hostname} actualizada")),
                Err(e) => Action::TunnelRouteError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_delete_route(&mut self, tunnel_id: String, hostname: String) {
        self.status = "Borrando ruta…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .delete_tunnel_route(&account_id, &tunnel_id, &hostname)
                .await
            {
                Ok(()) => Action::TunnelRouteMutated(format!("Ruta {hostname} borrada")),
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn change_tunnel(&mut self, delta: i32) {
        if self.tunnels.select(delta)
            && let Some(tunnel_id) = self.tunnels.selected_id()
        {
            self.load_ingress(tunnel_id);
        }
    }

    pub(crate) fn confirm_cleanup(&mut self) {
        let Some(tunnel) = self.tunnels.selected() else {
            return;
        };
        let (tunnel_id, name) = (tunnel.id.clone(), tunnel.name.clone());
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Limpiar conexiones".into(),
            body: format!("¿Desconectar todas las conexiones de {name}?"),
            on_yes: Action::CleanupConnections { tunnel_id },
        }));
    }

    pub(crate) fn confirm_delete_tunnel(&mut self) {
        let Some(tunnel) = self.tunnels.selected() else {
            return;
        };
        let (tunnel_id, name) = (tunnel.id.clone(), tunnel.name.clone());
        self.popup = Some(Popup::Confirm(Confirm {
            title: "Borrar túnel".into(),
            body: format!("¿Borrar el túnel {name}? Se limpian sus conexiones primero."),
            on_yes: Action::DeleteTunnel { tunnel_id },
        }));
    }

    pub(crate) fn load_tunnels(&mut self) {
        self.tunnels.loading = true;
        self.tunnels.error = None;
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.list_tunnels(&account_id).await {
                Ok(tunnels) => Action::TunnelsLoaded(tunnels),
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn load_ingress(&mut self, tunnel_id: String) {
        self.tunnels.begin_loading_ingress();
        self.spawn_api(move |client, account_id, tx| async move {
            // Un 404 aquí = túnel local sin config remota → se trata como vacío.
            let rules = match client.tunnel_ingress(&account_id, &tunnel_id).await {
                Ok(rules) => rules,
                Err(e) => {
                    tracing::debug!("ingress {tunnel_id}: {e}");
                    Vec::new()
                }
            };
            let _ = tx.send(Action::IngressLoaded { tunnel_id, rules });
        });
    }

    pub(crate) fn spawn_create_tunnel(&mut self, name: String) {
        self.status = "Creando túnel…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.create_tunnel(&account_id, &name).await {
                Ok(t) => Action::TunnelCreated {
                    name: t.name,
                    token: t.token,
                },
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_cleanup(&mut self, tunnel_id: String) {
        self.status = "Limpiando conexiones…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client
                .cleanup_tunnel_connections(&account_id, &tunnel_id)
                .await
            {
                Ok(()) => Action::TunnelMutated("Conexiones limpiadas".into()),
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_delete_tunnel(&mut self, tunnel_id: String) {
        self.status = "Borrando túnel…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            // Limpiar conexiones primero (si no hay, se ignora el error).
            let _ = client
                .cleanup_tunnel_connections(&account_id, &tunnel_id)
                .await;
            let action = match client.delete_tunnel(&account_id, &tunnel_id).await {
                Ok(()) => Action::TunnelMutated("Túnel borrado".into()),
                Err(e) => Action::TunnelError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }
}
