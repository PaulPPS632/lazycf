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

/// Extrae los nombres de columna de un `CREATE TABLE …` de sqlite_master
/// (best-effort para el autocompletado; si no parsea, lista vacía).
pub fn parse_create_columns(sql: &str) -> Vec<String> {
    // Contenido entre el primer '(' y su ')' balanceado.
    let Some(start) = sql.find('(') else {
        return Vec::new();
    };
    let mut depth = 0usize;
    let mut end = None;
    for (i, c) in sql[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(end) = end else { return Vec::new() };
    let body = &sql[start + 1..end];

    // Split por comas a nivel 0 de paréntesis; primer token de cada segmento.
    const TABLE_CONSTRAINTS: [&str; 5] = ["PRIMARY", "FOREIGN", "UNIQUE", "CHECK", "CONSTRAINT"];
    let mut cols = Vec::new();
    let mut depth = 0usize;
    let mut seg = String::new();
    for c in body.chars().chain(std::iter::once(',')) {
        match c {
            '(' => {
                depth += 1;
                seg.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                seg.push(c);
            }
            ',' if depth == 0 => {
                let first = seg
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '[' | ']'))
                    .to_string();
                if !first.is_empty()
                    && !TABLE_CONSTRAINTS
                        .iter()
                        .any(|k| first.eq_ignore_ascii_case(k))
                {
                    cols.push(first);
                }
                seg.clear();
            }
            _ => seg.push(c),
        }
    }
    cols
}

/// Convierte una celda JSON a texto para mostrar en la tabla.
fn cell_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_create_columns;

    #[test]
    fn columnas_basicas_y_constraints() {
        let sql = r#"CREATE TABLE "users" (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            `name` TEXT NOT NULL,
            [email] TEXT UNIQUE,
            score NUMERIC(10, 2) DEFAULT (1 + 2),
            PRIMARY KEY (id),
            FOREIGN KEY (email) REFERENCES x(y),
            CONSTRAINT chk CHECK (score > 0)
        )"#;
        assert_eq!(parse_create_columns(sql), ["id", "name", "email", "score"]);
    }

    #[test]
    fn sin_parentesis_devuelve_vacio() {
        assert!(parse_create_columns("CREATE VIEW v AS SELECT 1").is_empty());
    }
}
