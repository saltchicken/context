use crate::cli::Cli;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use pathdiff::diff_paths;
use serde::{Deserialize, Serialize};
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
    pub max_files: usize,
    pub force: bool,
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

#[derive(Debug, Clone, Serialize)]
pub struct FileData {
    pub path: String,
    pub content: Option<String>,
    pub error: Option<String>,
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FsData {
    pub tree: String,
    pub files: Vec<FileData>,
}

fn load_presets_file() -> Result<PresetsFile> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    let context_dir = config_dir.join("context");
    let config_path = context_dir.join("presets.toml");

    if !config_path.exists() {
        if fs::create_dir_all(&context_dir).is_ok() {
            let default_toml = include_str!("../presets.example.toml");
            if let Err(e) = fs::write(&config_path, default_toml) {
                log::warn!("Failed to write default presets.toml: {}", e);
                return Ok(PresetsFile::default());
            } else {
                log::info!("Created default presets file at {:?}", config_path);
            }
        } else {
            return Ok(PresetsFile::default());
        }
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
    max_files: usize,
    force: bool,
) -> Result<RuntimeConfig> {
    let presets_file = load_presets_file()?;

    let global = presets_file.global;
    let preset = preset_name
        .and_then(|k| presets_file.presets.get(k))
        .cloned()
        .unwrap_or_default();

    let mut final_include = combine_lists(vec![global.include, preset.include, include]);

    if final_include.is_empty() {
        final_include = vec!["**".into()];
    }

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
        max_files,
        force,
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
        args.max_files,
        args.force,
    )
}

fn is_binary_bytes(data: &[u8]) -> bool {
    if data.contains(&0) {
        return true;
    }
    let mut control_chars = 0;
    for &byte in data {
        if byte < 32 && byte != b'\n' && byte != b'\r' && byte != b'\t' && byte != 0x0C {
            control_chars += 1;
        }
    }
    let ratio = control_chars as f32 / data.len() as f32;
    ratio > 0.3
}

enum FileReadResult {
    Text(String),
    Binary,
    NonUtf8,
}

fn read_text_file(path: &Path) -> std::io::Result<FileReadResult> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;

    let mut chunk = [0; 8192];
    let n = file.read(&mut chunk)?;

    if n == 0 {
        return Ok(FileReadResult::Text(String::new()));
    }

    if is_binary_bytes(&chunk[..n]) {
        return Ok(FileReadResult::Binary);
    }

    let mut buffer = Vec::new();
    buffer.extend_from_slice(&chunk[..n]);
    file.read_to_end(&mut buffer)?;

    match String::from_utf8(buffer) {
        Ok(s) => Ok(FileReadResult::Text(s)),
        Err(_) => Ok(FileReadResult::NonUtf8),
    }
}

pub struct Scanner {
    root: PathBuf,
    include_set: GlobSet,
    exclude_set: GlobSet,
    tree_only_set: GlobSet,
    max_size: u64,
    max_files: usize,
}

impl Scanner {
    pub fn new(root: PathBuf, config: &RuntimeConfig) -> Result<Self> {
        Ok(Self {
            root,
            include_set: build_globset(&config.include)?,
            exclude_set: build_globset(&config.exclude)?,
            tree_only_set: build_globset(&config.include_in_tree)?,
            max_size: config.max_size,
            max_files: config.max_files,
        })
    }

    pub fn scan(&self) -> Vec<FileEntry> {
        let mut entries = Vec::new();
        let walker = WalkBuilder::new(&self.root)
            .hidden(false)
            .git_ignore(true)
            .build();

        for result in walker {
            if entries.len() >= self.max_files {
                log::warn!(
                    "⚠️ Reached maximum file limit ({}). Stopping scan early.",
                    self.max_files
                );
                break;
            }

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

fn gather_data(entries: &[FileEntry], config: &RuntimeConfig) -> FsData {
    let mut tree_out = String::new();
    for entry in entries {
        let indent = "    ".repeat(entry.depth.saturating_sub(1));
        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
        let marker = if entry.is_dir { "/" } else { "" };
        tree_out.push_str(&format!("{}{}{}\n", indent, name, marker));
    }

    let mut files = Vec::new();
    if !config.tree_only_output {
        for entry in entries {
            if entry.include_content {
                if entry.exceeds_size {
                    files.push(FileData {
                        path: entry.relative_path.clone(),
                        content: None,
                        error: Some("File exceeds maximum size limit.".into()),
                        skipped: None,
                    });
                    continue;
                }

                match read_text_file(&entry.path) {
                    Ok(FileReadResult::Text(content)) => {
                        files.push(FileData {
                            path: entry.relative_path.clone(),
                            content: Some(content),
                            error: None,
                            skipped: None,
                        });
                    }
                    Ok(FileReadResult::Binary) => {
                        files.push(FileData {
                            path: entry.relative_path.clone(),
                            content: None,
                            error: None,
                            skipped: Some("Binary file detected.".into()),
                        });
                    }
                    Ok(FileReadResult::NonUtf8) => {
                        files.push(FileData {
                            path: entry.relative_path.clone(),
                            content: None,
                            error: None,
                            skipped: Some("Non-UTF-8 text / binary file detected.".into()),
                        });
                    }
                    Err(e) => {
                        files.push(FileData {
                            path: entry.relative_path.clone(),
                            content: None,
                            error: Some(format!("Error reading file: {}", e)),
                            skipped: None,
                        });
                    }
                }
            }
        }
    }

    FsData {
        tree: tree_out.trim_end().to_string(),
        files,
    }
}

pub fn gather(args: &Cli) -> Result<Option<FsData>> {
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

    if !config.force {
        if let Some(home) = dirs::home_dir() {
            if target_dir == home.canonicalize().unwrap_or_else(|_| home.clone()) {
                anyhow::bail!("Cowardly refusing to scan the entire home directory. Use --force to override, or specify a narrower path/include pattern.");
            }
        }
        if target_dir == PathBuf::from("/") {
            anyhow::bail!(
                "Cowardly refusing to scan the entire root directory. Use --force to override."
            );
        }
    }

    let scanner = Scanner::new(target_dir.clone(), &config)?;
    let entries = scanner.scan();

    if entries.is_empty() {
        return Ok(None);
    }

    let data = gather_data(&entries, &config);
    Ok(Some(data))
}
