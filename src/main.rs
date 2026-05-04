use anyhow::Result;
use clap::Parser;
use context::cli::Cli;
use context::{db, format, fs};
use env_logger::Env;

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables first so log filters (e.g., RUST_LOG) are applied correctly.
    dotenvy::dotenv().ok();

    // Initialize logging (default to info if not set)
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    // Parse the unified global CLI options
    let cli = Cli::parse();

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
        &cli.format,
        cli.prompt.as_deref(),
        fs_data.as_ref(),
        db_data.as_deref(),
    );
    let trimmed_output = output.trim();

    if !trimmed_output.is_empty() {
        // Print the actual generated context to STDOUT
        println!("{}", trimmed_output);

        // Skip printing stats if the user requested the tree view exclusively
        if !cli.tree {
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
