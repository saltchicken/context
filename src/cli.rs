use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "context",
    author,
    version,
    about = "A universal tool to gather file and codebase context for LLMs"
)]
pub struct Cli {
    /// Paths to scan for files (defaults to current directory)
    #[arg(default_value = ".", num_args = 1..)]
    pub paths: Vec<PathBuf>,

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

    /// Use a predefined set of options from presets.toml
    #[arg(long, help_heading = "File Scanning")]
    pub preset: Option<String>,

    /// Disable using the root of the git project (forces using the current directory)
    #[arg(long, visible_alias = "cwd", help_heading = "File Scanning")]
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

    /// Patterns to exclude (files/directories for code)
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

    /// PostgreSQL database connection URL to extract schema context
    #[arg(long, help_heading = "Database Scanning")]
    pub db: Option<String>,
}
