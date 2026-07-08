//! Métodos de `App` del módulo d1 (extraídos de `app/mod.rs`).

use super::*;

impl App {
    pub(crate) fn change_db(&mut self, delta: i32) {
        if self.d1.select_db(delta)
            && let Some(db_id) = self.d1.selected_db_id()
        {
            self.load_tables(db_id);
        }
    }

    pub(crate) fn change_table(&mut self, delta: i32) {
        if self.d1.select_table(delta) {
            self.load_table_schema();
        }
    }

    pub(crate) fn reload_tables(&mut self) {
        if let Some(db_id) = self.d1.selected_db_id() {
            self.load_tables(db_id);
        }
    }

    /// Ejecuta el contenido del editor SQL contra la base seleccionada.
    pub(crate) fn run_editor(&mut self) {
        let Some(db_id) = self.d1.selected_db_id() else {
            self.status = "Selecciona una base".into();
            return;
        };
        let sql = self.d1.sql_trimmed();
        if sql.is_empty() {
            self.status = "Escribe una consulta".into();
            return;
        }
        // Consulta libre: el filtro WHERE no aplica (no hay tabla base conocida).
        self.d1.set_filter_table(None);
        self.spawn_d1_query(db_id, "consulta".into(), sql);
    }

    /// `Enter` sobre una tabla: vuelca `SELECT *` en el editor y lo ejecuta.
    pub(crate) fn run_select(&mut self) {
        let (Some(db_id), Some(table)) = (self.d1.selected_db_id(), self.d1.selected_table())
        else {
            return;
        };
        let sql = format!("SELECT * FROM {} LIMIT 50", quote_ident(&table));
        self.d1.set_sql(sql.clone());
        self.d1.set_filter_table(Some(table.clone()));
        self.spawn_d1_query(db_id, format!("{table} · SELECT * LIMIT 50"), sql);
    }

    /// Reejecuta la tabla actual con la cláusula WHERE de la barra de filtro.
    pub(crate) fn apply_where_filter(&mut self) {
        let (Some(db_id), Some(table)) = (self.d1.selected_db_id(), self.d1.filter_table()) else {
            self.status = "El filtro aplica a una tabla seleccionada".into();
            return;
        };
        let clause = self.d1.where_trimmed();
        let ident = quote_ident(&table);
        let (sql, title) = if clause.is_empty() {
            (
                format!("SELECT * FROM {ident} LIMIT 50"),
                format!("{table} · SELECT * LIMIT 50"),
            )
        } else {
            (
                format!("SELECT * FROM {ident} WHERE {clause} LIMIT 50"),
                format!("{table} · WHERE {clause}"),
            )
        };
        self.d1.set_filter_table(Some(table));
        self.spawn_d1_query(db_id, title, sql);
    }

    /// Popup con el valor completo de la celda seleccionada.
    pub(crate) fn open_cell_view(&mut self) {
        if let Some((col, val)) = self.d1.selected_cell_value() {
            self.popup = Some(Popup::Message(Message {
                title: col,
                body: val,
                is_error: false,
            }));
        }
    }

    pub(crate) fn copy_cell(&mut self) {
        if let Some((_, val)) = self.d1.selected_cell_value() {
            crate::tui::osc52_copy(&val);
            self.status = "Celda copiada".into();
        }
    }

    pub(crate) fn copy_row(&mut self) {
        if let Some(tsv) = self.d1.selected_row_tsv() {
            crate::tui::osc52_copy(&tsv);
            self.status = "Fila copiada".into();
        }
    }

    pub(crate) fn load_table_schema(&mut self) {
        let (Some(db_id), Some(table)) = (self.d1.selected_db_id(), self.d1.selected_table())
        else {
            return;
        };
        let sql = format!("PRAGMA table_info({})", quote_ident(&table));
        self.d1.set_filter_table(None);
        self.spawn_d1_query(db_id, format!("{table} · columnas"), sql);
    }

    pub(crate) fn load_databases(&mut self) {
        self.d1.loading = true;
        self.d1.error = None;
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.list_databases(&account_id).await {
                Ok(dbs) => Action::D1DatabasesLoaded(dbs),
                Err(e) => Action::D1Error(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn load_tables(&mut self, db_id: String) {
        self.d1.begin_tables(db_id.clone());
        // `sql` (el CREATE) alimenta el autocompletado de columnas del editor.
        let sql = "SELECT name, sql FROM sqlite_master WHERE type IN ('table','view') \
                   AND name NOT LIKE 'sqlite_%' ORDER BY name";
        self.spawn_api(move |client, account_id, tx| async move {
            let action = match client.d1_query(&account_id, &db_id, sql).await {
                Ok(o) => {
                    let mut tables = Vec::new();
                    let mut schema = std::collections::HashMap::new();
                    for row in o.rows {
                        let Some(name) = row.first().cloned() else {
                            continue;
                        };
                        let cols = row
                            .get(1)
                            .map(|c| crate::api::d1::parse_create_columns(c))
                            .unwrap_or_default();
                        schema.insert(name.clone(), cols);
                        tables.push(name);
                    }
                    Action::D1TablesLoaded {
                        db_id,
                        tables,
                        schema,
                    }
                }
                Err(e) => Action::D1TablesError(e.to_string()),
            };
            let _ = tx.send(action);
        });
    }

    pub(crate) fn spawn_d1_query(&mut self, db_id: String, title: String, sql: String) {
        self.d1.begin_result();
        self.status = "Ejecutando SQL…".into();
        self.spawn_api(move |client, account_id, tx| async move {
            let outcome = client
                .d1_query(&account_id, &db_id, &sql)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Action::D1ResultLoaded {
                db_id,
                title,
                outcome,
            });
        });
    }
}
