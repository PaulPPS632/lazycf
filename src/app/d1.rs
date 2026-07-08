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
        // La consulta libre pasa a ser la base del filtro WHERE (se envuelve
        // como subquery al aplicar la cláusula).
        self.d1
            .set_filter_base(Some(crate::components::d1::FilterBase::Query(sql.clone())));
        self.spawn_d1_query(db_id, "consulta".into(), auto_limit(&sql));
    }

    /// `Enter` sobre una tabla: vuelca `SELECT *` en el editor y lo ejecuta.
    pub(crate) fn run_select(&mut self) {
        let (Some(db_id), Some(table)) = (self.d1.selected_db_id(), self.d1.selected_table())
        else {
            return;
        };
        let sql = format!("SELECT * FROM {} LIMIT 50", quote_ident(&table));
        self.d1.set_sql(sql.clone());
        self.d1
            .set_filter_base(Some(crate::components::d1::FilterBase::Table(table.clone())));
        self.spawn_d1_query(db_id, format!("{table} · SELECT * LIMIT 50"), sql);
    }

    /// Reejecuta la base actual (tabla o consulta libre) con la cláusula WHERE
    /// de la barra de filtro. Las consultas libres se envuelven como subquery.
    pub(crate) fn apply_where_filter(&mut self) {
        use crate::components::d1::FilterBase;
        let (Some(db_id), Some(base)) = (self.d1.selected_db_id(), self.d1.filter_base()) else {
            self.status = "El filtro aplica tras ejecutar una tabla o consulta".into();
            return;
        };
        let clause = self.d1.where_trimmed();
        let (sql, title) = match &base {
            FilterBase::Table(table) => {
                let ident = quote_ident(table);
                if clause.is_empty() {
                    (
                        format!("SELECT * FROM {ident} LIMIT 50"),
                        format!("{table} · SELECT * LIMIT 50"),
                    )
                } else {
                    (
                        format!("SELECT * FROM {ident} WHERE {clause} LIMIT 50"),
                        format!("{table} · WHERE {clause}"),
                    )
                }
            }
            FilterBase::Query(query) => {
                // Sin los `;` finales: la consulta va dentro de un paréntesis.
                let mut inner = query.trim_end();
                while let Some(stripped) = inner.strip_suffix(';') {
                    inner = stripped.trim_end();
                }
                if clause.is_empty() {
                    (auto_limit(inner), "consulta".to_string())
                } else {
                    (
                        auto_limit(&format!("SELECT * FROM ({inner}) WHERE {clause}")),
                        format!("consulta · WHERE {clause}"),
                    )
                }
            }
        };
        self.d1.set_filter_base(Some(base));
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
        // El PRAGMA no es filtrable con WHERE.
        self.d1.set_filter_base(None);
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

/// Añade `LIMIT MAX_GRID_ROWS+1` a las consultas de lectura del editor que no
/// traen su propio LIMIT (truco N+1: si llega la fila extra, hubo truncado).
/// INSERT/UPDATE/DELETE/PRAGMA/etc. pasan intactas.
fn auto_limit(sql: &str) -> String {
    let lower = sql.to_lowercase();
    let trimmed = lower.trim_start();
    let is_read = trimmed.starts_with("select") || trimmed.starts_with("with");
    // Token `limit` en cualquier parte (best-effort: cubre el caso normal;
    // un `limit` dentro de un string literal solo desactiva el auto-límite).
    let has_limit = lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|tok| tok == "limit");
    if is_read && !has_limit {
        // Sin los `;` finales: `… ; LIMIT n` es un error de sintaxis en SQLite.
        let mut base = sql.trim_end();
        while let Some(stripped) = base.strip_suffix(';') {
            base = stripped.trim_end();
        }
        format!("{base} LIMIT {}", crate::api::d1::MAX_GRID_ROWS + 1)
    } else {
        sql.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::auto_limit;

    #[test]
    fn select_sin_limit_lo_recibe() {
        let out = auto_limit("SELECT * FROM users");
        assert!(out.ends_with("LIMIT 2001"), "{out}");
    }

    #[test]
    fn select_con_limit_queda_intacto() {
        let sql = "select * from users LIMIT 10";
        assert_eq!(auto_limit(sql), sql);
    }

    #[test]
    fn with_cte_lo_recibe() {
        let out = auto_limit("WITH t AS (SELECT 1) SELECT * FROM t");
        assert!(out.ends_with("LIMIT 2001"), "{out}");
    }

    #[test]
    fn mutaciones_intactas() {
        for sql in [
            "INSERT INTO x VALUES (1)",
            "update x set a = 1",
            "DELETE FROM x",
            "PRAGMA table_info(x)",
        ] {
            assert_eq!(auto_limit(sql), sql);
        }
    }

    #[test]
    fn limit_como_identificador_no_confunde() {
        // Columna llamada `limits` NO es el token `limit`.
        let out = auto_limit("SELECT limits FROM plans");
        assert!(out.ends_with("LIMIT 2001"), "{out}");
    }

    #[test]
    fn punto_y_coma_final_se_recorta() {
        // `… ; LIMIT n` es error de sintaxis en SQLite (bug reportado).
        assert_eq!(
            auto_limit("select * from capitulo_imagen;"),
            "select * from capitulo_imagen LIMIT 2001"
        );
        assert_eq!(auto_limit("select 1 ;  ; "), "select 1 LIMIT 2001");
    }
}
