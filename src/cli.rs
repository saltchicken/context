use clap::Parser;
use std::path::PathBuf;

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
    pub path: PathBuf,

    /// Prepend a prompt to the generated context
    #[arg(short, long)]
    pub prompt: Option<String>,

    /// Shortcut to scan a specific folder inside your OS config directory (e.g., --config nvim)
    #[arg(long, help_heading = "File Scanning")]
    pub config: Option<String>,

    /// Use a predefined set of options from presets.toml
    #[arg(long, help_heading = "File Scanning")]
    pub preset: Option<String>,

    /// Show only the directory tree structure (code context)
    #[arg(long, help_heading = "File Scanning")]
    pub tree: bool,

    /// Patterns for files to include for content (e.g., '**/*.rs')
    #[arg(long, num_args = 1.., help_heading = "File Scanning")]
    pub include: Option<Vec<String>>,

    /// Patterns for files to show in tree but without content
    #[arg(long, num_args = 1.., help_heading = "File Scanning")]
    pub include_in_tree: Option<Vec<String>>,

    /// Patterns to exclude (files/directories for code, or table names for SQL)
    #[arg(long, num_args = 1.., help_heading = "File Scanning")]
    pub exclude: Option<Vec<String>>,

    /// Skip files larger than this size in bytes (default: 1MB)
    #[arg(long, default_value = "1048576", help_heading = "File Scanning")]
    pub max_size: u64,

    /// Include database schema context (reads DB_URL from env)
    #[arg(long, help_heading = "Database Options")]
    pub sql: bool,

    /// Optional database connection string (triggers --sql automatically)
    #[arg(long, help_heading = "Database Options")]
    pub db_url: Option<String>,

    /// Collect sample rows from tables (SQL mode only)
    #[arg(long, help_heading = "Database Options")]
    pub samples: bool,
}
