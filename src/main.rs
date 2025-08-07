use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use walkdir::{DirEntry, WalkDir};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(default_value = ".")]
    path: PathBuf,

    #[arg(short, long, value_delimiter = ',')]
    include: Option<Vec<String>>,

    #[arg(short, long, value_delimiter = ',')]
    exclude: Option<Vec<String>>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let include_set: HashSet<String> = cli
        .include
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim_start_matches('.').to_string())
        .collect();
    
    let exclude_set: HashSet<String> = cli
        .exclude
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim_start_matches('.').to_string())
        .collect();

    let mut output_buffer = String::new();
    let mut files_copied = 0;
    let walker = WalkDir::new(&cli.path)
        .into_iter()
        .filter_entry(|e| !is_unwanted_dir(e));

    for entry_result in walker {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Warning: Could not access entry: {}", e);
                continue;
            }
        };

        if entry.file_type().is_dir() {
            continue;
        }

        if should_copy_file(entry.path(), &include_set, &exclude_set) {
            match fs::read_to_string(entry.path()) {
                Ok(content) => {
                    output_buffer.push_str(&format!("--- {} ---\n", entry.path().display()));
                    output_buffer.push_str(&content);
                    output_buffer.push_str("\n\n");
                    files_copied += 1;
                }
                Err(_) => {
                    eprintln!(
                        "Warning: Skipping non-UTF8 or unreadable file: {}",
                        entry.path().display()
                    );
                }
            }
        }
    }

    if files_copied > 0 {
        let mut clipboard = arboard::Clipboard::new()?;
        clipboard.set_text(output_buffer)?;
        eprintln!("Copied content of {} file(s) to clipboard.", files_copied);
    } else {
        eprintln!("No files found matching the criteria.");
    }

    Ok(())
}

fn is_unwanted_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .map(|s| s == ".git" || s == "target" || s == "node_modules")
            .unwrap_or(false)
}

fn should_copy_file(
    path: &Path,
    include_set: &HashSet<String>,
    exclude_set: &HashSet<String>,
) -> bool {
    if path.file_name().and_then(|s| s.to_str()).map_or(false, |s| s.starts_with('.')) {
        return false;
    }

    let extension_str = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => ext,
        None => return include_set.is_empty(),
    };

    if !exclude_set.is_empty() && exclude_set.contains(extension_str) {
        return false;
    }

    if !include_set.is_empty() && !include_set.contains(extension_str) {
        return false;
    }

    true
}