# fclip

`fclip` is a CLI tool written in Rust that recursively reads the contents of specified files from a given directory and copies them to your clipboard as a single block of text.

The main goal is to help users easily dump massive amounts of files into LLMs for context.

---

## ✨ Features

-   **Blazing Fast:** Written in Rust for maximum performance and efficiency, even on large codebases.
-   **Recursive Copy:** Scans a directory and all its subdirectories to gather files.
-   **Smart Filtering:** Easily include or exclude files by their extension (e.g., `copy only .rs and .toml files`).
-   **Intelligent Ignoring:** Automatically skips version control directories (`.git`), build artifacts (`target/`, `node_modules/`), and hidden dotfiles by default to keep the output clean.
-   **Single Binary:** Compiles to a single, dependency-free executable that you can place anywhere on your system.

## 🚀 Installation

You must have the [Rust toolchain](https://rustup.rs/) installed to build from source.

### Recommended Method (via `cargo install`)

This will build the optimized binary and place it in your Cargo binary path, making it available everywhere.

1.  Clone this repository:
    ```sh
    git clone https://github.com/munavv3r/fclip.git
    cd fclip
    ```

2.  Install using `cargo`:
    ```sh
    cargo install --path .
    ```

After installation, restart your terminal and the `fclip` command will be available.

## Usage

The basic syntax is `fclip [OPTIONS] [PATH]`. If no path is provided, it defaults to the current directory (`.`).

### Copy an Entire Project

This will copy all supported files from the current directory and its subdirectories.
```sh
fclip .
```

### Copy Files from a Specific Folder

```sh
fclip ./src
```

### Copy Only Specific File Types

Use the `-i` or `--include` flag with a comma-separated list of extensions. The leading dot is optional.

```sh
# Copy only Rust and TOML files
fclip --include rs,toml .
```

### Exclude Specific File Types

Use the `-e` or `--exclude` flag. This is useful for ignoring minified files or logs.

```sh
# Copy all files except .lock files
fclip --exclude lock .
```

### Combine Filters

You can use include and exclude flags together for more precise control. For example, to copy all `.js` files except for `.min.js`:

```sh
fclip --include js --exclude min.js .
```

### Get Help

See all available options and commands by running:

```sh
fclip --help
```
