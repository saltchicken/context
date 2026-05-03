use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "context",
    author,
    version,
    about = "A universal tool to gather file, codebase, and database context for LLMs"
)]
pub struct Cli {
    /// Path to scan for files (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Include database schema context (reads DB_URL from env)
    #[arg(long)]
    pub sql: bool,

    /// Optional database connection string (triggers --sql automatically)
    #[arg(long)]
    pub db_url: Option<String>,

    /// Use a predefined set of options from presets.toml
    #[arg(long)]
    pub preset: Option<String>,

    /// Show only the directory tree structure (code context)
    #[arg(long)]
    pub tree: bool,

    /// Patterns for files to include for content (e.g., '**/*.rs')
    #[arg(long, num_args = 1..)]
    pub include: Option<Vec<String>>,

    /// Patterns for files to show in tree but without content
    #[arg(long, num_args = 1..)]
    pub include_in_tree: Option<Vec<String>>,

    /// Patterns to exclude (files/directories for code, or table names for SQL)
    #[arg(long, num_args = 1..)]
    pub exclude: Option<Vec<String>>,

    /// Collect sample rows from tables (SQL mode only)
    #[arg(long)]
    pub samples: bool,
}
