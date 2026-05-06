use crate::cli::Cli;
use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{FromRow, Row};
use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub db_url: String,
    pub db_name: String,
    pub collect_samples: bool,
    pub ignore_tables: Vec<String>,
    pub max_sample_len: usize,
}

pub fn resolve_config(cli: &Cli) -> Result<AppConfig> {
    let db_url = cli
        .db_url
        .clone()
        .or_else(|| env::var("DB_URL").ok())
        .context("DB_URL must be set via --db-url or in .env/environment variables")?;

    let db_name = db_url.split('/').last().unwrap_or("Unknown").to_string();
    let ignore_tables = cli.exclude.clone().unwrap_or_default();

    Ok(AppConfig {
        db_url,
        db_name,
        collect_samples: cli.samples,
        ignore_tables,
        max_sample_len: cli.max_sample_len,
    })
}

#[derive(FromRow, Debug, Clone, Serialize)]
pub struct ColumnInfo {
    pub column_name: String,
    pub data_type: String,
    pub is_nullable: String,
    pub udt_name: String,
    pub comment: Option<String>,
}

#[derive(FromRow, Debug, Clone, Serialize)]
pub struct ForeignKeyInfo {
    pub column_name: String,
    pub foreign_table_name: String,
    pub foreign_column_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableData {
    pub name: String,
    pub comment: Option<String>,
    pub columns: Vec<ColumnInfo>,
    pub primary_keys: Vec<String>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub sample_rows: Vec<String>,
}

pub struct Inspector<'a> {
    pool: &'a sqlx::PgPool,
    collect_samples: bool,
    ignore_tables: Vec<String>,
    max_sample_len: usize,
}

impl<'a> Inspector<'a> {
    pub fn new(
        pool: &'a sqlx::PgPool,
        collect_samples: bool,
        ignore_tables: Vec<String>,
        max_sample_len: usize,
    ) -> Self {
        Self {
            pool,
            collect_samples,
            ignore_tables,
            max_sample_len,
        }
    }

    pub async fn scan(&self) -> Result<Vec<TableData>> {
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public'",
        )
        .fetch_all(self.pool)
        .await?;

        let mut results = Vec::new();

        for (table_name,) in tables {
            if self.ignore_tables.contains(&table_name) {
                continue;
            }

            let table_comment: Option<String> = sqlx::query_scalar(
                "SELECT pg_catalog.obj_description(format('%I.%I', 'public', $1)::regclass::oid, 'pg_class')"
            )
            .bind(&table_name)
            .fetch_one(self.pool)
            .await?;

            let columns = self.get_columns(&table_name).await?;
            let primary_keys = self.get_primary_keys(&table_name).await?;
            let foreign_keys = self.get_foreign_keys(&table_name).await?;

            let sample_rows = if self.collect_samples {
                self.get_sample_data(&table_name, &columns).await?
            } else {
                Vec::new()
            };

            results.push(TableData {
                name: table_name,
                comment: table_comment,
                columns,
                primary_keys,
                foreign_keys,
                sample_rows,
            });
        }

        Ok(results)
    }

    async fn get_columns(&self, table_name: &str) -> Result<Vec<ColumnInfo>> {
        sqlx::query_as::<_, ColumnInfo>(
            r#"
            SELECT 
                column_name, 
                data_type, 
                is_nullable, 
                udt_name,
                pg_catalog.col_description(format('%I.%I', table_schema, table_name)::regclass::oid, ordinal_position) as comment
            FROM information_schema.columns 
            WHERE table_name = $1 AND table_schema = 'public'
            ORDER BY ordinal_position
            "#,
        )
        .bind(table_name)
        .fetch_all(self.pool)
        .await
        .map_err(|e| e.into())
    }

    async fn get_primary_keys(&self, table_name: &str) -> Result<Vec<String>> {
        let result: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT kcu.column_name
            FROM information_schema.table_constraints tc
            JOIN information_schema.key_column_usage kcu
              ON tc.constraint_name = kcu.constraint_name
              AND tc.table_schema = kcu.table_schema
            WHERE tc.constraint_type = 'PRIMARY KEY' 
              AND tc.table_name = $1
            "#,
        )
        .bind(table_name)
        .fetch_all(self.pool)
        .await?;
        Ok(result.into_iter().map(|(name,)| name).collect())
    }

    async fn get_foreign_keys(&self, table_name: &str) -> Result<Vec<ForeignKeyInfo>> {
        sqlx::query_as::<_, ForeignKeyInfo>(
            r#"
            SELECT
                kcu.column_name,
                ccu.table_name AS foreign_table_name,
                ccu.column_name AS foreign_column_name
            FROM information_schema.key_column_usage AS kcu
            JOIN information_schema.referential_constraints AS rc
                ON kcu.constraint_name = rc.constraint_name
            JOIN information_schema.constraint_column_usage AS ccu
                ON rc.unique_constraint_name = ccu.constraint_name
            WHERE kcu.table_name = $1
            "#,
        )
        .bind(table_name)
        .fetch_all(self.pool)
        .await
        .map_err(|e| e.into())
    }

    async fn get_sample_data(
        &self,
        table_name: &str,
        columns: &[ColumnInfo],
    ) -> Result<Vec<String>> {
        let mut select_parts = Vec::new();
        for col in columns {
            let safe_col_name = col.column_name.replace('"', "\"\"");

            if col.data_type == "bytea" {
                select_parts.push(format!("'[bytea]'::text AS \"{}\"", safe_col_name));
            } else if col.udt_name == "vector" {
                select_parts.push(format!("'[vector]'::text AS \"{}\"", safe_col_name));
            } else {
                select_parts.push(format!("\"{}\"", safe_col_name));
            }
        }

        if select_parts.is_empty() {
            return Ok(vec![]);
        }

        let select_list = select_parts.join(", ");
        let safe_table_name = table_name.replace('"', "\"\"");
        let data_query = format!(
            "SELECT row_to_json(t)::text FROM (SELECT {} FROM \"{}\" LIMIT 5) t",
            select_list, safe_table_name
        );

        let rows = sqlx::query(&data_query)
            .map(|row: PgRow| {
                let mut json_str = row.get::<String, _>(0);

                // Parse JSON and selectively truncate strings to maintain JSON validity
                if self.max_sample_len > 0 {
                    if let Ok(mut json_val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        truncate_json_strings(&mut json_val, self.max_sample_len);
                        if let Ok(new_str) = serde_json::to_string(&json_val) {
                            json_str = new_str;
                        }
                    }
                }

                json_str
            })
            .fetch_all(self.pool)
            .await
            .unwrap_or_default();

        Ok(rows)
    }
}

/// Recursively traverses a JSON value and truncates string values that exceed `max_len`.
fn truncate_json_strings(val: &mut serde_json::Value, max_len: usize) {
    if max_len == 0 {
        return;
    }
    match val {
        serde_json::Value::String(s) => {
            // Optimization: Skip O(N) characters iteration for naturally short strings
            if s.len() > max_len {
                if s.chars().count() > max_len {
                    let mut truncated: String = s.chars().take(max_len).collect();
                    truncated.push_str("...");
                    *s = truncated;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                truncate_json_strings(item, max_len);
            }
        }
        serde_json::Value::Object(obj) => {
            for v in obj.values_mut() {
                truncate_json_strings(v, max_len);
            }
        }
        _ => {}
    }
}

pub async fn gather(args: &Cli) -> Result<Option<Vec<TableData>>> {
    let config = resolve_config(args)?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.db_url)
        .await
        .context("Failed to connect to database")?;

    let inspector = Inspector::new(
        &pool,
        config.collect_samples,
        config.ignore_tables.clone(),
        config.max_sample_len,
    );

    let table_data = inspector.scan().await?;

    if table_data.is_empty() {
        Ok(None)
    } else {
        Ok(Some(table_data))
    }
}
