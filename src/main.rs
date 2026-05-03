use clap::Parser;
use context::cli::Cli;
use context::{db, fs};
use env_logger::Env;

#[tokio::main]
async fn main() {
    // Initialize logging (default to info if not set)
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    // Parse the unified global CLI options
    let cli = Cli::parse();

    // Determine what functionality to run
    let run_db = cli.sql || cli.db_url.is_some();
    let has_code_args =
        cli.include.is_some() || cli.include_in_tree.is_some() || cli.preset.is_some() || cli.tree;
    let run_fs = !run_db || has_code_args; // Run code by default if no SQL, or if explicitly requested

    let mut output = String::new();
    let mut context_found = false;

    // Prepend prompt if provided
    if let Some(prompt) = &cli.prompt {
        output.push_str(prompt);
        output.push_str("\n\n");
    }

    // 1. Gather File/Code Context
    if run_fs {
        match fs::run(&cli) {
            Ok(Some(code_out)) => {
                output.push_str(&code_out);
                context_found = true;
            }
            Ok(None) => log::info!("No file content found matching criteria."),
            Err(e) => log::error!("❌ Code scanner error: {:?}", e),
        }
    }

    // 2. Gather SQL Context
    if run_db {
        match db::run(&cli).await {
            Ok(Some(sql_out)) => {
                if !output.is_empty() {
                    output.push_str("\n\n");
                }
                output.push_str(&sql_out);
                context_found = true;
            }
            Ok(None) => log::info!("No database schema found."),
            Err(e) => log::error!("❌ SQL scanner error: {:?}", e),
        }
    }

    if !context_found {
        log::warn!("⚠️ No context generated. Try tweaking your arguments.");
    }

    let trimmed_output = output.trim();
    if !trimmed_output.is_empty() {
        // Print the actual generated context to STDOUT
        println!("{}", trimmed_output);

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
