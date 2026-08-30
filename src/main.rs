use anyhow::{Context, Result};
use clap::Parser;
use context::cli::Cli;
use context::{config, format, fs};
use env_logger::Env;

fn main() -> Result<()> {
    // Parse the unified global CLI options first so we can check for flags like --quiet
    let mut cli = Cli::parse();

    // Initialize logging (default to warn if quiet is enabled, otherwise info)
    let default_log_level = if cli.quiet { "warn" } else { "info" };
    env_logger::Builder::from_env(Env::default().default_filter_or(default_log_level)).init();

    // Load prompt file if specified
    let mut final_prompt = cli.prompt.clone();
    if let Some(prompt_path) = &cli.prompt_file {
        let expanded_prompt_path = fs::expand_tilde(prompt_path);
        let file_content = std::fs::read_to_string(&expanded_prompt_path)
            .with_context(|| format!("Failed to read prompt file: {:?}", expanded_prompt_path))?;

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
    cli.no_git_root = cli.no_git_root || !user_config.git_root.unwrap_or(true);

    // Resolve target directories after merging config and CLI arguments
    let target_dirs = fs::resolve_target_dirs(&cli)?;

    // Use the first target directory's name as the fallback preset for standalone files
    let fallback_preset = target_dirs
        .first()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    let mut all_fs_data = Vec::new();
    let mut context_found = false;

    // 1. Gather File/Code Context across multiple directories
    match fs::gather_multiple(&target_dirs, &cli) {
        Ok(Some(mut data)) => {
            all_fs_data.append(&mut data);
            context_found = true;
        }
        Ok(None) => log::info!("No file content found matching criteria in directories."),
        Err(e) => anyhow::bail!("❌ Code scanner error: {:#}", e),
    }

    // 2. Gather explicitly included standalone files
    let extra_files = fs::resolve_extra_files(&cli, fallback_preset)?;
    if !extra_files.is_empty() {
        match fs::gather_extra_files(&extra_files) {
            Ok(Some(data)) => {
                all_fs_data.push(data);
                context_found = true;
            }
            Ok(None) => {}
            Err(e) => log::warn!("Failed to read extra files: {}", e),
        }
    }

    // Checking if we got nothing out of the process AND there's no custom prompt
    if !context_found && final_prompt.is_none() {
        log::warn!("⚠️ No context generated. Try tweaking your arguments.");
    }

    // Build the final output natively in the format
    let fs_data_ref = if all_fs_data.is_empty() { None } else { Some(all_fs_data.as_slice()) };
    
    let output = format::format_output(
        final_instructions
            .as_deref()
            .filter(|s| !s.trim().is_empty()),
        final_prompt.as_deref().filter(|s| !s.trim().is_empty()),
        fs_data_ref, 
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
