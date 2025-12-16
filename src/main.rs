use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

    /// Search for TODOs in a specific git commit range
    Range {
        /// Starting commit (e.g., "HEAD~10", "abc123")
        #[arg(short, long)]
        from: String,

        /// Ending commit (default: "HEAD")
        #[arg(short, long, default_value = "HEAD")]
        to: String,

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

        Commands::Range {
            from,
            to,
            pattern,
            context,
            directory,
        } => search_commit_range(&from, &to, &pattern, context, directory)?,
    }

    Ok(())
}

fn search_current_files(
    pattern: &str,
    context: usize,
    file_type: Option<String>,
    directory: PathBuf,
) -> Result<()> {
    println!("🔍 Searching for '{}' in current files...\n", pattern);

    let mut cmd = Command::new("rg");
    cmd.arg(pattern)
        .arg(format!("-C{}", context))
        .arg("--color=always")
        .arg("--heading")
        .arg("--line-number");

    if let Some(ft) = file_type {
        cmd.arg("-g").arg(ft);
    }

    cmd.arg(directory);

    let output = cmd
        .output()
        .context("Failed to execute ripgrep. Is 'rg' installed?")?;

    if output.status.success() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No matches found") || output.stdout.is_empty() {
            println!("No matches found.");
        } else {
            eprintln!("{}", stderr);
        }
    }

    Ok(())
}

fn search_since_date(date: &str, pattern: &str, context: usize, directory: PathBuf) -> Result<()> {
    // Validate date format
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD (e.g., 2025-12-01)")?;

    println!("Searching for '{}' added since {}...\n", pattern, date);

    let git_cmd = format!(
        "git log --since=\"{}\" -p --pretty=format:\"%h %ad %s\" --date=short",
        date
    );

    let mut git = Command::new("sh")
        .arg("-c")
        .arg(&git_cmd)
        .current_dir(directory)
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to execute git command. Is this a git repository?")?;

    let git_stdout = git.stdout.take().context("Failed to capture git output")?;

    let grep = Command::new("grep")
        .arg(format!("-C{}", context))
        .arg("--color=always")
        .arg(pattern)
        .stdin(Stdio::from(git_stdout))
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to execute grep")?;

    let output = grep
        .wait_with_output()
        .context("Failed to read grep output")?;
    git.wait()?;

    if output.status.success() && !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    } else {
        println!("No matches found.");
    }

    Ok(())
}

fn search_commit_range(
    from: &str,
    to: &str,
    pattern: &str,
    context: usize,
    directory: PathBuf,
) -> Result<()> {
    println!(
        "Searching for '{}' in commits {}..{}...\n",
        pattern, from, to
    );

    let git_cmd = format!(
        "git log {}..{} -p --pretty=format:\"%h %ad %s\" --date=short",
        from, to
    );

    let mut git = Command::new("sh")
        .arg("-c")
        .arg(&git_cmd)
        .current_dir(directory)
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to execute git command. Is this a git repository?")?;

    let git_stdout = git.stdout.take().context("Failed to capture git output")?;

    let grep = Command::new("grep")
        .arg(format!("-C{}", context))
        .arg("--color=always")
        .arg(pattern)
        .stdin(Stdio::from(git_stdout))
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to execute grep")?;

    let output = grep
        .wait_with_output()
        .context("Failed to read grep output")?;
    git.wait()?;

    if output.status.success() && !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    } else {
        println!("No matches found.");
    }

    Ok(())
}
