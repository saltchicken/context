use crate::format::OutputFormat;
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
    /// Paths to scan for files (defaults to current directory)
    #[arg(default_value = ".", num_args = 1..)]
    pub paths: Vec<PathBuf>,

    /// Choose the output format (overrides config.toml)
    #[arg(long, value_enum, help_heading = "Output Options")]
    pub format: Option<OutputFormat>,

    /// Suppress stderr output (e.g., stats and info logs)
    #[arg(short, long, help_heading = "Output Options")]
    pub quiet: bool,

    /// Prepend a prompt to the generated context
    #[arg(short, long, help_heading = "Output Options")]
    pub prompt: Option<String>,

    /// Read a prompt from a file to prepend to the generated context
    #[arg(long, help_heading = "Output Options")]
    pub prompt_file: Option<PathBuf>,

    /// Choose an instruction preset from config to prepend (defaults to "default")
    #[arg(short = 'i', long, help_heading = "Output Options")]
    pub instructions: Option<String>,

    /// Shortcut to scan a specific folder inside your OS config directory (e.g., --config nvim)
    #[arg(long, help_heading = "File Scanning")]
    pub config: Option<String>,

    /// Use a predefined set of options from presets.toml
    #[arg(long, help_heading = "File Scanning")]
    pub preset: Option<String>,

    /// Intelligently find and use the root of the git project as the scan path
    #[arg(long, help_heading = "File Scanning")]
    pub git_root: bool,

    /// Disable using the root of the git project (overrides config.toml and --git-root)
    #[arg(long, overrides_with = "git_root", help_heading = "File Scanning")]
    pub no_git_root: bool,

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

    /// Stop scanning after this many files to prevent out-of-memory issues (default: 10000)
    #[arg(long, default_value = "10000", help_heading = "File Scanning")]
    pub max_files: usize,

    /// Force scanning of sensitive directories (like home or root)
    #[arg(long, help_heading = "File Scanning")]
    pub force: bool,

    /// Output absolute paths instead of relative paths in the tree and file blocks
    #[arg(long, help_heading = "File Scanning")]
    pub absolute_paths: bool,

    /// Include database schema context (reads DB_URL from env)
    #[arg(long, help_heading = "Database Options")]
    pub sql: bool,

    /// Optional database connection string (triggers --sql automatically)
    #[arg(long, help_heading = "Database Options")]
    pub db_url: Option<String>,

    /// Collect sample rows from tables (SQL mode only)
    #[arg(long, help_heading = "Database Options")]
    pub samples: bool,

    /// Max length for string values in database sample rows (0 to disable)
    #[arg(long, default_value = "256", help_heading = "Database Options")]
    pub max_sample_len: usize,
}
