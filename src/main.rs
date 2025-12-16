use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
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

/// Represents a match found via git blame
#[derive(Debug)]
struct BlameMatch {
    file: String,
    line_number: usize,
    line_content: String,
    commit_date: NaiveDate,
    commit_hash: String,
}

/// Parse git blame output and find matches for the pattern
fn find_matches_with_blame(file: &str, pattern: &str, directory: &Path) -> Result<Vec<BlameMatch>> {
    let file_path = directory.join(file);
    if !file_path.exists() {
        return Ok(vec![]);
    }

    // Run git blame with porcelain format for easy parsing
    let output = Command::new("git")
        .arg("blame")
        .arg("--line-porcelain")
        .arg(file)
        .current_dir(directory)
        .output()
        .context("Failed to execute git blame")?;

    if !output.status.success() {
        // File might not be tracked or other git error, skip it
        return Ok(vec![]);
    }

    let blame_output = String::from_utf8_lossy(&output.stdout);
    let mut matches = Vec::new();

    let mut current_hash = String::new();
    let mut current_line_number = 0usize;
    let mut current_date: Option<NaiveDate> = None;

    for line in blame_output.lines() {
        // Line starting with a hash is the header line
        if line.len() >= 40 && line.chars().take(40).all(|c| c.is_ascii_hexdigit()) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            current_hash = parts[0].to_string();
            if parts.len() >= 2 {
                current_line_number = parts[1].parse().unwrap_or(0);
            }
        } else if line.starts_with("committer-time ") {
            // Parse unix timestamp
            if let Some(timestamp_str) = line.strip_prefix("committer-time ") {
                if let Ok(timestamp) = timestamp_str.trim().parse::<i64>() {
                    current_date =
                        chrono::DateTime::from_timestamp(timestamp, 0).map(|dt| dt.date_naive());
                }
            }
        } else if let Some(content) = line.strip_prefix('\t') {
            // This is the actual line content (prefixed with tab)
            // Remove the leading tab

            if content.contains(pattern) {
                if let Some(date) = current_date {
                    matches.push(BlameMatch {
                        file: file.to_string(),
                        line_number: current_line_number,
                        line_content: content.to_string(),
                        commit_date: date,
                        commit_hash: current_hash.clone(),
                    });
                }
            }
        }
    }

    Ok(matches)
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
    matches: &[BlameMatch],
    context: usize,
    directory: &Path,
) -> Result<()> {
    // Group matches by file
    let mut by_file: HashMap<String, Vec<&BlameMatch>> = HashMap::new();
    for m in matches {
        by_file.entry(m.file.clone()).or_default().push(m);
    }

    let mut first_file = true;
    for (file, file_matches) in by_file {
        if !first_file {
            println!();
        }
        first_file = false;

        let lines = match read_file_lines(&file, directory) {
            Ok(l) => l,
            Err(_) => continue,
        };

        // Sort matches by line number
        let mut sorted_matches = file_matches;
        sorted_matches.sort_by_key(|m| m.line_number);

        // Track which lines we've printed to avoid duplicates in overlapping contexts
        let mut printed_ranges: Vec<(usize, usize)> = Vec::new();

        for m in sorted_matches {
            let start = m.line_number.saturating_sub(context).max(1);
            let end = (m.line_number + context).min(lines.len());

            // Check if this overlaps with already printed range
            let overlaps = printed_ranges.iter().any(|(s, e)| start <= *e && end >= *s);

            if !overlaps {
                // Print file header with commit info
                println!(
                    "\x1b[35m{}\x1b[0m (added \x1b[36m{}\x1b[0m in \x1b[33m{}\x1b[0m)",
                    file,
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
                printed_ranges.push((start, end));
            } else {
                // Just print the match line info if context was already shown
                println!(
                    "\x1b[35m{}\x1b[0m:\x1b[32m{}\x1b[0m: {} (added \x1b[36m{}\x1b[0m)",
                    file,
                    m.line_number,
                    m.line_content.trim(),
                    m.commit_date
                );
            }
        }
    }

    Ok(())
}

fn search_since_date(date: &str, pattern: &str, context: usize, directory: PathBuf) -> Result<()> {
    // Validate and parse date
    let since_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD (e.g., 2025-12-01)")?;

    println!(
        "Searching for '{}' in lines added since {}...\n",
        pattern, date
    );

    // Use git log -S (pickaxe) to find commits that added the pattern since the date
    // This is MUCH faster than blaming every file
    let log_output = Command::new("git")
        .arg("log")
        .arg(format!("--since={}", date))
        .arg("-S")
        .arg(pattern)
        .arg("--format=%H")
        .arg("--diff-filter=AM") // Only additions and modifications
        .arg("--name-only")
        .current_dir(&directory)
        .output()
        .context("Failed to execute git log")?;

    if !log_output.status.success() {
        anyhow::bail!("git log failed. Is this a git repository?");
    }

    // Parse the output to get unique files that have been modified
    let output_str = String::from_utf8_lossy(&log_output.stdout);
    let mut files_to_check = std::collections::HashSet::new();

    for line in output_str.lines() {
        let line = line.trim();
        // Skip empty lines and commit hashes
        if !line.is_empty() && !line.chars().all(|c| c.is_ascii_hexdigit()) {
            files_to_check.insert(line.to_string());
        }
    }

    if files_to_check.is_empty() {
        println!("No files with '{}' changes found since {}.", pattern, date);
        return Ok(());
    }

    let mut all_matches: Vec<BlameMatch> = Vec::new();

    // Only blame files that we know have had changes to the pattern
    for file in files_to_check {
        let matches = find_matches_with_blame(&file, pattern, &directory)?;
        for m in matches {
            if m.commit_date >= since_date {
                all_matches.push(m);
            }
        }
    }

    if all_matches.is_empty() {
        println!("No '{}' found in lines added since {}.", pattern, date);
        return Ok(());
    }

    println!("Found {} match(es):\n", all_matches.len());
    print_matches_with_context(&all_matches, context, &directory)?;

    Ok(())
}
