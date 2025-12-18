use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(name = "fask")]
#[command(about = "Find and search for TODOs in your codebase", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search for TODOs in current files (like ripgrep)
    Current {
        /// Pattern to search for (default: "TODO")
        #[arg(short, long, default_value = "TODO")]
        pattern: String,

        /// Number of context lines to show
        #[arg(short = 'C', long, default_value = "2")]
        context: usize,

        /// File pattern to include (e.g., "*.rs", "*.js")
        #[arg(short = 't', long)]
        file_type: Option<String>,

        /// Directory to search in (default: current directory)
        #[arg(short, long, default_value = ".")]
        directory: PathBuf,
    },

    /// Search for TODOs added after a specific date in git history
    Since {
        /// Date in YYYY-MM-DD format (e.g., "2025-12-01")
        #[arg(short, long)]
        date: String,

        /// Pattern to search for (default: "TODO")
        #[arg(short, long, default_value = "TODO")]
        pattern: String,

        /// Number of context lines to show
        #[arg(short = 'C', long, default_value = "2")]
        context: usize,

        /// Directory to search in (default: current directory)
        #[arg(short = 'D', long, default_value = ".")]
        directory: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Current {
            pattern,
            context,
            file_type,
            directory,
        } => search_current_files(&pattern, context, file_type, directory)?,

        Commands::Since {
            date,
            pattern,
            context,
            directory,
        } => search_since_date(&date, &pattern, context, directory)?,
    }

    Ok(())
}

fn search_current_files(
    pattern: &str,
    context: usize,
    file_type: Option<String>,
    directory: PathBuf,
) -> Result<()> {
    println!("Searching for '{}' in current files...\n", pattern);

    let mut cmd = Command::new("rg");
    cmd.arg(pattern)
        .arg(format!("-C{}", context))
        .arg("--color=always")
        .arg("--line-number")
        .arg("--column");

    if let Some(ft) = file_type {
        cmd.arg("-g").arg(ft);
    }

    cmd.arg(directory);

    let output = cmd
        .output()
        .context("Failed to execute ripgrep. Is 'rg' installed?")?;

    if output.status.success() && !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    } else {
        println!("No matches found.");
    }

    Ok(())
}

/// Represents a match found in git history
#[derive(Debug, Clone)]
struct GitMatch {
    file: String,
    line_number: usize,
    line_content: String,
    commit_date: NaiveDate,
    commit_hash: String,
}

/// Represents a line that was added in a commit (from diff parsing)
#[derive(Debug)]
struct AddedLine {
    file: String,
    content: String,
    commit_date: NaiveDate,
    commit_hash: String,
}

/// Parse git log -p output to find lines that were added containing the pattern
fn parse_git_log_diff(output: &str, pattern: &str) -> Vec<AddedLine> {
    let mut results = Vec::new();
    let mut current_hash = String::new();
    let mut current_date: Option<NaiveDate> = None;
    let mut current_file: Option<String> = None;

    for line in output.lines() {
        // Commit line: "commit <hash>"
        if let Some(hash) = line.strip_prefix("commit ") {
            current_hash = hash.trim().to_string();
            current_date = None;
            current_file = None;
        }
        // Date line: "Date: <date>"
        else if let Some(date_str) = line.strip_prefix("Date:") {
            // Parse date like "2025-01-15" from the formatted output
            let date_str = date_str.trim();
            if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                current_date = Some(date);
            }
        }
        // Diff file header: "diff --git a/path b/path" or "+++ b/path"
        else if let Some(rest) = line.strip_prefix("+++ b/") {
            current_file = Some(rest.to_string());
        }
        // Added line in diff (starts with + but not +++)
        else if line.starts_with('+') && !line.starts_with("+++") {
            let content = &line[1..]; // Remove the leading +
            if content.contains(pattern) {
                if let (Some(date), Some(file)) = (current_date, &current_file) {
                    results.push(AddedLine {
                        file: file.clone(),
                        content: content.to_string(),
                        commit_date: date,
                        commit_hash: current_hash.clone(),
                    });
                }
            }
        }
    }

    results
}

/// Find where an added line currently exists in a file (if it still exists)
/// Returns the line number if found, along with the actual current line content
fn find_line_in_current_file(
    file: &str,
    content: &str,
    pattern: &str,
    directory: &Path,
) -> Option<(usize, String)> {
    let file_path = directory.join(file);
    let file_content = std::fs::read_to_string(&file_path).ok()?;

    let content_trimmed = content.trim();

    for (idx, line) in file_content.lines().enumerate() {
        let line_trimmed = line.trim();

        // The line must contain the pattern we're searching for
        if !line.contains(pattern) {
            continue;
        }

        // Check if this line matches the added content
        // Either exact match or the content is contained in the line (handles minor changes)
        if line_trimmed == content_trimmed || line_trimmed.contains(content_trimmed) {
            return Some((idx + 1, line.to_string())); // 1-based line number
        }
    }
    None
}

/// Read file contents to get context lines
fn read_file_lines(file: &str, directory: &Path) -> Result<Vec<String>> {
    let file_path = directory.join(file);
    let content = std::fs::read_to_string(&file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;
    Ok(content.lines().map(|s| s.to_string()).collect())
}

/// Print matches with context
fn print_matches_with_context(
    matches: &[GitMatch],
    context: usize,
    directory: &Path,
) -> Result<()> {
    // Sort all matches by date (oldest first)
    let mut sorted_matches: Vec<&GitMatch> = matches.iter().collect();
    sorted_matches.sort_by_key(|m| m.commit_date);

    let mut first_match = true;
    for m in sorted_matches {
        if !first_match {
            println!();
        }
        first_match = false;

        let lines = match read_file_lines(&m.file, directory) {
            Ok(l) => l,
            Err(_) => {
                // Print basic info if we can't read the file
                println!(
                    "\x1b[35m{}\x1b[0m:\x1b[32m{}\x1b[0m: {} (added \x1b[36m{}\x1b[0m in \x1b[33m{}\x1b[0m)",
                    m.file,
                    m.line_number,
                    m.line_content.trim(),
                    m.commit_date,
                    &m.commit_hash[..8.min(m.commit_hash.len())]
                );
                continue;
            }
        };

        let start = m.line_number.saturating_sub(context).max(1);
        let end = (m.line_number + context).min(lines.len());

        // Print file header with commit info
        println!(
            "\x1b[35m{}\x1b[0m (added \x1b[36m{}\x1b[0m in \x1b[33m{}\x1b[0m)",
            m.file,
            m.commit_date,
            &m.commit_hash[..8.min(m.commit_hash.len())]
        );

        for i in start..=end {
            if i > lines.len() {
                break;
            }
            let line_content = &lines[i - 1];
            if i == m.line_number {
                // Highlight the matching line
                println!("\x1b[32m{:>4}\x1b[0m: \x1b[1m{}\x1b[0m", i, line_content);
            } else {
                // Context line
                println!("\x1b[2m{:>4}: {}\x1b[0m", i, line_content);
            }
        }
    }

    Ok(())
}

fn search_since_date(date: &str, pattern: &str, context: usize, directory: PathBuf) -> Result<()> {
    // Validate and parse date
    let _since_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD (e.g., 2025-12-01)")?;

    println!(
        "Searching for '{}' in lines added since {}...\n",
        pattern, date
    );

    // Use git log -S with -p to get the actual diffs
    // This is fast because -S (pickaxe) is optimized, and we get exact info about what was added
    let log_output = Command::new("git")
        .arg("log")
        .arg(format!("--since={}", date))
        .arg("-S")
        .arg(pattern)
        .arg("-p") // Show patches (diffs)
        .arg("--format=commit %H%nDate: %ad")
        .arg("--date=short")
        .arg("--diff-filter=AM") // Only additions and modifications
        .current_dir(&directory)
        .output()
        .context("Failed to execute git log")?;

    if !log_output.status.success() {
        anyhow::bail!("git log failed. Is this a git repository?");
    }

    let output_str = String::from_utf8_lossy(&log_output.stdout);

    // Parse the diff output to find lines that were actually added
    let added_lines = parse_git_log_diff(&output_str, pattern);

    if added_lines.is_empty() {
        println!("No '{}' additions found since {}.", pattern, date);
        return Ok(());
    }

    // Now find where these lines currently exist in the files (if they still exist)
    // Process in parallel for speed
    let all_matches: Vec<GitMatch> = added_lines
        .par_iter()
        .filter_map(|added| {
            // Check if the file still exists and find the line
            let file_path = directory.join(&added.file);
            if !file_path.exists() {
                return None;
            }

            // Find where this content is now in the file
            find_line_in_current_file(&added.file, &added.content, pattern, &directory).map(
                |(line_number, current_line)| GitMatch {
                    file: added.file.clone(),
                    line_number,
                    line_content: current_line,
                    commit_date: added.commit_date,
                    commit_hash: added.commit_hash.clone(),
                },
            )
        })
        .collect();

    // Deduplicate matches (same file + line number)
    let mut seen = std::collections::HashSet::new();
    let unique_matches: Vec<GitMatch> = all_matches
        .into_iter()
        .filter(|m| seen.insert((m.file.clone(), m.line_number)))
        .collect();

    if unique_matches.is_empty() {
        println!(
            "No '{}' found in lines added since {} (lines may have been removed).",
            pattern, date
        );
        return Ok(());
    }

    println!("Found {} match(es):\n", unique_matches.len());
    print_matches_with_context(&unique_matches, context, &directory)?;

    Ok(())
}
