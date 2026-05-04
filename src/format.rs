use crate::db::TableData;
use crate::fs::{FileData, FsData};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(ValueEnum, Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Xml,
    Markdown,
    Json,
}

#[derive(Serialize)]
struct JsonOutput<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    directory_structure: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<&'a [FileData]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database_schema: Option<&'a [TableData]>,
}

pub fn format_output(
    format: &OutputFormat,
    prompt: Option<&str>,
    fs: Option<&FsData>,
    db: Option<&[TableData]>,
) -> String {
    match format {
        OutputFormat::Xml => format_xml(prompt, fs, db),
        OutputFormat::Markdown => format_markdown(prompt, fs, db),
        OutputFormat::Json => format_json(prompt, fs, db),
    }
}

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn format_xml(prompt: Option<&str>, fs: Option<&FsData>, db: Option<&[TableData]>) -> String {
    let mut out = String::new();

    if let Some(p) = prompt {
        out.push_str(p);
        out.push_str("\n\n");
    }

    if let Some(fs) = fs {
        out.push_str("<directory_structure>\n");
        out.push_str(&fs.tree);
        out.push_str("\n</directory_structure>\n\n");

        if !fs.files.is_empty() {
            out.push_str("<file_contents>\n");
            for f in &fs.files {
                if let Some(err) = &f.error {
                    out.push_str(&format!(
                        "<file path=\"{}\" error=\"true\">\nError: {}\n</file>\n\n",
                        escape_xml(&f.path),
                        escape_xml(err)
                    ));
                } else if let Some(skip) = &f.skipped {
                    out.push_str(&format!(
                        "<file path=\"{}\" skipped=\"true\">\nSkipped: {}\n</file>\n\n",
                        escape_xml(&f.path),
                        escape_xml(skip)
                    ));
                } else if let Some(content) = &f.content {
                    out.push_str(&format!(
                        "<file path=\"{}\">\n{}\n</file>\n\n",
                        escape_xml(&f.path),
                        content
                    ));
                }
            }
            out.push_str("</file_contents>\n\n");
        }
    }

    if let Some(db) = db {
        out.push_str("<database_schema>\n");
        for table in db {
            out.push_str(&format!("<table name=\"{}\">\n", escape_xml(&table.name)));

            if let Some(comment) = &table.comment {
                out.push_str(&format!(
                    "  <description>{}</description>\n",
                    escape_xml(comment.trim())
                ));
            }

            out.push_str("  <columns>\n");
            for col in &table.columns {
                let mut col_tag = format!(
                    "    <column name=\"{}\" type=\"{}\" nullable=\"{}\"",
                    escape_xml(&col.column_name),
                    escape_xml(&col.data_type),
                    escape_xml(&col.is_nullable)
                );
                if let Some(comment) = &col.comment {
                    col_tag.push_str(&format!(" description=\"{}\"", escape_xml(comment.trim())));
                }
                col_tag.push_str(" />\n");
                out.push_str(&col_tag);
            }
            out.push_str("  </columns>\n");

            if !table.primary_keys.is_empty() {
                out.push_str(&format!(
                    "  <primary_key>{}</primary_key>\n",
                    escape_xml(&table.primary_keys.join(", "))
                ));
            }

            if !table.foreign_keys.is_empty() {
                out.push_str("  <foreign_keys>\n");
                for fk in &table.foreign_keys {
                    out.push_str(&format!(
                        "    <foreign_key column=\"{}\" foreign_table=\"{}\" foreign_column=\"{}\" />\n",
                        escape_xml(&fk.column_name),
                        escape_xml(&fk.foreign_table_name),
                        escape_xml(&fk.foreign_column_name)
                    ));
                }
                out.push_str("  </foreign_keys>\n");
            }

            if !table.sample_rows.is_empty() {
                out.push_str("  <sample_data>\n");
                for row in &table.sample_rows {
                    out.push_str(&format!("    <row>{}</row>\n", escape_xml(row)));
                }
                out.push_str("  </sample_data>\n");
            }

            out.push_str("</table>\n\n");
        }
        out.push_str("</database_schema>\n\n");
    }

    out.trim_end().to_string()
}

fn format_markdown(prompt: Option<&str>, fs: Option<&FsData>, db: Option<&[TableData]>) -> String {
    let mut out = String::new();

    if let Some(p) = prompt {
        out.push_str(p);
        out.push_str("\n\n");
    }

    if let Some(fs) = fs {
        out.push_str("## Directory Structure\n\n```\n");
        out.push_str(&fs.tree);
        out.push_str("\n```\n\n");

        if !fs.files.is_empty() {
            out.push_str("## File Contents\n\n");
            for f in &fs.files {
                out.push_str(&format!("### File: `{}`\n\n", f.path));
                if let Some(err) = &f.error {
                    out.push_str(&format!("*Error: {}*\n\n", err));
                } else if let Some(skip) = &f.skipped {
                    out.push_str(&format!("*Skipped: {}*\n\n", skip));
                } else if let Some(content) = &f.content {
                    out.push_str("```\n");
                    out.push_str(content);
                    if !content.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n\n");
                }
            }
        }
    }

    if let Some(db) = db {
        out.push_str("## Database Schema\n\n");
        for table in db {
            out.push_str(&format!("### Table: `{}`\n\n", table.name));
            if let Some(comment) = &table.comment {
                out.push_str(&format!("*{}*\n\n", comment));
            }

            out.push_str("| Column | Type | Nullable | Description |\n");
            out.push_str("| --- | --- | --- | --- |\n");
            for col in &table.columns {
                let desc = col.comment.as_deref().unwrap_or("").replace('\n', " ");
                out.push_str(&format!(
                    "| `{}` | `{}` | `{}` | {} |\n",
                    col.column_name, col.data_type, col.is_nullable, desc
                ));
            }
            out.push('\n');

            if !table.primary_keys.is_empty() {
                out.push_str(&format!(
                    "**Primary Keys**: `{}`\n\n",
                    table.primary_keys.join(", ")
                ));
            }

            if !table.foreign_keys.is_empty() {
                out.push_str("**Foreign Keys**:\n\n");
                for fk in &table.foreign_keys {
                    out.push_str(&format!(
                        "* `{}` -> `{}`.`{}`\n",
                        fk.column_name, fk.foreign_table_name, fk.foreign_column_name
                    ));
                }
                out.push('\n');
            }

            if !table.sample_rows.is_empty() {
                out.push_str("**Sample Data**:\n\n```json\n");
                for row in &table.sample_rows {
                    out.push_str(row);
                    out.push('\n');
                }
                out.push_str("```\n\n");
            }
        }
    }

    out.trim_end().to_string()
}

fn format_json(prompt: Option<&str>, fs: Option<&FsData>, db: Option<&[TableData]>) -> String {
    let output = JsonOutput {
        prompt,
        directory_structure: fs.map(|f| f.tree.as_str()),
        files: fs.map(|f| f.files.as_slice()),
        database_schema: db,
    };
    serde_json::to_string_pretty(&output)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize to JSON: {}\"}}", e))
}
