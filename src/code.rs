use crate::cli::Cli;
use crate::common::RuntimeConfig;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;

#[derive(Deserialize, Debug, Default)]
struct PresetsFile {
    #[serde(default)]
    global: PresetConfig,
    #[serde(flatten)]
    presets: HashMap<String, PresetConfig>,
}

#[derive(Deserialize, Debug, Clone, Default)]
struct PresetConfig {
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    include_in_tree: Option<Vec<String>>,
}

fn load_presets_file() -> Result<PresetsFile> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let config_path = home.join(".config").join("context").join("presets.toml");

    if !config_path.exists() {
        return Ok(PresetsFile::default());
    }

    let content = fs::read_to_string(&config_path)
        .context(format!("Failed to read config at {:?}", config_path))?;

    let parsed: PresetsFile = toml::from_str(&content).context("Failed to parse presets.toml")?;
    Ok(parsed)
}

fn combine_lists(lists: Vec<Option<Vec<String>>>) -> Vec<String> {
    let mut combined = Vec::new();
    for list in lists.into_iter().flatten() {
        combined.extend(list);
    }
    // Deduplicate while keeping order
    let mut seen = std::collections::HashSet::new();
    combined.retain(|item| seen.insert(item.clone()));
    combined
}

pub fn build_config(
    preset_name: Option<&str>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    include_in_tree: Option<Vec<String>>,
    tree_only: bool,
) -> Result<RuntimeConfig> {
    let presets_file = load_presets_file()?;

    let global = presets_file.global;
    let preset = preset_name
        .and_then(|k| presets_file.presets.get(k))
        .cloned()
        .unwrap_or_default();

    let mut final_include = combine_lists(vec![global.include, preset.include, include]);

    // Provide an intelligent "universal" default if no includes were specified
    if final_include.is_empty() {
        final_include = vec!["**".into()];
    }

    // Always exclude certain problematic binaries universally
    let hardcoded_excludes = vec![
        "**/.git/**".into(),
        "**/*.db".into(),
        "**/*.sqlite".into(),
        "**/*.png".into(),
        "**/*.jpg".into(),
        "**/*.so".into(),
        "**/*.zip".into(),
        "**/*.tar.gz".into(),
    ];

    let final_exclude = combine_lists(vec![
        Some(hardcoded_excludes),
        global.exclude,
        preset.exclude,
        exclude,
    ]);

    let final_include_in_tree = combine_lists(vec![
        global.include_in_tree,
        preset.include_in_tree,
        include_in_tree,
    ]);

    Ok(RuntimeConfig {
        include: final_include,
        exclude: final_exclude,
        include_in_tree: final_include_in_tree,
        tree_only_output: tree_only,
    })
}

pub fn resolve_config(args: &Cli, fallback_preset: Option<&str>) -> Result<RuntimeConfig> {
    let selected_preset = args.preset.as_deref().or(fallback_preset);

    build_config(
        selected_preset,
        args.include.clone(),
        args.exclude.clone(),
        args.include_in_tree.clone(),
        args.tree,
    )
}

pub fn run(args: &Cli) -> Result<Option<String>> {
    let current_dir = env::current_dir().context("Failed to get current directory")?;

    let target_dir = current_dir.join(&args.path);
    let target_dir = target_dir.canonicalize().unwrap_or(target_dir);

    if !target_dir.exists() {
        anyhow::bail!("Target directory does not exist: {:?}", target_dir);
    }

    let project_name = target_dir.file_name().and_then(|n| n.to_str());
    let config = resolve_config(args, project_name)?;

    let output = crate::common::generate(config, target_dir)?;

    if output.is_empty() {
        Ok(None)
    } else {
        Ok(Some(output))
    }
}
