use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use ignore::{WalkBuilder, types::TypesBuilder};
use glob::Pattern;

const AFTER_HELP: &str = "\
EXAMPLES:
  # Copy all files from the current directory, respecting .gitignore
  fclip

  # Copy only Rust and Toml files from the 'src' directory
  fclip --include rs,toml ./src

  # Copy all files except .log and .tmp files
  fclip --exclude log,tmp .

  # Copy all files, but go no deeper than 2 directories
  fclip --depth 2 .

  # Explicitly include a file that is in .gitignore
  fclip --unignore 'README.md'
  fclip --unignore '*.md'
  fclip --unignore '.env.example'
";

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "A CLI tool to copy the contents of a codebase to the clipboard.",
    long_about = "fclip walks the specified directory, collects the content of all relevant files, and copies it to the clipboard, formatted with file paths as headers. It intelligently respects .gitignore rules and allows for fine-grained filtering by file extension.",
    after_help = AFTER_HELP
)]
struct Cli {

    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    #[arg(long, short)]
    depth: Option<usize>,

    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    use_gitignore: bool,

    #[arg(long, value_delimiter = ',')]
    unignore: Option<Vec<String>>,

    #[arg(short, long, value_delimiter = ',')]
    include: Option<Vec<String>>,

    #[arg(short, long, value_delimiter = ',')]
    exclude: Option<Vec<String>>,
    #[arg(long, short)]
    verbose: bool,
}

fn should_unignore_file(path: &Path, unignore_patterns: &[Pattern], verbose: bool) -> bool {
    let path_str = path.to_string_lossy();
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    
    for pattern in unignore_patterns {
        if pattern.matches(&path_str) {
            if verbose {
                eprintln!("File {} matches unignore pattern {} (full path)", path_str, pattern);
            }
            return true;
        }
        
        if pattern.matches(&file_name) {
            if verbose {
                eprintln!("File {} matches unignore pattern {} (filename)", path_str, pattern);
            }
            return true;
        }
        
        let unix_path = path_str.replace('\\', "/");
        if pattern.matches(&unix_path) {
            if verbose {
                eprintln!("File {} matches unignore pattern {} (unix path)", path_str, pattern);
            }
            return true;
        }
    }
    
    false
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut output_buffer = String::new();
    let mut files_copied = 0;

    let unignore_patterns: Result<Vec<Pattern>, _> = cli.unignore
        .as_ref()
        .map(|patterns| {
            patterns.iter()
                .map(|p| Pattern::new(p.trim()))
                .collect()
        })
        .unwrap_or_else(|| Ok(Vec::new()));
    
    let unignore_patterns = unignore_patterns.map_err(|e| anyhow::anyhow!("Invalid glob pattern: {}", e))?;

    let mut types_builder = TypesBuilder::new();
    types_builder.add_defaults();
    
    if let Some(includes) = &cli.include {
        for ext in includes {
            let clean_ext = ext.trim().trim_start_matches('.');
            types_builder.add(clean_ext, &format!("*.{}", clean_ext))?;
            types_builder.select(clean_ext);
        }
    } else {
        types_builder.select("all");
    }

    if let Some(excludes) = &cli.exclude {
        for ext in excludes {
            let clean_ext = ext.trim().trim_start_matches('.');
            types_builder.add(clean_ext, &format!("*.{}", clean_ext))?;
            types_builder.negate(clean_ext);
        }
    }
    let types = types_builder.build()?;

    for path in &cli.paths {
        if cli.verbose {
            eprintln!("Walking path: {}", path.display());
        }

        let mut walker = WalkBuilder::new(path);
        walker
            .max_depth(cli.depth)
            .git_ignore(cli.use_gitignore)
            .types(types.clone());

        let mut found_files = std::collections::HashSet::new();

        for result in walker.build() {
            let entry = match result {
                Ok(e) => e,
                Err(e) => {
                    if cli.verbose {
                        eprintln!("Warning: {}", e);
                    }
                    continue;
                }
            };
            
            if entry.file_type().map_or(false, |ft| ft.is_file()) {
                found_files.insert(entry.path().to_path_buf());
            }
        }

        if !unignore_patterns.is_empty() {
            let mut walker_no_ignore = WalkBuilder::new(path);
            walker_no_ignore
                .max_depth(cli.depth)
                .git_ignore(false)
                .types(types.clone());

            for result in walker_no_ignore.build() {
                let entry = match result {
                    Ok(e) => e,
                    Err(e) => {
                        if cli.verbose {
                            eprintln!("Warning: {}", e);
                        }
                        continue;
                    }
                };
                
                if entry.file_type().map_or(false, |ft| ft.is_file()) {
                    let file_path = entry.path().to_path_buf();

                    if !found_files.contains(&file_path) {
                        if should_unignore_file(&file_path, &unignore_patterns, cli.verbose) {
                            found_files.insert(file_path);
                        }
                    }
                }
            }
        }

        let mut file_paths: Vec<_> = found_files.into_iter().collect();
        file_paths.sort();

        for file_path in file_paths {
            if cli.verbose {
                eprintln!("Processing: {}", file_path.display());
            }
            
            match fs::read_to_string(&file_path) {
                Ok(content) => {
                    output_buffer.push_str(&format!("--- {} ---\n", file_path.display()));
                    output_buffer.push_str(&content);
                    output_buffer.push_str("\n\n");
                    files_copied += 1;
                    
                    if cli.verbose {
                        eprintln!("Copied: {}", file_path.display());
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Skipping file {}: {}", file_path.display(), e);
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