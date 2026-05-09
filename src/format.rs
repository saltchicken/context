use crate::db::TableData;
use crate::fs::FsData;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt::Write;

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
    instructions: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    projects: Option<&'a [FsData]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database_schema: Option<&'a [TableData]>,
}

pub fn format_output(
    format: &OutputFormat,
    instructions: Option<&str>,
    prompt: Option<&str>,
    fs: Option<&[FsData]>,
    db: Option<&[TableData]>,
) -> String {
    match format {
        OutputFormat::Xml => format_xml(instructions, prompt, fs, db),
        OutputFormat::Markdown => format_markdown(instructions, prompt, fs, db),
        OutputFormat::Json => format_json(instructions, prompt, fs, db),
    }
}

/// Escapes XML characters in a single pass.
/// Returns `Cow::Borrowed` (no allocation) if no escaping was needed.
fn escape_xml(input: &str) -> Cow<'_, str> {
    let mut last_end = 0;
    let mut escaped: Option<String> = None;

    for (i, c) in input.char_indices() {
        let escape = match c {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&apos;",
            _ => continue,
        };

        if escaped.is_none() {
            escaped = Some(String::with_capacity(input.len() + 16)); // Slight overallocation for escapes
        }

        let s = escaped.as_mut().unwrap();
        s.push_str(&input[last_end..i]);
        s.push_str(escape);
        last_end = i + c.len_utf8();
    }

    if let Some(mut s) = escaped {
        s.push_str(&input[last_end..]);
        Cow::Owned(s)
    } else {
        Cow::Borrowed(input)
    }
}

/// Helper to estimate the required buffer capacity to avoid constant re-allocations
fn estimate_capacity(
    instructions: Option<&str>,
    prompt: Option<&str>,
    fs: Option<&[FsData]>,
    db: Option<&[TableData]>,
) -> usize {
    let mut cap = 0;
    if let Some(i) = instructions {
        cap += i.len() + 32; // Added capacity for wrapper tags or headers
    }
    if let Some(p) = prompt {
        cap += p.len() + 32; // Added capacity for wrapper tags or headers
    }
    if let Some(fs_list) = fs {
        for fs_item in fs_list {
            cap += fs_item.project_name.len() + 64; // Capacity for project tags
            cap += fs_item.tree.len() + 64;
            for f in &fs_item.files {
                cap += f.path.len()
                    + f.content.as_ref().map_or(0, |c| c.len())
                    + f.error.as_ref().map_or(0, |e| e.len())
                    + f.skipped.as_ref().map_or(0, |s| s.len())
                    + 128; // Wrapper tags overhead
            }
        }
    }
    if let Some(db) = db {
        cap += db.len() * 1024; // Rough average estimate per table schema
    }
    cap
}

fn format_xml(
    instructions: Option<&str>,
    prompt: Option<&str>,
    fs: Option<&[FsData]>,
    db: Option<&[TableData]>,
) -> String {
    let capacity = estimate_capacity(instructions, prompt, fs, db);
    let mut out = String::with_capacity(capacity);

    if let Some(i) = instructions {
        out.push_str("<instructions>\n");
        out.push_str(i);
        out.push_str("\n</instructions>\n\n");
    }

    if let Some(p) = prompt {
        out.push_str("<prompt>\n");
        out.push_str(p);
        out.push_str("\n</prompt>\n\n");
    }

    if let Some(fs_list) = fs {
        for fs_item in fs_list {
            let _ = write!(out, "<project name=\"{}\">\n", escape_xml(&fs_item.project_name));

            out.push_str("<directory_structure>\n");
            out.push_str(&fs_item.tree);
            out.push_str("\n</directory_structure>\n\n");

            if !fs_item.files.is_empty() {
                out.push_str("<file_contents>\n");
                for f in &fs_item.files {
                    if let Some(err) = &f.error {
                        let _ = write!(
                            out,
                            "<file path=\"{}\" error=\"true\">\nError: {}\n</file>\n\n",
                            escape_xml(&f.path),
                            escape_xml(err)
                        );
                    } else if let Some(skip) = &f.skipped {
                        let _ = write!(
                            out,
                            "<file path=\"{}\" skipped=\"true\">\nSkipped: {}\n</file>\n\n",
                            escape_xml(&f.path),
                            escape_xml(skip)
                        );
                    } else if let Some(content) = &f.content {
                        let _ = write!(
                            out,
                            "<file path=\"{}\">\n{}\n</file>\n\n",
                            escape_xml(&f.path),
                            content // Not escaping raw content to match original logic and LLM prompt patterns
                        );
                    }
                }
                out.push_str("</file_contents>\n\n");
            }

            out.push_str("</project>\n\n");
        }
    }

    if let Some(db) = db {
        out.push_str("<database_schema>\n");
        for table in db {
            let _ = write!(out, "<table name=\"{}\">\n", escape_xml(&table.name));

            if let Some(comment) = &table.comment {
                let _ = write!(
                    out,
                    "  <description>{}</description>\n",
                    escape_xml(comment.trim())
                );
            }

            out.push_str("  <columns>\n");
            for col in &table.columns {
                let _ = write!(
                    out,
                    "    <column name=\"{}\" type=\"{}\" nullable=\"{}\"",
                    escape_xml(&col.column_name),
                    escape_xml(&col.data_type),
                    escape_xml(&col.is_nullable)
                );
                if let Some(comment) = &col.comment {
                    let _ = write!(out, " description=\"{}\"", escape_xml(comment.trim()));
                }
                out.push_str(" />\n");
            }
            out.push_str("  </columns>\n");

            if !table.primary_keys.is_empty() {
                let _ = write!(
                    out,
                    "  <primary_key>{}</primary_key>\n",
                    escape_xml(&table.primary_keys.join(", "))
                );
            }

            if !table.foreign_keys.is_empty() {
                out.push_str("  <foreign_keys>\n");
                for fk in &table.foreign_keys {
                    let _ = write!(
                        out,
                        "    <foreign_key column=\"{}\" foreign_table=\"{}\" foreign_column=\"{}\" />\n",
                        escape_xml(&fk.column_name),
                        escape_xml(&fk.foreign_table_name),
                        escape_xml(&fk.foreign_column_name)
                    );
                }
                out.push_str("  </foreign_keys>\n");
            }

            if !table.sample_rows.is_empty() {
                out.push_str("  <sample_data>\n");
                for row in &table.sample_rows {
                    let _ = write!(out, "    <row>{}</row>\n", escape_xml(row));
                }
                out.push_str("  </sample_data>\n");
            }

            out.push_str("</table>\n\n");
        }
        out.push_str("</database_schema>\n\n");
    }

    out.trim_end().to_string()
}

fn format_markdown(
    instructions: Option<&str>,
    prompt: Option<&str>,
    fs: Option<&[FsData]>,
    db: Option<&[TableData]>,
) -> String {
    let capacity = estimate_capacity(instructions, prompt, fs, db);
    let mut out = String::with_capacity(capacity);

    if let Some(i) = instructions {
        out.push_str("## Instructions\n\n");
        out.push_str(i);
        out.push_str("\n\n");
    }

    if let Some(p) = prompt {
        out.push_str("## Prompt\n\n");
        out.push_str(p);
        out.push_str("\n\n");
    }

    if let Some(fs_list) = fs {
        for fs_item in fs_list {
            let _ = write!(out, "## Project: `{}`\n\n", fs_item.project_name);

            out.push_str("### Directory Structure\n\n```\n");
            out.push_str(&fs_item.tree);
            out.push_str("\n```\n\n");

            if !fs_item.files.is_empty() {
                out.push_str("### File Contents\n\n");
                for f in &fs_item.files {
                    let _ = write!(out, "#### File: `{}`\n\n", f.path);

                    if let Some(err) = &f.error {
                        let _ = write!(out, "*Error: {}*\n\n", err);
                    } else if let Some(skip) = &f.skipped {
                        let _ = write!(out, "*Skipped: {}*\n\n", skip);
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
    }

    if let Some(db) = db {
        out.push_str("## Database Schema\n\n");
        for table in db {
            let _ = write!(out, "### Table: `{}`\n\n", table.name);

            if let Some(comment) = &table.comment {
                let _ = write!(out, "*{}*\n\n", comment);
            }

            out.push_str("| Column | Type | Nullable | Description |\n");
            out.push_str("| --- | --- | --- | --- |\n");
            for col in &table.columns {
                let desc = col.comment.as_deref().unwrap_or("").replace('\n', " ");
                let _ = write!(
                    out,
                    "| `{}` | `{}` | `{}` | {} |\n",
                    col.column_name, col.data_type, col.is_nullable, desc
                );
            }
            out.push('\n');

            if !table.primary_keys.is_empty() {
                let _ = write!(
                    out,
                    "**Primary Keys**: `{}`\n\n",
                    table.primary_keys.join(", ")
                );
            }

            if !table.foreign_keys.is_empty() {
                out.push_str("**Foreign Keys**:\n\n");
                for fk in &table.foreign_keys {
                    let _ = write!(
                        out,
                        "* `{}` -> `{}`.`{}`\n",
                        fk.column_name, fk.foreign_table_name, fk.foreign_column_name
                    );
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

fn format_json(
    instructions: Option<&str>,
    prompt: Option<&str>,
    fs: Option<&[FsData]>,
    db: Option<&[TableData]>,
) -> String {
    let output = JsonOutput {
        instructions,
        prompt,
        projects: fs,
        database_schema: db,
    };
    serde_json::to_string_pretty(&output)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize to JSON: {}\"}}", e))
}
