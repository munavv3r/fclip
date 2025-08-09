# fclip

A powerful CLI tool to recursively copy file contents to your clipboard, supercharged for LLM context.

---

`fclip` is a command-line tool written in Rust that walks a directory, reads the contents of relevant files, and copies them to your clipboard as a single, formatted block of text.

Its primary goal is to make it effortless to provide large amounts of code or text as context to Large Language Models (LLMs).

## Features

- **Powerful Content Aggregation**: Recursively scans directories to gather file contents into a single text block.
- **Intelligent Filtering**:
  - Respects `.gitignore`, `.ignore`, and other global ignore files by default.
  - Precisely `--include` or `--exclude` files by extension.
  - Ability to `--unignore` specific files or patterns that would normally be ignored.
- **Advanced Control**:
  - Limit recursion with `--depth` to avoid going too deep into directories.
  - Set a `--max-size-mb` limit to prevent accidentally copying enormous projects.
  - Perform a `--dry-run` to see which files *would* be copied without actually touching the clipboard.
- **Flexible Output Formatting**:
  - Choose between `default`, `markdown` (with code blocks), and `json` formats using the `--format` flag.
- **Smart & Safe**:
  - Automatically detects and skips binary files.
  - Provides detailed file statistics with the `--stats` flag.
- **Performant & Portable**:
  - Written in Rust for maximum speed, even on large codebases.
  - Compiles to a single, dependency-free binary.

## Installation

You must have the [Rust toolchain](https://rustup.rs/) installed to build from source.

The recommended method is to install directly with `cargo`, which builds the optimized binary and makes it available in your system's PATH.

1. Clone the repository:
   ```sh
   git clone https://github.com/munavv3r/fclip.git
   cd fclip
   ```

2. Install using `cargo`:
   ```sh
   cargo install --path .
   ```

After installation, restart your terminal or source your shell profile, and the `fclip` command will be available.

## Usage

The basic syntax is `fclip [OPTIONS] [PATHS...]`. If no path is provided, it defaults to the current directory (`.`).

### Basic Examples

```sh
# Copy all relevant files from the current directory
fclip

# Copy files from a specific directory
fclip ./src

# Copy files from multiple locations at once
fclip ./src ./docs
```

### Filtering Files

```sh
# Copy only Rust and Toml files
fclip --include rs,toml .

# Copy all files except .log and .tmp files
fclip --exclude log,tmp

# Include all '.md' files, but exclude 'NOTE.md'
fclip --include md --exclude NOTE.md
```

### Controlling the Walk

```sh
# Copy files, but go no deeper than 2 directories from the starting point
fclip --depth 2 .

# Explicitly include the '.env.example' file, even if it's in .gitignore
fclip --unignore .env.example

# You can also use glob patterns to un-ignore files
fclip --unignore '*.md'
```

### Output and Safety

```sh
# Format the output as Markdown with language-tagged code blocks
fclip --format markdown .

# Perform a dry run to see what files would be copied, without modifying the clipboard
fclip --dry-run

# Show detailed statistics about the files being copied
fclip --stats

# Set a maximum total size of 5MB for the copied content
fclip --max-size-mb 5
```

### Getting Help

To see all available commands and options, run:

```sh
fclip --help
```

## All Options

- `--include (-i)`: Comma-separated list of file extensions to include. Default: (all).
- `--exclude (-e)`: Comma-separated list of file extensions to exclude. Default: (none).
- `--depth`: Max depth to search for files. Default: (none).
- `--unignore`: Comma-separated list of glob patterns to un-ignore. Default: (none).
- `--use-gitignore`: Whether to respect .gitignore files. Default: true.
- `--format`: Output format (default, markdown, json). Default: default.
- `--max-size-mb`: Maximum total size of content to copy in megabytes. Default: 10.
- `--dry-run`: List files that would be copied without action. Default: false.
- `--stats`: Show statistics about the copied files. Default: false.
- `--verbose (-v)`: Enable verbose logging during processing. Default: false.
- `--help (-h)`: Show the help message.
- `--version (-V)`: Show the application version.
