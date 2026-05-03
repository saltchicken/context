use crate::cli::Cli;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use pathdiff::diff_paths;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Represents the final configuration after merging presets and CLI args.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_in_tree: Vec<String>,
    pub tree_only_output: bool,
    pub max_size: u64,
}

/// Represents a single file discovered during the scan.
#[derive(Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub relative_path: String,
    pub depth: usize,
    pub is_dir: bool,
    pub include_content: bool,
    pub exceeds_size: bool,
}

fn load_presets_file() -> Result<PresetsFile> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    let config_path = config_dir.join("context").join("presets.toml");

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
    max_size: u64,
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
        max_size,
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
        args.max_size,
    )
}

pub struct OutputGenerator;

impl OutputGenerator {
    pub fn generate_tree(entries: &[FileEntry]) -> String {
        let mut output = String::new();

        for entry in entries {
            let indent = "    ".repeat(entry.depth.saturating_sub(1));
            let name = entry.path.file_name().unwrap_or_default().to_string_lossy();

            let marker = if entry.is_dir { "/" } else { "" };
            output.push_str(&format!("{}{}{}\n", indent, name, marker));
        }

        output.trim_end().to_string()
    }

    pub fn generate_content(entries: &[FileEntry]) -> String {
        let mut blocks = Vec::new();

        for entry in entries {
            if entry.include_content {
                if entry.exceeds_size {
                    blocks.push(format!(
                        "<file path=\"{}\" error=\"true\">\nError: File exceeds maximum size limit.\n</file>",
                        entry.relative_path
                    ));
                    continue;
                }

                match fs::read_to_string(&entry.path) {
                    Ok(content) => {
                        blocks.push(format!(
                            "<file path=\"{}\">\n{}\n</file>",
                            entry.relative_path, content
                        ));
                    }
                    Err(e) => {
                        blocks.push(format!(
                            "<file path=\"{}\" error=\"true\">\nError reading file: {}\n</file>",
                            entry.relative_path, e
                        ));
                    }
                }
            }
        }

        blocks.join("\n\n")
    }

    pub fn format_full_output(tree: &str, content: &str) -> String {
        let mut out = String::from("<directory_structure>\n");
        out.push_str(tree);
        out.push_str("\n</directory_structure>");

        if !content.is_empty() {
            out.push_str("\n\n<file_contents>\n");
            out.push_str(content);
            out.push_str("\n</file_contents>");
        }

        out
    }
}

pub struct Scanner {
    root: PathBuf,
    include_set: GlobSet,
    exclude_set: GlobSet,
    tree_only_set: GlobSet,
    max_size: u64,
}

impl Scanner {
    pub fn new(root: PathBuf, config: &RuntimeConfig) -> Result<Self> {
        Ok(Self {
            root,
            include_set: build_globset(&config.include)?,
            exclude_set: build_globset(&config.exclude)?,
            tree_only_set: build_globset(&config.include_in_tree)?,
            max_size: config.max_size,
        })
    }

    pub fn scan(&self) -> Vec<FileEntry> {
        let mut entries = Vec::new();

        let walker = WalkBuilder::new(&self.root)
            .hidden(false)
            .git_ignore(true)
            .build();

        for result in walker {
            match result {
                Ok(entry) => {
                    if let Some(processed) = self.process_entry(entry.path()) {
                        entries.push(processed);
                    }
                }
                Err(err) => log::warn!("Error walking entry: {}", err),
            }
        }

        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    fn process_entry(&self, path: &Path) -> Option<FileEntry> {
        if path == self.root {
            return None;
        }

        if path.components().any(|c| c.as_os_str() == ".git") {
            return None;
        }

        let relative = diff_paths(path, &self.root)?;
        let relative_str = relative.to_string_lossy();

        if self.exclude_set.is_match(&relative) {
            return None;
        }

        let metadata = path.metadata().ok()?;
        let is_dir = metadata.is_dir();
        let size = metadata.len();

        let matches_include = self.include_set.is_match(&relative);
        let matches_tree = self.tree_only_set.is_match(&relative);

        if !is_dir && !matches_include && !matches_tree {
            return None;
        }

        let depth = relative.components().count();

        Some(FileEntry {
            path: path.to_path_buf(),
            relative_path: relative_str.to_string(),
            depth,
            is_dir,
            include_content: !is_dir && matches_include && !matches_tree,
            exceeds_size: size > self.max_size,
        })
    }
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        builder.add(Glob::new(pat).context(format!("Invalid glob pattern: {}", pat))?);
    }
    Ok(builder.build()?)
}

pub fn generate(config: RuntimeConfig, root: PathBuf) -> Result<String> {
    let scanner = Scanner::new(root, &config)?;
    let entries = scanner.scan();

    if entries.is_empty() {
        return Ok(String::new());
    }

    let tree_str = OutputGenerator::generate_tree(&entries);

    let final_output = if config.tree_only_output {
        format!(
            "<directory_structure>\n{}\n</directory_structure>",
            tree_str
        )
    } else {
        let content_str = OutputGenerator::generate_content(&entries);
        OutputGenerator::format_full_output(&tree_str, &content_str)
    };

    Ok(final_output)
}

pub fn run(args: &Cli) -> Result<Option<String>> {
    let target_dir = if let Some(config_name) = &args.config {
        dirs::config_dir()
            .context("Could not determine config directory")?
            .join(config_name)
    } else {
        let current_dir = env::current_dir().context("Failed to get current directory")?;
        current_dir.join(&args.path)
    };

    let target_dir = target_dir.canonicalize().unwrap_or(target_dir);

    if !target_dir.exists() {
        anyhow::bail!("Target directory does not exist: {:?}", target_dir);
    }

    let project_name = target_dir.file_name().and_then(|n| n.to_str());
    let config = resolve_config(args, project_name)?;

    let output = generate(config, target_dir)?;

    if output.is_empty() {
        Ok(None)
    } else {
        Ok(Some(output))
    }
}
