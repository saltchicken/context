use anyhow::{Context, Result};
use postgres::{Client, NoTls};

pub struct DbData {
    pub schema: String,
}

pub fn gather(db_url: &str) -> Result<DbData> {
    let mut client = Client::connect(db_url, NoTls)
        .context("Failed to connect to PostgreSQL database")?;

    let rows = client
        .query(
            "SELECT 
                cols.table_name, 
                cols.column_name, 
                cols.data_type,
                (
                    SELECT pg_catalog.col_description(c.oid, cols.ordinal_position::int)
                    FROM pg_catalog.pg_class c
                    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                    WHERE c.relname = cols.table_name AND n.nspname = cols.table_schema
                ) as column_comment
             FROM information_schema.columns cols 
             WHERE cols.table_schema = 'public' 
             ORDER BY cols.table_name, cols.ordinal_position",
            &[],
        )
        .context("Failed to query database schema")?;

    let mut schema = String::new();
    let mut current_table = String::new();

    for row in rows {
        let table_name: String = row.get(0);
        let column_name: String = row.get(1);
        let data_type: String = row.get(2);
        let column_comment: Option<String> = row.get(3);

        if table_name != current_table {
            if !current_table.is_empty() {
                schema.push('\n');
            }
            schema.push_str(&format!("Table: {}\n", table_name));
            current_table = table_name;
        }
        
        if let Some(comment) = column_comment {
            schema.push_str(&format!("  - {}: {} /* {} */\n", column_name, data_type, comment));
        } else {
            schema.push_str(&format!("  - {}: {}\n", column_name, data_type));
        }
    }

    Ok(DbData { schema })
}