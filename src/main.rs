use anyhow::Result;
use clap::Parser;
use context::cli::Cli;
use context::{config, db, format, fs};
use env_logger::Env;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse the unified global CLI options first so we can check for flags like --quiet
    let mut cli = Cli::parse();

    // Evaluate the target path early to load project-specific .env files
    let initial_target = if let Some(config_name) = &cli.config {
        dirs::config_dir().unwrap_or_default().join(config_name)
    } else {
        std::env::current_dir().unwrap_or_default().join(&cli.path)
    };

    let mut target_dir = initial_target.canonicalize().unwrap_or(initial_target);

    if cli.git_root {
        if let Some(git_root) = fs::find_git_root(&target_dir) {
            target_dir = git_root;
        }
    }

    // Try to load .env from the target directory
    dotenvy::from_path(target_dir.join(".env")).ok();

    // Fallback to standard CWD dotenv
    dotenvy::dotenv().ok();

    // Initialize logging (default to warn if quiet is enabled, otherwise info)
    let default_log_level = if cli.quiet { "warn" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(default_log_level)).init();

    // Load config from config.toml
    let user_config = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to load config.toml: {}", e);
            config::UserConfig::default()
        }
    };

    // Apply config defaults to CLI options
    cli.git_root = (cli.git_root || user_config.git_root.unwrap_or(false)) && !cli.no_git_root;
    let resolved_format = cli
        .format
        .clone()
        .unwrap_or(user_config.format.unwrap_or(format::OutputFormat::Xml));

    // Determine what functionality to run
    let run_db = cli.sql || cli.db_url.is_some();
    let has_code_args =
        cli.include.is_some() || cli.include_in_tree.is_some() || cli.preset.is_some() || cli.tree;

    // Run file scanner if:
    // 1. Explicitly requested via code arguments (has_code_args)
    // 2. The user passed --sql (which defaults to pulling both file and sql context)
    // 3. No database flags were provided at all (!run_db)
    // This implies using --db-url exclusively will only pull SQL context.
    let run_fs = has_code_args || cli.sql || !run_db;

    let mut fs_data = None;
    let mut db_data = None;
    let mut context_found = false;

    // 1. Gather File/Code Context
    if run_fs {
        match fs::gather(&cli) {
            Ok(Some(data)) => {
                fs_data = Some(data);
                context_found = true;
            }
            Ok(None) => log::info!("No file content found matching criteria."),
            Err(e) => log::error!("❌ Code scanner error: {:#}", e),
        }
    }

    // 2. Gather SQL Context
    if run_db {
        match db::gather(&cli).await {
            Ok(Some(data)) => {
                db_data = Some(data);
                context_found = true;
            }
            Ok(None) => log::info!("No database schema found."),
            Err(e) => log::error!("❌ SQL scanner error: {:#}", e),
        }
    }

    // Checking if we got nothing out of both processes AND there's no custom prompt
    if !context_found && cli.prompt.is_none() {
        log::warn!("⚠️ No context generated. Try tweaking your arguments.");
    }

    // Build the final output applying the selected format abstraction
    let output = format::format_output(
        &resolved_format,
        cli.prompt.as_deref(),
        fs_data.as_ref(),
        db_data.as_deref(),
    );
    let trimmed_output = output.trim();

    if !trimmed_output.is_empty() {
        // Print the actual generated context to STDOUT
        println!("{}", trimmed_output);

        // Skip printing stats if the user requested the tree view exclusively or enabled quiet mode
        if !cli.tree && !cli.quiet {
            // Calculate statistics
            let lines = trimmed_output.lines().count();
            // A common rough estimate for LLMs: 1 token ≈ 4 chars
            let approx_tokens = trimmed_output.len() / 4;

            // Print stats to STDERR so it doesn't get piped to wl-copy or output files
            eprintln!(
                "\n✅ Context generated: {} lines, ~{} tokens",
                lines, approx_tokens
            );
        }
    }

    Ok(())
}
