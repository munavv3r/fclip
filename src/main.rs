use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;

use anyhow::Result;
use clap::Parser;
use ignore::{WalkBuilder, types::TypesBuilder};
use glob::Pattern;

fn is_likely_binary(bytes: &[u8]) -> bool {
    let sample_size = bytes.len().min(1024);
    let sample = &bytes[0..sample_size];
    
    let null_count = sample.iter().filter(|&&b| b == 0).count();
    let non_printable_count = sample.iter()
        .filter(|&&b| b < 32 && b != 9 && b != 10 && b != 13)
        .count();
    
    null_count > 0 || (non_printable_count as f32 / sample_size as f32) > 0.3
}

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
    
    #[arg(long)]
    dry_run: bool,

    #[arg(long, default_value_t = 10)]
    max_size_mb: usize,

    #[arg(long, value_enum, default_value_t = OutputFormat::Default)]
    format: OutputFormat,

    #[arg(long)]
    stats: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum OutputFormat {
    Default,
    Markdown,
    Json,
}

fn format_output(files: &[(PathBuf, String)], format: &OutputFormat) -> String {
    match format {
        OutputFormat::Default => {
            let mut output = String::new();
            for (path, content) in files {
                output.push_str(&format!("--- {} ---\n", path.display()));
                output.push_str(content);
                if !content.ends_with('\n') {
                    output.push('\n');
                }
                output.push('\n');
            }
            output
        }
        OutputFormat::Markdown => {
            let mut output = String::new();
            for (path, content) in files {
                output.push_str(&format!("## {}\n\n", path.display()));
                
                let ext = path.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                
                let lang = match ext {
                    "rs" => "rust",
                    "py" => "python", 
                    "js" => "javascript",
                    "ts" => "typescript",
                    "html" => "html",
                    "css" => "css",
                    "json" => "json",
                    "toml" => "toml",
                    "yml" | "yaml" => "yaml",
                    "md" => "markdown",
                    "sh" => "bash",
                    "ps1" => "powershell",
                    _ => "",
                };
                
                output.push_str(&format!("```{}\n", lang));
                output.push_str(content);
                if !content.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n\n");
            }
            output
        }
        OutputFormat::Json => {
            let files_json: Vec<serde_json::Value> = files.iter()
                .map(|(path, content)| {
                    serde_json::json!({
                        "path": path.to_string_lossy(),
                        "content": content
                    })
                })
                .collect();
            
            serde_json::to_string_pretty(&serde_json::json!({
                "files": files_json
            })).unwrap_or_else(|_| "Error formatting JSON".to_string())
        }
    }
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

fn print_stats(files_data: &[(PathBuf, String)], total_size: usize) {
    let mut ext_counts: HashMap<String, usize> = HashMap::new();
    let mut ext_sizes: HashMap<String, usize> = HashMap::new();
    let mut total_lines = 0;
    
    for (path, content) in files_data {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("(no extension)")
            .to_string();
        
        *ext_counts.entry(ext.clone()).or_insert(0) += 1;
        *ext_sizes.entry(ext).or_insert(0) += content.len();
        total_lines += content.lines().count();
    }
    
    eprintln!("Total files: {}", files_data.len());
    eprintln!("Total size: {:.1} KB", total_size as f64 / 1024.0);
    eprintln!("Total lines: {}", total_lines);
    eprintln!("\nBy file type:");
    
    let mut ext_data: Vec<_> = ext_counts.iter().collect();
    ext_data.sort_by_key(|&(_, count)| std::cmp::Reverse(*count));
    
    for (ext, count) in ext_data {
        let size_kb = ext_sizes[ext] as f64 / 1024.0;
        eprintln!("  {}: {} files ({:.1} KB)", ext, count, size_kb);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut files_data = Vec::new();
    let mut total_size_bytes = 0usize;
    let max_size_bytes = cli.max_size_mb * 1024 * 1024;
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
                Ok(mut content) => {

                    if content.starts_with('\u{FEFF}') {
                        content = content.trim_start_matches('\u{FEFF}').to_string();
                    }
                    
                    content = content.replace("\r\n", "\n");
                    
                    let content_size = content.len();
                    
                    if total_size_bytes + content_size > max_size_bytes {
                        eprintln!("Warning: Skipping {} - would exceed size limit of {}MB", 
                                file_path.display(), cli.max_size_mb);
                        continue;
                    }
                    
                    total_size_bytes += content_size;
                    files_data.push((file_path.clone(), content));
                    
                    if cli.verbose {
                        eprintln!("Added: {} ({} bytes)", file_path.display(), content_size);
                    }
                }
                Err(e) => {
                    if let Ok(bytes) = fs::read(&file_path) {
                        if is_likely_binary(&bytes) {
                            if cli.verbose {
                                eprintln!("Skipping binary file: {}", file_path.display());
                            }
                        } else {
                            eprintln!("Warning: File {} appears to be text but has encoding issues: {}", file_path.display(), e);
                        }
                    } else {
                        eprintln!("Warning: Cannot read file {}: {}", file_path.display(), e);
                    }
                }
            }
        }
    }

    if !files_data.is_empty() {
        let formatted_output = format_output(&files_data, &cli.format);
        
        if cli.dry_run {
            eprintln!("=== DRY RUN - Would copy {} file(s) ({:.1} KB) ===", 
                     files_data.len(), total_size_bytes as f64 / 1024.0);
            
            for (path, content) in &files_data {
                let lines = content.lines().count();
                eprintln!("  {} ({} lines, {} bytes)", 
                         path.display(), lines, content.len());
            }
            
            if cli.stats {
                eprintln!("\n=== STATISTICS ===");
                print_stats(&files_data, total_size_bytes);
            }
        } else {
            let mut clipboard = arboard::Clipboard::new()?;
            clipboard.set_text(formatted_output)?;
            eprintln!("Copied content of {} file(s) to clipboard ({:.1} KB).", 
                     files_data.len(), total_size_bytes as f64 / 1024.0);
            
            if cli.stats {
                eprintln!("\n=== STATISTICS ===");
                print_stats(&files_data, total_size_bytes);
            }
        }
    } else {
        eprintln!("No files found matching the criteria.");
    }

    Ok(())
}