use clap::Parser;
use context::cli::Cli;
use context::{code, sql};
use env_logger::Env;

#[tokio::main]
async fn main() {
    // Initialize logging (default to info if not set)
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    // Parse the unified global CLI options
    let cli = Cli::parse();

    // Determine what functionality to run
    let run_sql = cli.sql || cli.db_url.is_some();
    let has_code_args = cli.include.is_some() || cli.include_in_tree.is_some() || cli.preset.is_some() || cli.tree;
    let run_code = !run_sql || has_code_args; // Run code by default if no SQL, or if explicitly requested

    let mut output = String::new();

    // 1. Gather File/Code Context
    if run_code {
        match code::run(&cli) {
            Ok(Some(code_out)) => output.push_str(&code_out),
            Ok(None) => log::info!("No file content found matching criteria."),
            Err(e) => log::error!("❌ Code scanner error: {:?}", e),
        }
    }

    // 2. Gather SQL Context
    if run_sql {
        match sql::run(&cli).await {
            Ok(Some(sql_out)) => {
                if !output.is_empty() {
                    output.push_str("\n\n");
                }
                output.push_str(&sql_out);
            }
            Ok(None) => log::info!("No database schema found."),
            Err(e) => log::error!("❌ SQL scanner error: {:?}", e),
        }
    }

    if output.trim().is_empty() {
        log::warn!("⚠️ No context generated. Try tweaking your arguments.");
    } else {
        println!("{}", output.trim());
    }
}
