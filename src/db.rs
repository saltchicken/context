use crate::cli::Cli;
use anyhow::{Context, Result};
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{FromRow, Row};
use std::env;
use std::fmt::Write;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub db_url: String,
    pub db_name: String,
    pub collect_samples: bool,
    pub ignore_tables: Vec<String>,
}

pub fn resolve_config(cli: &Cli) -> Result<AppConfig> {
    // Load environment variables from .env file if present
    dotenvy::dotenv().ok();

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
    })
}

#[derive(FromRow, Debug, Clone)]
pub struct ColumnInfo {
    pub column_name: String,
    pub data_type: String,
    pub is_nullable: String,
    pub udt_name: String,
    pub comment: Option<String>,
}

#[derive(FromRow, Debug, Clone)]
pub struct ForeignKeyInfo {
    pub column_name: String,
    pub foreign_table_name: String,
    pub foreign_column_name: String,
}

#[derive(Debug, Clone)]
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
}

impl<'a> Inspector<'a> {
    pub fn new(pool: &'a sqlx::PgPool, collect_samples: bool, ignore_tables: Vec<String>) -> Self {
        Self {
            pool,
            collect_samples,
            ignore_tables,
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
            if col.data_type == "bytea" {
                select_parts.push(format!("'[bytea]'::text AS \"{}\"", col.column_name));
            } else if col.udt_name == "vector" {
                select_parts.push(format!("'[vector]'::text AS \"{}\"", col.column_name));
            } else {
                select_parts.push(format!("\"{}\"", col.column_name));
            }
        }

        if select_parts.is_empty() {
            return Ok(vec![]);
        }

        let select_list = select_parts.join(", ");
        let data_query = format!(
            "SELECT row_to_json(t)::text FROM (SELECT {} FROM \"{}\" LIMIT 5) t",
            select_list, table_name
        );

        let rows = sqlx::query(&data_query)
            .map(|row: PgRow| row.get::<String, _>(0))
            .fetch_all(self.pool)
            .await
            .unwrap_or_default();
        Ok(rows)
    }
}

pub struct OutputGenerator;

impl OutputGenerator {
    pub fn generate_markdown(
        db_name: &str,
        tables: &[TableData],
    ) -> Result<String, std::fmt::Error> {
        let mut output = String::new();

        writeln!(output, "Database Schema for: {}\n", db_name)?;

        for table in tables {
            writeln!(output, "## Table: {}", table.name)?;

            if let Some(comment) = &table.comment {
                writeln!(output, "\n**Description:** {}\n", comment.trim())?;
            } else {
                writeln!(output)?;
            }

            writeln!(output, "| Column | Type | Nullable | Description |")?;
            writeln!(output, "|---|---|---|---|")?;
            for col in &table.columns {
                let clean_comment = col
                    .comment
                    .as_deref()
                    .unwrap_or("")
                    .replace('\n', " ")
                    .replace('|', "\\|");

                writeln!(
                    output,
                    "| {} | {} | {} | {} |",
                    col.column_name, col.data_type, col.is_nullable, clean_comment
                )?;
            }

            if !table.primary_keys.is_empty() {
                writeln!(
                    output,
                    "\n**Primary Key:** {}",
                    table.primary_keys.join(", ")
                )?;
            }

            if !table.foreign_keys.is_empty() {
                writeln!(output, "\n**Foreign Keys:**")?;
                for fk in &table.foreign_keys {
                    writeln!(
                        output,
                        "- `{}.{}` -> `{}.{}`",
                        table.name, fk.column_name, fk.foreign_table_name, fk.foreign_column_name
                    )?;
                }
            }

            if !table.sample_rows.is_empty() {
                writeln!(output, "\n**Sample Data (Top 5 rows):**")?;
                for row in &table.sample_rows {
                    writeln!(output, "- `{}`", row)?;
                }
            }

            writeln!(output, "\n---\n")?;
        }

        Ok(output)
    }
}

pub async fn generate_report(config: &AppConfig) -> Result<String> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.db_url)
        .await
        .context("Failed to connect to database")?;

    let inspector = Inspector::new(&pool, config.collect_samples, config.ignore_tables.clone());
    let table_data = inspector.scan().await?;

    let output = OutputGenerator::generate_markdown(&config.db_name, &table_data)?;
    Ok(output)
}

pub async fn run(args: &Cli) -> Result<Option<String>> {
    let config = resolve_config(args)?;
    let output = generate_report(&config).await?;
    if output.is_empty() {
        Ok(None)
    } else {
        Ok(Some(output))
    }
}
