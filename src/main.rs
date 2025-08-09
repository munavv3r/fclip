use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use clap::Parser;
use glob::Pattern;
use ignore::{WalkBuilder, types::TypesBuilder};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde_json::Value;

fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    
    let chars = text.chars().count();
    let bytes = text.len();

    let mut ascii_chars = 0;
    let mut whitespace_chars = 0;
    let mut punctuation_chars = 0;
    let mut unicode_chars = 0;
    let mut newlines = 0;
    
    for ch in text.chars() {
        match ch {
            '\n' => newlines += 1,
            c if c.is_ascii_whitespace() => whitespace_chars += 1,
            c if c.is_ascii_punctuation() => punctuation_chars += 1,
            c if c.is_ascii() => ascii_chars += 1,
            _ => unicode_chars += 1,
        }
    }
    
    let base_tokens = (ascii_chars as f64 * 0.75) +
                     (whitespace_chars as f64 * 0.25) +
                     (punctuation_chars as f64 * 1.0) +
                     (unicode_chars as f64 * 1.5) +
                     (newlines as f64 * 1.0);
    
    let code_indicators = text.matches(&['{', '}', '(', ')', '[', ']', ';', '=', '"']).count();
    let code_factor = if code_indicators > chars / 20 { 1.3 } else { 1.0 };

    let length_factor = if chars > 10000 { 0.95 } else if chars > 1000 { 1.0 } else { 1.1 };

    let complexity_factor = (bytes as f64 / chars.max(1) as f64) * 0.1 + 0.9;
    
    let estimated = base_tokens * code_factor * length_factor * complexity_factor;
    
    let min_estimate = chars / 6;
    let max_estimate = chars * 2;
    
    (estimated as usize).max(min_estimate).min(max_estimate)
}

fn is_likely_binary(bytes: &[u8]) -> bool {
    let sample_size = bytes.len().min(1024);
    let sample = &bytes[0..sample_size];
    
    let null_count = sample.iter().filter(|&&b| b == 0).count();
    let non_printable_count = sample.iter()
        .filter(|&&b| b < 32 && b != 9 && b != 10 && b != 13)
        .count();
    
    null_count > 0 || (non_printable_count as f32 / sample_size as f32) > 0.3
}

fn should_auto_exclude(path: &Path) -> bool {
    let common_excludes = [
        "node_modules", "target", ".git", ".svn", ".hg",
        "dist", "build", "__pycache__", ".pytest_cache",
        "coverage", ".coverage", ".nyc_output",
        "vendor", "deps", ".gradle", ".m2",
        ".idea", ".vscode", ".vs", "*.log", "*.tmp",
        "*.cache", "package-lock.json", "yarn.lock",
        "Cargo.lock", "poetry.lock", "Pipfile.lock",
        ".DS_Store", "Thumbs.db", "*.swp", "*.swo",
    ];
    
    let path_str = path.to_string_lossy().to_lowercase();
    let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    
    for exclude in &common_excludes {
        if exclude.contains('*') {
            if exclude.starts_with("*.") {
                let ext = exclude.trim_start_matches("*.");
                if path_str.ends_with(ext) {
                    return true;
                }
            }
        } else if path_str.contains(exclude) || file_name.contains(exclude) {
            return true;
        }
    }
    
    false
}

fn compress_content(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = String::new();
    
    for line in lines {
        if line.trim().is_empty() {
            result.push('\n');
            continue;
        }
        
        let leading_whitespace_end = line.chars()
            .position(|c| c != ' ' && c != '\t')
            .unwrap_or(line.len());
        
        let indentation = &line[..leading_whitespace_end];
        let content_part = &line[leading_whitespace_end..];
        
        result.push_str(indentation);
        
        let mut prev_space = false;
        let mut in_string = false;
        let mut string_char = '"';
        let mut prev_char = '\0';
        
        for ch in content_part.chars() {
            match ch {
                '"' | '\'' if prev_char != '\\' => {
                    if !in_string {
                        in_string = true;
                        string_char = ch;
                    } else if ch == string_char {
                        in_string = false;
                    }
                    result.push(ch);
                    prev_space = false;
                }
                ' ' if !in_string => {
                    if !prev_space {
                        result.push(' ');
                        prev_space = true;
                    }
                }
                '\t' if !in_string => {
                    if !prev_space {
                        result.push(' ');
                        prev_space = true;
                    }
                }
                _ => {
                    result.push(ch);
                    prev_space = false;
                }
            }
            prev_char = ch;
        }
        
        result.push('\n');
    }
    
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    
    result
}


fn generate_directory_tree(paths: &[PathBuf], max_depth: Option<usize>) -> String {
    let mut tree = String::from("## Project Structure\n\n```\n");
    
    for path in paths {
        if path.is_dir() {
            tree.push_str(&format!("{}/\n", path.display()));
            add_directory_contents(&mut tree, path, 0, max_depth.unwrap_or(3), "");
        } else {
            tree.push_str(&format!("{}\n", path.display()));
        }
    }
    
    tree.push_str("```\n\n");
    tree
}

fn add_directory_contents(tree: &mut String, dir: &Path, current_depth: usize, max_depth: usize, prefix: &str) {
    if current_depth >= max_depth {
        return;
    }
    
    if let Ok(entries) = fs::read_dir(dir) {
        let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        items.sort_by_key(|entry| entry.file_name());
        
        for (i, entry) in items.iter().enumerate() {
            let path = entry.path();
            let is_last = i == items.len() - 1;
            let current_prefix = if is_last { "└── " } else { "├── " };
            let next_prefix = if is_last { "    " } else { "│   " };
            
            if should_auto_exclude(&path) {
                continue;
            }
            
            tree.push_str(&format!("{}{}{}\n", prefix, current_prefix, 
                         entry.file_name().to_string_lossy()));
            
            if path.is_dir() && current_depth < max_depth - 1 {
                add_directory_contents(tree, &path, current_depth + 1, max_depth, 
                                     &format!("{}{}", prefix, next_prefix));
            }
        }
    }
}

fn find_dependencies(paths: &[PathBuf]) -> String {
    let mut deps = String::from("## Dependencies\n\n");
    let mut found_any = false;
    
    for path in paths {
        let search_dir = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        
        let package_json = search_dir.join("package.json");
        if package_json.exists() {
            if let Ok(content) = fs::read_to_string(&package_json) {
                if let Ok(json) = serde_json::from_str::<Value>(&content) {
                    deps.push_str("### JavaScript/Node.js (package.json)\n");
                    if let Some(dependencies) = json.get("dependencies").and_then(|d| d.as_object()) {
                        for (name, version) in dependencies {
                            deps.push_str(&format!("- {}: {}\n", name, version.as_str().unwrap_or("*")));
                        }
                    }
                    deps.push('\n');
                    found_any = true;
                }
            }
        }
        
        let cargo_toml = search_dir.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(content) = fs::read_to_string(&cargo_toml) {
                deps.push_str("### Rust (Cargo.toml)\n");
                let lines: Vec<&str> = content.lines().collect();
                let mut in_dependencies = false;
                
                for line in lines {
                    let trimmed = line.trim();
                    if trimmed == "[dependencies]" {
                        in_dependencies = true;
                        continue;
                    }
                    if trimmed.starts_with('[') && trimmed != "[dependencies]" {
                        in_dependencies = false;
                    }
                    if in_dependencies && trimmed.contains('=') && !trimmed.starts_with('#') {
                        deps.push_str(&format!("- {}\n", trimmed));
                    }
                }
                deps.push('\n');
                found_any = true;
            }
        }
        
        let requirements = search_dir.join("requirements.txt");
        if requirements.exists() {
            if let Ok(content) = fs::read_to_string(&requirements) {
                deps.push_str("### Python (requirements.txt)\n");
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        deps.push_str(&format!("- {}\n", trimmed));
                    }
                }
                deps.push('\n');
                found_any = true;
            }
        }
        
        let go_mod = search_dir.join("go.mod");
        if go_mod.exists() {
            if let Ok(content) = fs::read_to_string(&go_mod) {
                deps.push_str("### Go (go.mod)\n");
                let lines: Vec<&str> = content.lines().collect();
                let mut in_require = false;
                
                for line in lines {
                    let trimmed = line.trim();
                    if trimmed.starts_with("require (") {
                        in_require = true;
                        continue;
                    }
                    if trimmed == ")" && in_require {
                        in_require = false;
                    }
                    if (in_require || trimmed.starts_with("require ")) && !trimmed.starts_with("//") {
                        let clean_line = trimmed.replace("require ", "").replace("(", "").trim().to_string();
                        if !clean_line.is_empty() && clean_line != ")" {
                            deps.push_str(&format!("- {}\n", clean_line));
                        }
                    }
                }
                deps.push('\n');
                found_any = true;
            }
        }
    }
    
    if found_any {
        deps
    } else {
        String::new()
    }
}

fn group_files_by_type(files: &[(PathBuf, String)]) -> Vec<(String, Vec<&(PathBuf, String)>)> {
    let mut groups: HashMap<String, Vec<&(PathBuf, String)>> = HashMap::new();
    
    for file in files {
        let ext = file.0.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("no-extension");
        
        let group = match ext {
            "rs" => "Rust Source",
            "py" => "Python Source", 
            "js" | "jsx" => "JavaScript Source",
            "ts" | "tsx" => "TypeScript Source",
            "html" | "htm" => "HTML Templates",
            "css" | "scss" | "sass" => "Stylesheets",
            "json" => "JSON Configuration",
            "toml" => "TOML Configuration",
            "yml" | "yaml" => "YAML Configuration",
            "md" | "markdown" => "Documentation",
            "txt" | "text" => "Text Files",
            "sh" | "bash" | "zsh" => "Shell Scripts",
            "sql" => "Database Scripts",
            "go" => "Go Source",
            "java" => "Java Source",
            "c" | "h" => "C Source",
            "cpp" | "hpp" | "cc" => "C++ Source",
            "no-extension" => "Files without extension",
            _ => "Other Files",
        }.to_string();
        
        groups.entry(group).or_default().push(file);
    }
    
    let mut sorted_groups: Vec<_> = groups.into_iter().collect();
    sorted_groups.sort_by_key(|(group_name, files)| (group_name.clone(), files.len()));
    sorted_groups.reverse();
    
    sorted_groups
}

const AFTER_HELP: &str = "\
EXAMPLES:
  # Copy all files from the current directory, respecting .gitignore
  fclip

  # Copy only Rust and Toml files from the 'src' directory with structure
  fclip --include rs,toml --include-structure ./src

  # Copy all files except .log and .tmp files, limit to 50k tokens
  fclip --exclude log,tmp --max-tokens 50000 .

  # Output to file instead of clipboard, with dependencies info
  fclip --output-file codebase.txt --include-dependencies .

  # Compress whitespace and group by file type
  fclip --compress --group-by-type --max-tokens 100000 .
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

    #[arg(long)]
    max_tokens: Option<usize>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Default)]
    format: OutputFormat,

    #[arg(long)]
    stats: bool,

    #[arg(long)]
    include_structure: bool,

    #[arg(long)]
    include_dependencies: bool,

    #[arg(long)]
    group_by_type: bool,

    #[arg(long)]
    auto_exclude_common: bool,

    #[arg(long)]
    exclude_empty: bool,

    #[arg(long)]
    compress: bool,

    #[arg(long)]
    output_file: Option<PathBuf>,

    #[arg(long)]
    append_to_file: bool,

    #[arg(long)]
    split_by_size: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum OutputFormat {
    Default,
    Markdown,
    Json,
}

fn parse_size(size_str: &str) -> Result<usize> {
    let size_str = size_str.to_lowercase().replace(" ", "");
    
    let (num_str, multiplier) = if size_str.ends_with("gb") {
        (size_str.trim_end_matches("gb"), 1024 * 1024 * 1024)
    } else if size_str.ends_with("mb") {
        (size_str.trim_end_matches("mb"), 1024 * 1024)
    } else if size_str.ends_with("kb") {
        (size_str.trim_end_matches("kb"), 1024)
    } else if size_str.ends_with("b") {
        (size_str.trim_end_matches("b"), 1)
    } else {
        (size_str.as_str(), 1)
    };
    
    let num: f64 = num_str.parse()
        .map_err(|_| anyhow::anyhow!("Invalid size format: '{}'", size_str))?;
    
    if num < 0.0 {
        return Err(anyhow::anyhow!("Size cannot be negative"));
    }
    
    let result = (num * multiplier as f64) as usize;
    
    if result == 0 && num > 0.0 {
        Ok(1)
    } else {
        Ok(result)
    }
}


fn write_output_chunks(content: &str, output_file: &Path, chunk_size: usize, append: bool) -> Result<()> {
    if content.len() <= chunk_size {
        let mut file = if append {
            fs::OpenOptions::new().create(true).append(true).open(output_file)?
        } else {
            fs::File::create(output_file)?
        };
        file.write_all(content.as_bytes())?;
        println!("Output written to: {}", output_file.display());
    } else {
        let base_name = output_file.file_stem().unwrap().to_string_lossy();
        let extension = output_file.extension().unwrap_or_default().to_string_lossy();
        let parent = output_file.parent().unwrap_or(Path::new("."));
        
        let chunks: Vec<&str> = content.as_bytes()
            .chunks(chunk_size)
            .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
            .collect();
        
        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_filename = if extension.is_empty() {
                format!("{}_part_{:03}", base_name, i + 1)
            } else {
                format!("{}_part_{:03}.{}", base_name, i + 1, extension)
            };
            let chunk_path = parent.join(chunk_filename);
            
            let mut file = if append && i == 0 {
                fs::OpenOptions::new().create(true).append(true).open(&chunk_path)?
            } else {
                fs::File::create(&chunk_path)?
            };
            file.write_all(chunk.as_bytes())?;
            println!("Chunk {} written to: {}", i + 1, chunk_path.display());
        }
    }
    Ok(())
}

fn format_output(files: &[(PathBuf, String)], format: &OutputFormat, cli: &Cli) -> String {
    let mut output = String::new();
    
    if cli.include_structure {
        output.push_str(&generate_directory_tree(&cli.paths, cli.depth));
    }
    
    if cli.include_dependencies {
        let deps = find_dependencies(&cli.paths);
        if !deps.is_empty() {
            output.push_str(&deps);
        }
    }
    
    if cli.group_by_type {
        let grouped = group_files_by_type(files);
        for (group_name, group_files) in grouped {
            output.push_str(&format!("# {}\n\n", group_name));
            for (path, content) in group_files {
                let processed_content = if cli.compress {
                    compress_content(content)
                } else {
                    content.clone()
                };
                
                match format {
                    OutputFormat::Default => {
                        output.push_str(&format!("--- {} ---\n", path.display()));
                        output.push_str(&processed_content);
                        if !processed_content.ends_with('\n') {
                            output.push('\n');
                        }
                        output.push('\n');
                    }
                    OutputFormat::Markdown => {
                        output.push_str(&format!("## {}\n\n", path.display()));
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        let lang = match ext {
                            "rs" => "rust", "py" => "python", "js" => "javascript",
                            "ts" => "typescript", "html" => "html", "css" => "css",
                            "json" => "json", "toml" => "toml", "yml" | "yaml" => "yaml",
                            "md" => "markdown", "sh" => "bash", "ps1" => "powershell",
                            _ => "",
                        };
                        output.push_str(&format!("```{}\n", lang));
                        output.push_str(&processed_content);
                        if !processed_content.ends_with('\n') {
                            output.push('\n');
                        }
                        output.push_str("```\n\n");
                    }
                    OutputFormat::Json => {
                    }
                }
            }
            output.push('\n');
        }
        return output;
    }

    match format {
        OutputFormat::Default => {
            for (path, content) in files {
                let processed_content = if cli.compress {
                    compress_content(content)
                } else {
                    content.clone()
                };
                
                output.push_str(&format!("--- {} ---\n", path.display()));
                output.push_str(&processed_content);
                if !processed_content.ends_with('\n') {
                    output.push('\n');
                }
                output.push('\n');
            }
        }
        OutputFormat::Markdown => {
            for (path, content) in files {
                let processed_content = if cli.compress {
                    compress_content(content)
                } else {
                    content.clone()
                };
                
                output.push_str(&format!("## {}\n\n", path.display()));
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let lang = match ext {
                    "rs" => "rust", "py" => "python", "js" => "javascript",
                    "ts" => "typescript", "html" => "html", "css" => "css",
                    "json" => "json", "toml" => "toml", "yml" | "yaml" => "yaml",
                    "md" => "markdown", "sh" => "bash", "ps1" => "powershell",
                    _ => "",
                };
                output.push_str(&format!("```{}\n", lang));
                output.push_str(&processed_content);
                if !processed_content.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n\n");
            }
        }
        OutputFormat::Json => {
            let files_json: Vec<serde_json::Value> = files.iter()
                .map(|(path, content)| {
                    let processed_content = if cli.compress {
                        compress_content(content)
                    } else {
                        content.clone()
                    };
                    
                    serde_json::json!({
                        "path": path.to_string_lossy(),
                        "content": processed_content,
                        "tokens": estimate_tokens(&processed_content),
                        "size": processed_content.len()
                    })
                })
                .collect();
            
            let mut json_output = serde_json::json!({
                "files": files_json,
                "metadata": {
                    "total_files": files.len(),
                    "total_size": files.iter().map(|(_, c)| c.len()).sum::<usize>(),
                    "total_tokens": files.iter().map(|(_, c)| estimate_tokens(c)).sum::<usize>(),
                }
            });
            
            if cli.include_structure {
                json_output["structure"] = serde_json::Value::String(generate_directory_tree(&cli.paths, cli.depth));
            }
            
            if cli.include_dependencies {
                json_output["dependencies"] = serde_json::Value::String(find_dependencies(&cli.paths));
            }
            
            output = serde_json::to_string_pretty(&json_output).unwrap_or_else(|_| "Error formatting JSON".to_string());
        }
    }
    
    output
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

fn process_files_parallel(
    file_paths: Vec<PathBuf>,
    cli: &Cli,
    max_size_bytes: usize,
) -> Result<Vec<(PathBuf, String)>> {
    let total_files = file_paths.len();
    
    if total_files == 0 {
        return Ok(Vec::new());
    }
    
    let multi_progress = MultiProgress::new();
    let main_pb = multi_progress.add(ProgressBar::new(total_files as u64));
    main_pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );
    main_pb.set_message("Processing files...");
    
    let total_size = Arc::new(AtomicUsize::new(0));
    let total_tokens = Arc::new(AtomicUsize::new(0));
    
    let results: Vec<_> = file_paths
        .into_par_iter()
        .filter_map(|file_path| {
            let result = process_single_file(&file_path, cli);
            main_pb.inc(1);
            
            match result {
                Ok(Some((path, content))) => {
                    let content_size = content.len();
                    let content_tokens = estimate_tokens(&content);
                    
                    let current_size = total_size.load(Ordering::Relaxed);
                    if current_size + content_size > max_size_bytes {
                        if cli.verbose {
                            main_pb.println(format!(
                                "Warning: Skipping {} - would exceed size limit",
                                path.display()
                            ));
                        }
                        return None;
                    }
                    
                    if let Some(max_tokens) = cli.max_tokens {
                        let current_tokens = total_tokens.load(Ordering::Relaxed);
                        if current_tokens + content_tokens > max_tokens {
                            if cli.verbose {
                                main_pb.println(format!(
                                    "Warning: Skipping {} - would exceed token limit",
                                    path.display()
                                ));
                            }
                            return None;
                        }
                    }
                    
                    total_size.fetch_add(content_size, Ordering::Relaxed);
                    total_tokens.fetch_add(content_tokens, Ordering::Relaxed);
                    
                    if cli.verbose {
                        main_pb.println(format!(
                            "✓ {} ({} bytes, ~{} tokens)",
                            path.display(),
                            content_size,
                            content_tokens
                        ));
                    }
                    
                    Some((path, content))
                }
                Ok(None) => None,
                Err(e) => {
                    if cli.verbose {
                        main_pb.println(format!("✗ {}: {}", file_path.display(), e));
                    }
                    None
                }
            }
        })
        .collect();
    
    main_pb.finish_with_message("Complete!");
    
    let mut final_results = results;
    final_results.sort_by(|a, b| a.0.cmp(&b.0));
    
    Ok(final_results)
}

fn process_single_file(file_path: &PathBuf, cli: &Cli) -> Result<Option<(PathBuf, String)>> {
    if let Some(ref output_file) = cli.output_file {
        if let (Ok(file_canonical), Ok(output_canonical)) = 
            (file_path.canonicalize(), output_file.canonicalize()) {
            if file_canonical == output_canonical {
                return Ok(None);
            }
        }
    }
    
    match fs::read_to_string(file_path) {
        Ok(mut content) => {
            if cli.exclude_empty && content.trim().is_empty() {
                return Ok(None);
            }

            if content.starts_with('\u{FEFF}') {
                content = content.trim_start_matches('\u{FEFF}').to_string();
            }
            content = content.replace("\r\n", "\n");
            
            Ok(Some((file_path.clone(), content)))
        }
        Err(e) => {
            if let Ok(bytes) = fs::read(file_path) {
                if is_likely_binary(&bytes) {
                    Ok(None)
                } else {
                    Err(anyhow::anyhow!("Text file with encoding issues: {}", e))
                }
            } else {
                Err(anyhow::anyhow!("Cannot read file: {}", e))
            }
        }
    }
}

fn print_enhanced_stats(files_data: &[(PathBuf, String)], total_size: usize, total_tokens: usize) {
    let mut ext_counts: HashMap<String, usize> = HashMap::new();
    let mut ext_sizes: HashMap<String, usize> = HashMap::new();
    let mut ext_tokens: HashMap<String, usize> = HashMap::new();
    let mut total_lines = 0;
    let mut total_chars = 0;
    
    for (path, content) in files_data {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("(no extension)")
            .to_string();
        
        let tokens = estimate_tokens(content);
        let lines = content.lines().count();
        let chars = content.chars().count();
        
        *ext_counts.entry(ext.clone()).or_insert(0) += 1;
        *ext_sizes.entry(ext.clone()).or_insert(0) += content.len();
        *ext_tokens.entry(ext).or_insert(0) += tokens;
        total_lines += lines;
        total_chars += chars;
    }
    
    eprintln!("📊 PROCESSING COMPLETE");
    eprintln!("Files: {} | Size: {:.1} KB | Tokens: ~{} | Lines: {} | Characters: {}", 
             files_data.len(), 
             total_size as f64 / 1024.0, 
             total_tokens,
             total_lines,
             total_chars);
    
    if total_chars > 0 {
        let tokens_per_char = total_tokens as f64 / total_chars as f64;
        eprintln!("Token density: {:.2} tokens/char", tokens_per_char);
    }
    
    eprintln!("\n📁 BY FILE TYPE:");
    let mut ext_data: Vec<_> = ext_counts.iter().collect();
    ext_data.sort_by_key(|&(_, count)| std::cmp::Reverse(*count));
    
    for (ext, count) in ext_data {
        let size_kb = ext_sizes[ext] as f64 / 1024.0;
        let tokens = ext_tokens[ext];
        let avg_tokens_per_file = if *count > 0 { tokens / count } else { 0 };
        eprintln!("  {:12} {} files ({:6.1} KB, ~{:5} tokens, ~{}/file)", 
                 format!("{}:", ext), count, size_kb, tokens, avg_tokens_per_file);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let max_size_bytes = cli.max_size_mb * 1024 * 1024;

    let unignore_patterns: Result<Vec<Pattern>, _> = cli.unignore
        .as_ref()
        .map(|patterns| patterns.iter().map(|p| Pattern::new(p.trim())).collect())
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

    let all_file_paths = {
        let mut found_files = std::collections::HashSet::new();

        for path in &cli.paths {
            if cli.verbose {
                eprintln!("Walking path: {}", path.display());
            }

            let mut walker = WalkBuilder::new(path);
            walker
                .max_depth(cli.depth)
                .git_ignore(cli.use_gitignore)
                .types(types.clone());

            for result in walker.build() {
                let entry = match result {
                    Ok(e) => e,
                    Err(e) => {
                        if cli.verbose { eprintln!("Warning: {}", e); }
                        continue;
                    }
                };
                
                if entry.file_type().map_or(false, |ft| ft.is_file()) {
                    let file_path = entry.path();
                    if cli.auto_exclude_common && should_auto_exclude(file_path) {
                        if cli.verbose { eprintln!("Auto-excluded: {}", file_path.display()); }
                        continue;
                    }
                    found_files.insert(file_path.to_path_buf());
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
                            if cli.verbose { eprintln!("Warning: {}", e); }
                            continue;
                        }
                    };
                    
                    if entry.file_type().map_or(false, |ft| ft.is_file()) {
                        let file_path = entry.path().to_path_buf();
                        if !found_files.contains(&file_path) && should_unignore_file(&file_path, &unignore_patterns, cli.verbose) {
                            found_files.insert(file_path);
                        }
                    }
                }
            }
        }
        
        let mut paths: Vec<_> = found_files.into_iter().collect();
        paths.sort();
        paths
    };

    if cli.dry_run {
        let mut files_data = Vec::new();
        let mut total_size_bytes = 0;
        let mut total_tokens = 0;

        for file_path in all_file_paths {
            if let Ok(Some((path, content))) = process_single_file(&file_path, &cli) {
                let content_size = content.len();
                let content_tokens = estimate_tokens(&content);
                if total_size_bytes + content_size > max_size_bytes { continue; }
                if let Some(max_tokens) = cli.max_tokens {
                    if total_tokens + content_tokens > max_tokens { continue; }
                }
                total_size_bytes += content_size;
                total_tokens += content_tokens;
                files_data.push((path, content));
            }
        }

        eprintln!("=== DRY RUN - Would process {} file(s) ({:.1} KB, ~{} tokens) ===", 
                 files_data.len(), total_size_bytes as f64 / 1024.0, total_tokens);
        
        for (path, content) in &files_data {
            let lines = content.lines().count();
            let tokens = estimate_tokens(content);
            eprintln!("  {} ({} lines, {} bytes, ~{} tokens)", 
                     path.display(), lines, content.len(), tokens);
        }

        if cli.stats {
            eprintln!("\n=== STATISTICS ===");
            print_enhanced_stats(&files_data, total_size_bytes, total_tokens);
        }
    } else {
        let files_data = process_files_parallel(all_file_paths, &cli, max_size_bytes)?;
        
        if !files_data.is_empty() {
            let total_size_bytes: usize = files_data.iter().map(|(_, c)| c.len()).sum();
            let total_tokens: usize = files_data.iter().map(|(_, c)| estimate_tokens(c)).sum();
            
            let formatted_output = format_output(&files_data, &cli.format, &cli);
            let output_tokens = estimate_tokens(&formatted_output);
            
            if let Some(output_file) = &cli.output_file {
                if let Some(split_size_str) = &cli.split_by_size {
                    let split_size = parse_size(split_size_str)?;
                    write_output_chunks(&formatted_output, output_file, split_size, cli.append_to_file)?;
                } else {
                    let mut file = if cli.append_to_file {
                        fs::OpenOptions::new().create(true).append(true).open(output_file)?
                    } else {
                        fs::File::create(output_file)?
                    };
                    file.write_all(formatted_output.as_bytes())?;
                    println!("Output written to: {}", output_file.display());
                }
            } else {
                let mut clipboard = arboard::Clipboard::new()?;
                clipboard.set_text(formatted_output)?;
                eprintln!("Copied content of {} file(s) to clipboard.", files_data.len());
            }
            
            if cli.stats {
                eprintln!("\n=== STATISTICS ===");
                print_enhanced_stats(&files_data, total_size_bytes, total_tokens);
            }

            eprintln!("📋 Processed {} file(s) ({:.1} KB, ~{} tokens -> ~{} output tokens)",
                     files_data.len(), 
                     total_size_bytes as f64 / 1024.0, 
                     total_tokens, 
                     output_tokens);
        } else {
            eprintln!("No files found matching the criteria.");
        }
    }

    Ok(())
}