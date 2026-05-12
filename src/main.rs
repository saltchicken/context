use anyhow::{Context, Result};
use clap::Parser;
use context::cli::Cli;
use context::{config, db, format, fs};
use env_logger::Env;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse the unified global CLI options first so we can check for flags like --quiet
    let mut cli = Cli::parse();

    // Initialize logging (default to warn if quiet is enabled, otherwise info)
    let default_log_level = if cli.quiet { "warn" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(default_log_level)).init();

    // Load prompt file if specified
    let mut final_prompt = cli.prompt.clone();
    if let Some(prompt_path) = &cli.prompt_file {
        let file_content = std::fs::read_to_string(prompt_path)
            .with_context(|| format!("Failed to read prompt file: {:?}", prompt_path))?;

        if let Some(existing) = &mut final_prompt {
            existing.push_str("\n\n");
            existing.push_str(&file_content);
        } else {
            final_prompt = Some(file_content);
        }
    }

    // Load config from config.toml
    let user_config = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to load config.toml: {}", e);
            config::UserConfig::default()
        }
    };

    // Determine instruction preset to use
    let mut final_instructions: Option<String> = None;

    if let Some(cli_inst) = &cli.instructions {
        // User provided an instruction string via CLI
        let mut matched_preset = false;

        if let Some(inst_cfg) = &user_config.instructions {
            if let config::InstructionsConfig::Map(m) = inst_cfg {
                if let Some(preset_val) = m.get(cli_inst) {
                    final_instructions = Some(preset_val.clone());
                    matched_preset = true;
                }
            }
        }

        // If it didn't match a preset in the map, treat the CLI argument as the literal instruction string
        if !matched_preset {
            final_instructions = Some(cli_inst.clone());
        }
    } else {
        // No CLI instruction provided. Fall back to config "default" if available.
        if let Some(inst_cfg) = &user_config.instructions {
            match inst_cfg {
                config::InstructionsConfig::Single(s) => {
                    final_instructions = Some(s.clone());
                }
                config::InstructionsConfig::Map(m) => {
                    if let Some(s) = m.get("default") {
                        final_instructions = Some(s.clone());
                    }
                }
            }
        }
    }

    // Apply config defaults to CLI options
    cli.git_root = (cli.git_root || user_config.git_root.unwrap_or(false)) && !cli.no_git_root;

    // Resolve target directories after merging config and CLI arguments
    let target_dirs = fs::resolve_target_dirs(&cli)?;

    // Try to load .env from the first resolved directory as a primary guess
    if let Some(first_dir) = target_dirs.first() {
        dotenvy::from_path(first_dir.join(".env")).ok();
    }

    // Fallback to standard CWD dotenv
    dotenvy::dotenv().ok();

    // Determine what functionality to run
    let run_db = cli.sql || cli.db_url.is_some();
    let has_code_args =
        cli.include.is_some() || cli.include_in_tree.is_some() || cli.preset.is_some() || cli.tree;

    // Run file scanner if explicitly requested or if no database flags were provided at all
    let run_fs = has_code_args || cli.sql || !run_db;

    let mut fs_data = None;
    let mut db_data = None;
    let mut context_found = false;

    // 1. Gather File/Code Context across multiple directories
    if run_fs {
        match fs::gather_multiple(&target_dirs, &cli) {
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
    if !context_found && final_prompt.is_none() {
        log::warn!("⚠️ No context generated. Try tweaking your arguments.");
    }

    // Build the final output natively in the unified hybrid format
    let output = format::format_output(
        final_instructions
            .as_deref()
            .filter(|s| !s.trim().is_empty()),
        final_prompt.as_deref().filter(|s| !s.trim().is_empty()),
        fs_data.as_deref(), // Using deref to access the slice of projects
        db_data.as_deref(),
    );
    let trimmed_output = output.trim();

    if !trimmed_output.is_empty() {
        // Print the actual generated context to STDOUT
        println!("{}", trimmed_output);

        // Skip printing stats if the user requested the tree view exclusively or enabled quiet mode
        if !cli.tree && !cli.quiet {
            let lines = trimmed_output.lines().count();
            let approx_tokens = trimmed_output.len() / 4;

            eprintln!(
                "\n✅ Context generated: {} lines, ~{} tokens",
                lines, approx_tokens
            );
        }
    }

    Ok(())
}
