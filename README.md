🧠 ContextA universal tool to gather file, codebase, and database context for Large Language Models (LLMs).context is a powerful, highly-configurable CLI application written in Rust. It is designed to instantly aggregate your project's directory structure, file contents, and database schemas into an optimized format (XML, Markdown, or JSON) ready to be pasted into prompts for models like GPT-4, Claude, or Gemini.✨ Features📁 File & Directory Scanning: Automatically builds structural trees and extracts file contents. Respects .gitignore by default.🗄️ Database Introspection: Native support for PostgreSQL (sqlx). Extracts schemas, column types, primary/foreign keys, and optional sample data.📝 Multiple Output Formats: Choose between <xml> (best for Claude/Gemini), Markdown (best for ChatGPT), or JSON (for programmatic parsing).⚙️ High Configurability: Define global configurations (config.toml), setup reusable scan patterns (presets.toml), or use robust CLI arguments.🚀 Safe & Efficient: Written in Rust with Tokio. Built-in guardrails (max file sizes, max file limits, auto-binary detection) prevent context window blowouts.📦 InstallationTo install context, you will need the Rust toolchain installed.# Clone the repository
git clone [https://github.com/saltchicken/context.git](https://github.com/saltchicken/context.git)
cd context

# Build and install
cargo install --path .
🚀 UsageNavigate to your project directory and run context. The tool outputs directly to stdout, making it incredibly easy to pipe into your clipboard using pbcopy, xclip, or wl-copy.# Basic scan of the current directory (outputs XML by default)
context | pbcopy

# Specify a target directory and output as Markdown
context /path/to/project --format markdown | pbcopy
🎯 Prepending a PromptYou can include instructions for the LLM directly via the CLI:context --prompt "Analyze this Rust project for memory leaks and suggest improvements."

# Or read a prompt from a text file:
context --prompt-file instructions.txt
🗄️ Database ContextContext can connect directly to your PostgreSQL database to provide LLMs with your active schema and table relationships.# Provide the connection string directly
context --db-url "postgres://user:password@localhost:5432/mydb"

# Or use an environment variable / .env file and trigger SQL scan
context --sql

# Include 5 sample rows of data per table for better LLM understanding
context --sql --samples
🛠️ Options & Flagscontext provides granular control over what gets included in the context window.Flag / OptionDescription[PATH]Path to scan for files (defaults to .).--format <FORMAT>Output format: xml, markdown, or json.--prompt <PROMPT>Prepend a text prompt to the generated output.--prompt-file <FILE>Read a prompt from a file to prepend to the generated context.--quiet, -qSuppress stderr output (info/stats logs).--config <NAME>Scan a specific folder inside your OS config directory (e.g., --config nvim).--preset <NAME>Use a predefined set of include/exclude rules from presets.toml.--git-rootIntelligently find and use the root of the git project as the scan path.--no-git-rootDisable using the root of the git project (overrides config and --git-root).--treeShow only the directory tree structure (no file contents).--include <GLOBS>Patterns for files to include content (e.g. **/*.rs).--include-in-tree <GLOBS>Patterns for files to show in the structural tree, but skip contents.--exclude <GLOBS>Patterns to exclude (e.g. tests/**, or table names in SQL mode).--max-size <BYTES>Skip files larger than this size in bytes (default: 1048576 / 1MB).--max-files <COUNT>Stop scanning after this many files (default: 10000).--forceBypass safety checks to force scanning sensitive directories (like / or ~).--sqlInclude database schema context (reads DB_URL from env).--db-url <URL>Optional database connection string (triggers --sql automatically).--samplesCollect sample rows from tables (SQL mode only).--max-sample-len <LEN>Max length for string values in database sample rows (default: 256).Filtering ExampleInclude only Rust source files, exclude the benches directory, and show Cargo.toml in the file tree without extracting its text content:context --include "**/*.rs" --exclude "benches/**" --include-in-tree "Cargo.toml"
⚙️ Configurationcontext creates a configuration folder in your OS's default config directory (e.g., ~/.config/context/ on Linux/macOS or %APPDATA%\context\ on Windows).config.tomlSet your global default preferences.# ~/.config/context/config.toml
format = "markdown"
git_root = true
presets.tomlDefine reusable scanning patterns.# ~/.config/context/presets.toml

[global]
exclude = ["**/docs/**"]

[presets.rust]
include = ["**/*.rs", "Cargo.toml"]
exclude = ["target/**"]

[presets.web]
include = ["**/*.ts", "**/*.tsx", "**/*.css"]
include_in_tree = ["package.json"]
Run using: context --preset rust🛡️ Built-in SafeguardsTo prevent accidentally flooding the LLM or freezing the process:Binary & Non-UTF-8 Detection: Automatically skipped.Large File Limits: Skips files > 1MB by default (--max-size).File Counts: Caps total file processing to 10,000 (--max-files).Dangerous Path Protections: Refuses to indiscriminately scan / or ~ without the --force flag.
