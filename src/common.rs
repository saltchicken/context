use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use pathdiff::diff_paths;
use std::fs;
use std::path::{Path, PathBuf};

/// Represents the final configuration after merging presets and CLI args.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_in_tree: Vec<String>,
    pub tree_only_output: bool,
}

/// Represents a single file discovered during the scan.
#[derive(Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub relative_path: String,
    pub depth: usize,
    pub is_dir: bool,
    pub include_content: bool,
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
                match fs::read_to_string(&entry.path) {
                    Ok(content) => {
                        blocks.push(format!(
                            "<file path=\"{}\">\n{}\n</file>",
                            entry.relative_path, content
                        ));
                    }
                    Err(e) => {
                        blocks.push(format!(
                            "<file path=\"{}\" error=\"true\">Error reading file: {}</file>",
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
}

impl Scanner {
    pub fn new(root: PathBuf, config: &RuntimeConfig) -> Result<Self> {
        Ok(Self {
            root,
            include_set: build_globset(&config.include)?,
            exclude_set: build_globset(&config.exclude)?,
            tree_only_set: build_globset(&config.include_in_tree)?,
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

        let is_dir = path.is_dir();
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
