#!/usr/bin/env python3
import os
import sys
import argparse
import logging
import json
import re
from pathlib import Path
from dataclasses import dataclass
from typing import List, Optional, Tuple, Dict, Any

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        tomllib = None

try:
    import pathspec
except ImportError:
    pathspec = None


# --- CONFIGURATION DEFAULTS ---
DEFAULT_CONFIG_TOML = """\
# Default output format (xml, markdown, json)
format = "xml"

# Automatically find and use the root of the git project
git_root = false

# Global instruction set to inject into every context output
instructions = "You are a senior developer. Please review the following code for optimizations and security vulnerabilities."
"""

DEFAULT_PRESETS_TOML = """\
[global]
# Global patterns applied to every run
include = ["**/README.md"]
exclude = [
    "**/.git/**", "**/node_modules/**", "**/target/**", "**/__pycache__/**",
    "**/*.db", "**/*.sqlite", "**/*.png", "**/*.jpg", "**/*.so", "**/*.zip",
    "**/*.tar.gz", "**/*.gz", "**/*.tar", "**/*.tgz", "**/*.tar.xz", "**/*.xz",
    "**/*.min.js", "**/*.min.css", "**/*.min.js.map", "**/*.min.css.map",
    "**/package-lock.json", "**/pnpm-lock.yaml", "**/.venv/**", "**/venv/**"
]

[rust]
include = ["**/*.rs", "Cargo.toml"]
exclude = ["**/target/**"]
include_in_tree = ["Cargo.lock"]

[node]
include = ["**/*.ts", "**/*.tsx", "**/*.js", "**/*.jsx", "package.json", "tsconfig.json"]
exclude = ["**/node_modules/**", "**/dist/**", "**/build/**"]

[python]
include = ["**/*.py", "requirements.txt", "pyproject.toml"]
exclude = ["**/__pycache__/**", "**/.venv/**", "**/venv/**"]
"""

# --- LOGGING SETUP ---
logger = logging.getLogger("context")


# --- DATA STRUCTURES ---
@dataclass
class FileEntry:
    path: Path
    relative_path: str
    depth: int
    is_dir: bool
    include_content: bool
    exceeds_size: bool

@dataclass
class FileData:
    path: str
    content: Optional[str]
    error: Optional[str]
    skipped: Optional[str]

@dataclass
class FsData:
    tree: str
    files: List[FileData]

@dataclass
class RuntimeConfig:
    include: List[str]
    exclude: List[str]
    include_in_tree: List[str]
    tree_only_output: bool
    max_size: int
    max_files: int
    force: bool


# --- HELPER FUNCTIONS ---
def get_config_dir() -> Path:
    if sys.platform == "win32":
        base = Path(os.environ.get("APPDATA", "~")).expanduser()
    else:
        base = Path("~/.config").expanduser()
    return base / "context"

def load_toml(path: Path) -> dict:
    if not path.exists():
        return {}
    if tomllib is None:
        logger.warning("tomllib/tomli not installed. Cannot parse TOML files. 'pip install tomli'")
        return {}
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except Exception as e:
        logger.warning(f"Failed to parse {path}: {e}")
        return {}

def escape_xml(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;").replace("'", "&apos;")

def compile_glob(pattern: str) -> re.Pattern:
    """Converts a glob pattern like **/*.rs into a regex pattern."""
    p = pattern.replace('.', r'\.')
    # Use placeholders to prevent regex syntax from being clobbered
    p = p.replace('**/', '___ANYDIR___')
    p = p.replace('**', '___ANY___')
    p = p.replace('*', '[^/]*')
    p = p.replace('?', '[^/]')
    # Restore placeholders to valid regex syntax
    p = p.replace('___ANYDIR___', '(?:.*/)?')
    p = p.replace('___ANY___', '.*')
    return re.compile(f"^{p}$")

def match_globs(relative_path: str, patterns: List[re.Pattern]) -> bool:
    return any(p.match(relative_path) for p in patterns)

def combine_lists(lists: List[Optional[List[str]]]) -> List[str]:
    combined = []
    seen = set()
    for lst in lists:
        if lst:
            for item in lst:
                if item not in seen:
                    seen.add(item)
                    combined.append(item)
    return combined

def find_git_root(start_path: Path) -> Optional[Path]:
    for p in [start_path, *start_path.parents]:
        if (p / ".git").is_dir():
            return p
    return None

def is_binary_bytes(data: bytes) -> bool:
    if b'\0' in data:
        return True
    control_chars = sum(1 for b in data if b < 32 and b not in (9, 10, 13, 12))
    if len(data) > 0 and (control_chars / len(data)) > 0.3:
        return True
    return False

def read_text_file(path: Path) -> Tuple[str, Optional[str]]:
    try:
        with open(path, 'rb') as f:
            chunk = f.read(8192)
            if not chunk:
                return "Text", ""
            if is_binary_bytes(chunk):
                return "Binary", None
            f.seek(0)
            buffer = f.read()
            try:
                return "Text", buffer.decode('utf-8')
            except UnicodeDecodeError:
                return "NonUtf8", None
    except Exception as e:
        return "Error", str(e)


# --- CONFIGURATION RESOLUTION ---
class ConfigManager:
    def __init__(self):
        self.config_dir = get_config_dir()
        self.config_path = self.config_dir / "config.toml"
        self.presets_path = self.config_dir / "presets.toml"
        self._ensure_defaults()

    def _ensure_defaults(self):
        if not self.config_dir.exists():
            self.config_dir.mkdir(parents=True, exist_ok=True)
        if not self.config_path.exists():
            with open(self.config_path, "w") as f:
                f.write(DEFAULT_CONFIG_TOML)
            logger.info(f"Created default config file at {self.config_path}")
        if not self.presets_path.exists():
            with open(self.presets_path, "w") as f:
                f.write(DEFAULT_PRESETS_TOML)
            logger.info(f"Created default presets file at {self.presets_path}")

    def load_user_config(self) -> dict:
        return load_toml(self.config_path)

    def build_runtime_config(self, args: argparse.Namespace, fallback_preset: Optional[str]) -> RuntimeConfig:
        presets_data = load_toml(self.presets_path)
        global_preset = presets_data.get("global", {})
        
        selected_preset_name = args.preset or fallback_preset
        preset_config = presets_data.get(selected_preset_name, {}) if selected_preset_name else {}

        # Merge includes
        final_include = combine_lists([
            global_preset.get("include"),
            preset_config.get("include"),
            args.include
        ])
        if not final_include:
            final_include = ["**"]

        # Merge excludes
        hardcoded_excludes = ["**/.git/**"]
        final_exclude = combine_lists([
            hardcoded_excludes,
            global_preset.get("exclude"),
            preset_config.get("exclude"),
            args.exclude
        ])

        # Merge include_in_tree
        final_include_in_tree = combine_lists([
            global_preset.get("include_in_tree"),
            preset_config.get("include_in_tree"),
            args.include_in_tree
        ])

        return RuntimeConfig(
            include=final_include,
            exclude=final_exclude,
            include_in_tree=final_include_in_tree,
            tree_only_output=args.tree,
            max_size=args.max_size,
            max_files=args.max_files,
            force=args.force
        )


# --- SCANNER ---
class Scanner:
    def __init__(self, root: Path, config: RuntimeConfig):
        self.root = root
        self.config = config
        self.include_patterns = [compile_glob(p) for p in config.include]
        self.exclude_patterns = [compile_glob(p) for p in config.exclude]
        self.tree_patterns = [compile_glob(p) for p in config.include_in_tree]

        # Load .gitignore if pathspec is available
        self.gitignore_spec = None
        if pathspec:
            gitignore_path = self.root / ".gitignore"
            if gitignore_path.exists():
                try:
                    with open(gitignore_path, "r") as f:
                        self.gitignore_spec = pathspec.PathSpec.from_lines(
                            pathspec.patterns.GitWildMatchPattern, f
                        )
                except Exception as e:
                    logger.warning(f"Failed to read .gitignore: {e}")
        elif (self.root / ".gitignore").exists():
            logger.warning("Found .gitignore but 'pathspec' is not installed. Ignoring gitignore rules.")

    def scan(self) -> List[FileEntry]:
        entries = []
        
        for root, dirs, files in os.walk(self.root):
            if len(entries) >= self.config.max_files:
                logger.warning(f"⚠️ Reached maximum file limit ({self.config.max_files}). Stopping scan early.")
                break

            current_dir = Path(root)
            rel_dir = current_dir.relative_to(self.root)
            rel_dir_str = str(rel_dir).replace(os.sep, '/')
            
            # .gitignore filtering for dirs to prevent walking into ignored directories
            if self.gitignore_spec:
                ignored_dirs = set(self.gitignore_spec.match_files([f"{d}/" if rel_dir_str == "." else f"{rel_dir_str}/{d}/" for d in dirs]))
                dirs[:] = [d for d in dirs if f"{d}/" not in ignored_dirs and (f"{rel_dir_str}/{d}/" not in ignored_dirs)]
            
            # Custom exclude matching for dirs
            def is_dir_excluded(d_name: str) -> bool:
                p = d_name if rel_dir_str == "." else f"{rel_dir_str}/{d_name}"
                # Check both with and without a trailing slash to satisfy all glob variations
                return match_globs(p, self.exclude_patterns) or match_globs(f"{p}/", self.exclude_patterns)
            
            dirs[:] = [d for d in dirs if not is_dir_excluded(d)]
            
            # Skip .git manually just in case
            if ".git" in dirs:
                dirs.remove(".git")

            # Add directory entry if it's not the root
            if current_dir != self.root:
                depth = len(rel_dir.parts)
                entries.append(FileEntry(
                    path=current_dir,
                    relative_path=rel_dir_str,
                    depth=depth,
                    is_dir=True,
                    include_content=False,
                    exceeds_size=False
                ))

            for file_name in files:
                if len(entries) >= self.config.max_files:
                    break
                
                file_path = current_dir / file_name
                rel_file = file_path.relative_to(self.root)
                rel_file_str = str(rel_file).replace(os.sep, '/')

                if self.gitignore_spec and self.gitignore_spec.match_file(rel_file_str):
                    continue

                if match_globs(rel_file_str, self.exclude_patterns):
                    continue

                try:
                    size = file_path.stat().st_size
                except OSError:
                    continue

                matches_include = match_globs(rel_file_str, self.include_patterns)
                matches_tree = match_globs(rel_file_str, self.tree_patterns)

                if not matches_include and not matches_tree:
                    continue

                depth = len(rel_file.parts)
                entries.append(FileEntry(
                    path=file_path,
                    relative_path=rel_file_str,
                    depth=depth,
                    is_dir=False,
                    include_content=matches_include and not matches_tree,
                    exceeds_size=size > self.config.max_size
                ))

        # Sort entries properly to guarantee tree coherence
        entries.sort(key=lambda e: e.path)
        return entries


# --- FORMATTERS ---
class Formatter:
    @staticmethod
    def format_output(fmt: str, instructions: Optional[str], prompt: Optional[str], fs_data: Optional[FsData]) -> str:
        if fmt == "xml":
            return Formatter._format_xml(instructions, prompt, fs_data)
        elif fmt == "markdown":
            return Formatter._format_markdown(instructions, prompt, fs_data)
        elif fmt == "json":
            return Formatter._format_json(instructions, prompt, fs_data)
        return ""

    @staticmethod
    def _format_xml(instructions: Optional[str], prompt: Optional[str], fs: Optional[FsData]) -> str:
        out = []
        if instructions:
            out.append(f"<instructions>\n{instructions}\n</instructions>\n")
        if prompt:
            out.append(f"<prompt>\n{prompt}\n</prompt>\n")
            
        if fs:
            out.append(f"<directory_structure>\n{fs.tree}\n</directory_structure>\n")
            if fs.files:
                out.append("<file_contents>")
                for f in fs.files:
                    if f.error:
                        out.append(f'<file path="{escape_xml(f.path)}" error="true">\nError: {escape_xml(f.error)}\n</file>\n')
                    elif f.skipped:
                        out.append(f'<file path="{escape_xml(f.path)}" skipped="true">\nSkipped: {escape_xml(f.skipped)}\n</file>\n')
                    elif f.content is not None:
                        out.append(f'<file path="{escape_xml(f.path)}">\n{f.content}\n</file>\n')
                out.append("</file_contents>\n")
        return "\n".join(out).strip()

    @staticmethod
    def _format_markdown(instructions: Optional[str], prompt: Optional[str], fs: Optional[FsData]) -> str:
        out = []
        if instructions:
            out.append(f"## Instructions\n\n{instructions}\n")
        if prompt:
            out.append(f"## Prompt\n\n{prompt}\n")
            
        if fs:
            out.append(f"## Directory Structure\n\n```\n{fs.tree}\n```\n")
            if fs.files:
                out.append("## File Contents\n")
                for f in fs.files:
                    out.append(f"### File: `{f.path}`\n")
                    if f.error:
                        out.append(f"*Error: {f.error}*\n")
                    elif f.skipped:
                        out.append(f"*Skipped: {f.skipped}*\n")
                    elif f.content is not None:
                        content_str = f.content
                        if not content_str.endswith("\n"):
                            content_str += "\n"
                        out.append(f"```\n{content_str}```\n")
        return "\n".join(out).strip()

    @staticmethod
    def _format_json(instructions: Optional[str], prompt: Optional[str], fs: Optional[FsData]) -> str:
        output_dict = {}
        if instructions: output_dict["instructions"] = instructions
        if prompt: output_dict["prompt"] = prompt
        if fs:
            output_dict["directory_structure"] = fs.tree
            output_dict["files"] = [
                {k: v for k, v in f.__dict__.items() if v is not None}
                for f in fs.files
            ]
        try:
            return json.dumps(output_dict, indent=2)
        except Exception as e:
            return f'{{"error": "Failed to serialize to JSON: {e}"}}'


# --- MAIN EXECUTION ---
def gather_fs(target_dir: Path, config: RuntimeConfig) -> Optional[FsData]:
    if not target_dir.exists():
        logger.error(f"Target directory does not exist: {target_dir}")
        return None

    if not config.force:
        home = Path.home()
        if target_dir.resolve() == home.resolve():
            logger.error("Cowardly refusing to scan the entire home directory. Use --force to override.")
            sys.exit(1)
        if target_dir.resolve() == Path("/"):
            logger.error("Cowardly refusing to scan the entire root directory. Use --force to override.")
            sys.exit(1)

    scanner = Scanner(target_dir, config)
    entries = scanner.scan()

    if not entries:
        return None

    # Build Tree
    tree_out = []
    for entry in entries:
        indent = "    " * max(0, entry.depth - 1)
        marker = "/" if entry.is_dir else ("" if entry.include_content else " (content excluded)")
        tree_out.append(f"{indent}{entry.path.name}{marker}")
    tree_str = "\n".join(tree_out)

    # Gather File Contents
    files = []
    if not config.tree_only_output:
        for entry in entries:
            if entry.include_content:
                if entry.exceeds_size:
                    files.append(FileData(entry.relative_path, None, "File exceeds maximum size limit.", None))
                    continue

                status, content = read_text_file(entry.path)
                if status == "Text":
                    files.append(FileData(entry.relative_path, content, None, None))
                elif status == "Binary":
                    files.append(FileData(entry.relative_path, None, None, "Binary file detected."))
                elif status == "NonUtf8":
                    files.append(FileData(entry.relative_path, None, None, "Non-UTF-8 text / binary file detected."))
                else:
                    files.append(FileData(entry.relative_path, None, f"Error reading file: {content}", None))

    return FsData(tree=tree_str, files=files)

def main():
    parser = argparse.ArgumentParser(description="A universal tool to gather file and codebase context for LLMs.")
    parser.add_argument("path", nargs="?", default=".", help="Path to scan for files (defaults to current directory)")
    parser.add_argument("--format", choices=["xml", "markdown", "json"], help="Choose the output format")
    parser.add_argument("-q", "--quiet", action="store_true", help="Suppress stderr output (e.g., stats and info logs)")
    parser.add_argument("--prompt", help="Prepend a prompt to the generated context")
    parser.add_argument("--prompt-file", help="Read a prompt from a file to prepend to the generated context")
    parser.add_argument("--config", help="Shortcut to scan a specific folder inside your OS config directory")
    parser.add_argument("--preset", help="Use a predefined set of options from presets.toml")
    parser.add_argument("--git-root", action="store_true", help="Intelligently find and use the root of the git project")
    parser.add_argument("--no-git-root", action="store_true", help="Disable using the root of the git project")
    parser.add_argument("--tree", action="store_true", help="Show only the directory tree structure")
    parser.add_argument("--include", nargs="+", help="Patterns for files to include for content")
    parser.add_argument("--include-in-tree", nargs="+", help="Patterns for files to show in tree but without content")
    parser.add_argument("--exclude", nargs="+", help="Patterns to exclude")
    parser.add_argument("--max-size", type=int, default=1048576, help="Skip files larger than this size in bytes (default: 1MB)")
    parser.add_argument("--max-files", type=int, default=10000, help="Stop scanning after this many files (default: 10000)")
    parser.add_argument("--force", action="store_true", help="Force scanning of sensitive directories")

    args = parser.parse_args()

    # Logging config
    log_level = logging.WARNING if args.quiet else logging.INFO
    logging.basicConfig(level=log_level, format="%(message)s")

    # Handle prompts
    final_prompt = args.prompt or ""
    if args.prompt_file:
        try:
            with open(args.prompt_file, "r") as f:
                final_prompt = (final_prompt + "\n\n" + f.read()).strip()
        except Exception as e:
            logger.error(f"Failed to read prompt file: {e}")
            sys.exit(1)
    final_prompt = final_prompt if final_prompt else None

    # Load configuration
    cfg_manager = ConfigManager()
    user_config = cfg_manager.load_user_config()

    # Apply defaults from config
    use_git_root = (args.git_root or user_config.get("git_root", False)) and not args.no_git_root
    output_format = args.format or user_config.get("format", "xml")

    # Resolve target directory
    if args.config:
        target_dir = get_config_dir().parent / args.config
    else:
        target_dir = Path(args.path).resolve()

    if use_git_root:
        git_root = find_git_root(target_dir)
        if git_root:
            target_dir = git_root
        else:
            logger.warning(f"⚠️ --git-root specified, but no .git directory found. Falling back to {target_dir}")

    # Build the final filtering config
    project_name = target_dir.name
    runtime_config = cfg_manager.build_runtime_config(args, fallback_preset=project_name)

    # Gather data
    fs_data = gather_fs(target_dir, runtime_config)

    if not fs_data and not final_prompt:
        logger.warning("⚠️ No context generated. Try tweaking your arguments.")

    # Format and Output
    output = Formatter.format_output(
        fmt=output_format,
        instructions=user_config.get("instructions"),
        prompt=final_prompt,
        fs_data=fs_data
    )

    if output:
        print(output)
        if not args.tree and not args.quiet:
            lines = output.count("\n") + 1
            approx_tokens = len(output) // 4
            sys.stderr.write(f"\n✅ Context generated: {lines} lines, ~{approx_tokens} tokens\n")

if __name__ == "__main__":
    main()
