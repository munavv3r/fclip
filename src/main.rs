use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use ignore::{WalkBuilder, DirEntry, overrides::OverrideBuilder};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {

    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    #[arg(long, short)]
    depth: Option<usize>,

    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    use_gitignore: bool,

    #[arg(long, value_delimiter = ',')]
    exclude_file: Option<Vec<String>>,

    #[arg(short, long, value_delimiter = ',')]
    include: Option<Vec<String>>,

    #[arg(short, long, value_delimiter = ',')]
    exclude: Option<Vec<String>>,
}


fn main() -> Result<()> {
    let cli = Cli::parse();

    let include_exts: HashSet<String> = cli.include.unwrap_or_default().into_iter().map(|s| s.trim_start_matches('.').to_string()).collect();
    let exclude_exts: HashSet<String> = cli.exclude.unwrap_or_default().into_iter().map(|s| s.trim_start_matches('.').to_string()).collect();

    let mut output_buffer = String::new();
    let mut files_copied = 0;

    let mut override_builder = OverrideBuilder::new(".");
    override_builder.add("!.env")?; 
    if let Some(files) = cli.exclude_file {
        for file in files {
            override_builder.add(&format!("!{}", file))?;
        }
    }
    let overrides = override_builder.build()?;

    for path in &cli.paths {
        let mut walker = WalkBuilder::new(path);

        walker
            .max_depth(cli.depth)
            .git_ignore(cli.use_gitignore)
            .overrides(overrides.clone());

        for result in walker.build() {
            let entry = match result {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Warning: Could not process entry: {}", e);
                    continue;
                }
            };
            
            if should_copy_file(&entry, &include_exts, &exclude_exts) {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    output_buffer.push_str(&format!("--- {} ---\n", entry.path().display()));
                    output_buffer.push_str(&content);
                    output_buffer.push_str("\n\n");
                    files_copied += 1;
                } else {
                    eprintln!("Warning: Skipping non-UTF8 or unreadable file: {}", entry.path().display());
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

fn should_copy_file(
    entry: &DirEntry,
    include_exts: &HashSet<String>,
    exclude_exts: &HashSet<String>,
) -> bool {
    if entry.file_type().map_or(true, |ft| ft.is_dir()) {
        return false;
    }

    let path = entry.path();
    let extension_str = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => ext,
        None => return include_exts.is_empty(),
    };

    if !exclude_exts.is_empty() && exclude_exts.contains(extension_str) {
        return false;
    }
    
    if !include_exts.is_empty() && !include_exts.contains(extension_str) {
        return false;
    }

    true
}