//! Endpoints de D1 (Fase 5): listar bases, esquema (sqlite_master / PRAGMA) y
//! consola SQL vía `POST .../raw` (devuelve columnas + filas ordenadas).

use color_eyre::eyre::Result;
use serde::Deserialize;
use serde_json::json;

use super::CfClient;
use crate::model::{D1Database, QueryOutcome};

// --- Estructuras de la respuesta de `/raw` ---
// `result` es un array (un elemento por sentencia); cada uno trae
// `results: { columns, rows }` y `meta` con los contadores.

#[derive(Debug, Deserialize)]
struct RawResult {
    #[serde(default)]
    results: RawRows,
    #[serde(default)]
    meta: RawMeta,
    #[serde(default)]
    success: bool,
}

#[derive(Debug, Deserialize, Default)]
struct RawRows {
    #[serde(default)]
    columns: Vec<String>,
    #[serde(default)]
    rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize, Default)]
struct RawMeta {
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    rows_read: u64,
    #[serde(default)]
    rows_written: u64,
    #[serde(default)]
    changes: u64,
}

impl CfClient {
    /// `GET /accounts/{id}/d1/database` — bases de datos D1 de la cuenta.
    pub async fn list_databases(&self, account_id: &str) -> Result<Vec<D1Database>> {
        self.get(&format!("/accounts/{account_id}/d1/database")).await
    }

    /// `POST .../d1/database/{db}/raw` — ejecuta SQL y devuelve la última
    /// sentencia como tabla (columnas + filas ya en orden).
    pub async fn d1_query(
        &self,
        account_id: &str,
        db_id: &str,
        sql: &str,
    ) -> Result<QueryOutcome> {
        let body = json!({ "sql": sql });
        let mut results: Vec<RawResult> = self
            .post(
                &format!("/accounts/{account_id}/d1/database/{db_id}/raw"),
                &body,
            )
            .await?;

        // La última sentencia con éxito es la relevante (p. ej. el SELECT final).
        let last = results
            .iter()
            .rposition(|r| r.success)
            .map(|i| results.swap_remove(i))
            .or_else(|| results.pop());

        let Some(r) = last else {
            return Ok(QueryOutcome::default());
        };
        Ok(QueryOutcome {
            columns: r.results.columns,
            rows: r
                .results
                .rows
                .into_iter()
                .map(|row| row.iter().map(cell_to_string).collect())
                .collect(),
            rows_read: r.meta.rows_read,
            rows_written: r.meta.rows_written,
            changes: r.meta.changes,
            duration_ms: r.meta.duration,
        })
    }
}

/// Convierte una celda JSON a texto para mostrar en la tabla.
fn cell_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
