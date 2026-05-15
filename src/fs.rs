use crate::cli::Cli;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use pathdiff::diff_paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write, IsTerminal};
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
    pub absolute_paths: bool,
}

/// Represents a single file discovered during the scan.
#[derive(Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub display_path: String,
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
    pub project_name: String,
    pub tree: String,
    pub files: Vec<FileData>,
}

fn load_presets_file() -> Result<PresetsFile> {
    let config_dir = dirs::config_dir().context("Could not determine config directory")?;
    let context_dir = config_dir.join("context");
    let config_path = context_dir.join("presets.toml");

    if !config_path.exists() {
        if fs::create_dir_all(&context_dir).is_ok() {
            let default_toml = include_str!("../assets/presets.example.toml");
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
    lists.into_iter().flatten().flatten().collect()
}

fn prompt_and_create_preset(preset_name: &str) -> Result<PresetConfig> {
    // Safety check: Don't prompt if stdin isn't a terminal (e.g., if piped from another command)
    if !io::stdin().is_terminal() {
        log::warn!("No preset found for '{}' and stdin is not a terminal. Skipping initialization.", preset_name);
        return Ok(PresetConfig::default());
    }

    // Use stderr instead of stdout so the prompt remains visible when piping to wl-copy
    let mut stderr = io::stderr();
    write!(stderr, "⚠️ No preset found for '{}'. Initialize one? [y/N]: ", preset_name)?;
    stderr.flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        return Ok(PresetConfig::default());
    }

    write!(stderr, "Include patterns (default: **/*): ")?;
    stderr.flush()?;
    let mut include_input = String::new();
    io::stdin().read_line(&mut include_input)?;
    let include_input = include_input.trim();
    
    let include_vec = if include_input.is_empty() {
        vec!["**/*".to_string()]
    } else {
        include_input.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };

    write!(stderr, "Exclude patterns (default: none): ")?;
    stderr.flush()?;
    let mut exclude_input = String::new();
    io::stdin().read_line(&mut exclude_input)?;
    let exclude_input = exclude_input.trim();

    let exclude_vec = if exclude_input.is_empty() {
        None
    } else {
        let parsed: Vec<String> = exclude_input
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if parsed.is_empty() { None } else { Some(parsed) }
    };

    let new_preset = PresetConfig {
        include: Some(include_vec.clone()),
        exclude: exclude_vec.clone(),
        include_in_tree: None,
    };

    if let Some(config_dir) = dirs::config_dir() {
        let presets_path = config_dir.join("context").join("presets.toml");
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&presets_path) {
            let mut toml_block = format!("\n[\"{}\"]\n", preset_name);
            
            let include_fmt = include_vec.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(", ");
            toml_block.push_str(&format!("include = [{}]\n", include_fmt));

            if let Some(excl) = &exclude_vec {
                let exclude_fmt = excl.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(", ");
                toml_block.push_str(&format!("exclude = [{}]\n", exclude_fmt));
            }

            let _ = file.write_all(toml_block.as_bytes());
        }
    }

    Ok(new_preset)
}

#[allow(clippy::too_many_arguments)]
pub fn build_config(
    preset_name: Option<&str>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    include_in_tree: Option<Vec<String>>,
    tree_only: bool,
    max_size: u64,
    max_files: usize,
    force: bool,
    absolute_paths: bool,
) -> Result<RuntimeConfig> {
    let presets_file = load_presets_file().unwrap_or_default();

    let global = presets_file.global;
    let preset = if let Some(name) = preset_name {
        if let Some(p) = presets_file.presets.get(name) {
            p.clone()
        } else {
            prompt_and_create_preset(name).unwrap_or_default()
        }
    } else {
        PresetConfig::default()
    };

    let mut final_include = combine_lists(vec![global.include, preset.include, include]);

    if final_include.is_empty() {
        final_include = vec!["**".into()];
    }

    let hardcoded_excludes = vec![
        "**/.git/**".into(),
        "**/*.venv".into(),
        "**/.env*".into(),
        "**/*.pem".into(),
        "**/*.key".into(),
        "**/id_rsa*".into(),
        "**/secrets.json".into(),
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
        absolute_paths,
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
        args.absolute_paths,
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

    // Optimizing Capacity Allocations: Avoid extending arrays and reallocating heavily in a loop.
    let metadata = file.metadata()?;
    let file_size = metadata.len() as usize;

    let mut chunk = [0; 8192];
    let n = file.read(&mut chunk)?;

    if n == 0 {
        return Ok(FileReadResult::Text(String::new()));
    }

    if is_binary_bytes(&chunk[..n]) {
        return Ok(FileReadResult::Binary);
    }

    let mut buffer = Vec::with_capacity(file_size);
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
    absolute_paths: bool,
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
            absolute_paths: config.absolute_paths,
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
        let display_path = if self.absolute_paths {
            path.canonicalize()
                .unwrap_or_else(|_| path.to_path_buf())
                .to_string_lossy()
                .to_string()
        } else {
            relative_str.to_string()
        };

        Some(FileEntry {
            path: path.to_path_buf(),
            display_path,
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

fn gather_data(project_name: String, entries: &[FileEntry], config: &RuntimeConfig) -> FsData {
    let mut tree_out = String::new();

    // If absolute paths are used, we typically don't want a deeply nested tree
    // because each path is self-contained. For now, we'll keep the visual indentation
    // but the node text will be the full absolute path.
    for entry in entries {
        let indent = "    ".repeat(entry.depth.saturating_sub(1));
        
        let name = if config.absolute_paths {
            &entry.display_path
        } else {
            entry.path.file_name().unwrap_or_default().to_str().unwrap_or_default()
        };

        let marker = if entry.is_dir {
            "/"
        } else if !entry.include_content {
            " (content excluded)"
        } else {
            ""
        };

        tree_out.push_str(&format!("{}{}{}\n", indent, name, marker));
    }

    let mut files = Vec::new();
    if !config.tree_only_output {
        for entry in entries {
            if entry.include_content {
                if entry.exceeds_size {
                    files.push(FileData {
                        path: entry.display_path.clone(),
                        content: None,
                        error: Some("File exceeds maximum size limit.".into()),
                        skipped: None,
                    });
                    continue;
                }

                match read_text_file(&entry.path) {
                    Ok(FileReadResult::Text(content)) => {
                        files.push(FileData {
                            path: entry.display_path.clone(),
                            content: Some(content),
                            error: None,
                            skipped: None,
                        });
                    }
                    Ok(FileReadResult::Binary) => {
                        files.push(FileData {
                            path: entry.display_path.clone(),
                            content: None,
                            error: None,
                            skipped: Some("Binary file detected.".into()),
                        });
                    }
                    Ok(FileReadResult::NonUtf8) => {
                        files.push(FileData {
                            path: entry.display_path.clone(),
                            content: None,
                            error: None,
                            skipped: Some("Non-UTF-8 text / binary file detected.".into()),
                        });
                    }
                    Err(e) => {
                        files.push(FileData {
                            path: entry.display_path.clone(),
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
        project_name,
        tree: tree_out.trim_end().to_string(),
        files,
    }
}

/// Helper function to traverse upwards and find the git root
pub fn find_git_root(start_path: &Path) -> Option<PathBuf> {
    for ancestor in start_path.ancestors() {
        if ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

pub fn resolve_target_dirs(args: &Cli) -> Result<Vec<PathBuf>> {
    let mut resolved = Vec::new();

    for path_str in &args.paths {
        let current_dir = env::current_dir().context("Failed to get current directory")?;
        let initial_target = current_dir.join(path_str);

        let mut target_dir = initial_target.canonicalize().unwrap_or(initial_target);

        // Follow git root logic by default unless disabled
        if !args.no_git_root {
            if let Some(git_root) = find_git_root(&target_dir) {
                target_dir = git_root;
            } else {
                // FAIL explicitly if no .git directory is found in the path's ancestors
                anyhow::bail!(
                    "Could not find a .git repository root for {:?}. Run with --no-git-root (or --cwd) to force scanning the local directory anyway.", 
                    target_dir
                );
            }
        }

        // Deduplicate
        if !resolved.contains(&target_dir) {
            resolved.push(target_dir);
        }
    }

    Ok(resolved)
}

pub fn gather(target_dir: &Path, args: &Cli) -> Result<Option<FsData>> {
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

    let scanner = Scanner::new(target_dir.to_path_buf(), &config)?;
    let entries = scanner.scan();

    if entries.is_empty() {
        return Ok(None);
    }

    let resolved_name = project_name.unwrap_or("unknown").to_string();
    let data = gather_data(resolved_name, &entries, &config);
    Ok(Some(data))
}

/// Wraps the single `gather` function to aggregate multiple projects
pub fn gather_multiple(target_dirs: &[PathBuf], args: &Cli) -> Result<Option<Vec<FsData>>> {
    let mut results = Vec::new();

    for dir in target_dirs {
        match gather(dir, args) {
            Ok(Some(data)) => results.push(data),
            Ok(None) => {}, // Skip if empty
            Err(e) => log::warn!("Failed to gather context for {:?}: {}", dir, e),
        }
    }

    if results.is_empty() {
        Ok(None)
    } else {
        Ok(Some(results))
    }
}
