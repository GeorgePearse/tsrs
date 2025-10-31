//! CLI for tree-shaking operations
#![allow(
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args,
    clippy::map_unwrap_or,
    clippy::redundant_closure_for_method_calls,
    clippy::ptr_arg,
    clippy::manual_let_else,
    clippy::field_reassign_with_default,
    clippy::too_many_arguments,
    clippy::fn_params_excessive_bools,
    clippy::explicit_iter_loop,
    clippy::single_match_else
)]

use anyhow::Context;
use clap::{ArgAction, Parser, Subcommand};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};
use similar::TextDiff;
use globset::{Glob, GlobSet, GlobSetBuilder};
use num_cpus;
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::filter::EnvFilter;
use tsrs::{Minifier, MinifyPlan, VenvAnalyzer, VenvSlimmer};
use walkdir::WalkDir;

const DEFAULT_EXCLUDES: &[&str] = &["**/.git/**", "**/__pycache__/**", "**/.venv/**"];

#[derive(Parser)]
#[command(name = "tsrs")]
#[command(about = "Tree-shaking in Rust for Python", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Reduce logging to warnings and errors
    #[arg(global = true, short = 'q', long = "quiet")]
    quiet: bool,

    /// Increase logging verbosity (-v, -vv)
    #[arg(global = true, short = 'v', long = "verbose", action = ArgAction::Count)]
    verbose: u8,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a virtual environment
    Analyze {
        /// Path to the virtual environment
        #[arg(value_name = "VENV_PATH")]
        venv_path: PathBuf,
    },

    /// Create a slim version of a virtual environment based on code imports
    Slim {
        /// Path to the Python code directory to analyze
        #[arg(value_name = "PYTHON_DIRECTORY")]
        code_path: PathBuf,

        /// Path to the source virtual environment
        #[arg(value_name = "VENV_PATH")]
        venv_path: PathBuf,

        /// Path for the output slim venv (default: .venv-slim)
        #[arg(short, long, value_name = "OUTPUT_PATH")]
        output: Option<PathBuf>,
    },

    /// Print a planned rename map for locals in a Python file
    MinifyPlan {
        /// Path to the Python source file
        #[arg(value_name = "PYTHON_FILE")]
        python_file: PathBuf,
    },

    /// Generate rename plans for every Python file in a directory tree
    MinifyPlanDir {
        /// Directory containing Python sources to analyze
        #[arg(value_name = "INPUT_DIR")]
        input_dir: PathBuf,

        /// Path where the plan bundle JSON should be written
        #[arg(long, value_name = "PLAN_FILE")]
        out: PathBuf,

        /// Glob pattern to include (repeatable). Defaults to "**/*.py"
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,

        /// Glob pattern to exclude (repeatable)
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,

        /// Limit parallel workers when planning
        #[arg(long, value_name = "N")]
        jobs: Option<usize>,
    },

    /// Apply a precomputed rename plan to a Python file
    ApplyPlan {
        /// Path to the Python source file
        #[arg(value_name = "PYTHON_FILE")]
        python_file: PathBuf,

        /// Path to the JSON plan file produced by `minify-plan`
        #[arg(long, value_name = "PLAN_FILE")]
        plan: PathBuf,

        /// Rewrite the file in place instead of printing the rewritten code
        #[arg(long)]
        in_place: bool,

        /// Create a backup of the original file with the given suffix (requires --in-place)
        #[arg(long, value_name = "EXT")]
        backup_ext: Option<String>,

        /// Print rename statistics for the file
        #[arg(long)]
        stats: bool,

        /// Emit rename statistics in JSON format (requires --stats)
        #[arg(long)]
        json: bool,

        /// Write stats summary to a JSON file
        #[arg(long, value_name = "JSON_FILE")]
        output_json: Option<PathBuf>,

        /// Exit with a non-zero status if any bailouts occur
        #[arg(long)]
        fail_on_bailout: bool,

        /// Exit with a non-zero status if any errors occur
        #[arg(long)]
        fail_on_error: bool,

        /// Exit with a non-zero status if any changes are made
        #[arg(long)]
        fail_on_change: bool,

        /// Show unified diffs for rewritten files
        #[arg(long)]
        diff: bool,
    },

    /// Apply precomputed rename plans to every file in a directory tree
    ApplyPlanDir {
        /// Directory containing Python sources to process
        #[arg(value_name = "INPUT_DIR")]
        input_dir: PathBuf,

        /// Path to the JSON plan bundle produced by `minify-plan-dir`
        #[arg(long, value_name = "PLAN_FILE")]
        plan: PathBuf,

        /// Directory where rewritten files should be written
        #[arg(long, value_name = "OUTPUT_DIR")]
        out_dir: Option<PathBuf>,

        /// Rewrite files in place instead of mirroring to an output directory
        #[arg(long)]
        in_place: bool,

        /// Perform a dry run and print status without writing files
        #[arg(long)]
        dry_run: bool,

        /// Create a backup of rewritten files with the given suffix (requires --in-place)
        #[arg(long, value_name = "EXT")]
        backup_ext: Option<String>,

        /// Glob pattern to include (repeatable). Defaults to "**/*.py"
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,

        /// Glob pattern to exclude (repeatable)
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,

        /// Print per-file rename counts and totals in the summary
        #[arg(long)]
        stats: bool,

        /// Emit stats summary as JSON (requires --stats)
        #[arg(long)]
        json: bool,

        /// Write stats summary to a JSON file
        #[arg(long, value_name = "JSON_FILE")]
        output_json: Option<PathBuf>,

        /// Limit parallel workers when rewriting files
        #[arg(long, value_name = "N")]
        jobs: Option<usize>,

        /// Exit with a non-zero status if any bailouts occur
        #[arg(long)]
        fail_on_bailout: bool,

        /// Exit with a non-zero status if any errors occur
        #[arg(long)]
        fail_on_error: bool,

        /// Exit with a non-zero status if any changes are made
        #[arg(long)]
        fail_on_change: bool,

        /// Show unified diffs for rewritten files
        #[arg(long)]
        diff: bool,
    },

    /// Rewrite a Python file using safe local renames
    Minify {
        /// Path to the Python source file
        #[arg(value_name = "PYTHON_FILE")]
        python_file: PathBuf,

        /// Rewrite the file in place instead of printing the rewritten code
        #[arg(long)]
        in_place: bool,

        /// Create a backup of the original file with the given suffix (requires --in-place)
        #[arg(long, value_name = "EXT")]
        backup_ext: Option<String>,

        /// Print rename statistics for the file
        #[arg(long)]
        stats: bool,

        /// Emit rename statistics in JSON format (requires --stats)
        #[arg(long)]
        json: bool,

        /// Write stats summary to a JSON file
        #[arg(long, value_name = "JSON_FILE")]
        output_json: Option<PathBuf>,

        /// Exit with a non-zero status if any bailouts occur
        #[arg(long)]
        fail_on_bailout: bool,

        /// Exit with a non-zero status if any errors occur
        #[arg(long)]
        fail_on_error: bool,

        /// Exit with a non-zero status if any changes are made
        #[arg(long)]
        fail_on_change: bool,

        /// Show unified diffs for rewritten files
        #[arg(long)]
        diff: bool,
    },

    /// Rewrite all Python files in a directory tree using safe local renames
    MinifyDir {
        /// Directory containing Python sources to process
        #[arg(value_name = "INPUT_DIR")]
        input_dir: PathBuf,

        /// Directory where rewritten files should be written
        #[arg(long, value_name = "OUTPUT_DIR")]
        out_dir: Option<PathBuf>,

        /// Rewrite files in place instead of mirroring to an output directory
        #[arg(long)]
        in_place: bool,

        /// Perform a dry run and print status without writing files
        #[arg(long)]
        dry_run: bool,

        /// Create a backup of rewritten files with the given suffix (requires --in-place)
        #[arg(long, value_name = "EXT")]
        backup_ext: Option<String>,

        /// Glob pattern to include (repeatable). Defaults to "**/*.py"
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,

        /// Glob pattern to exclude (repeatable)
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,

        /// Print per-file rename counts and totals in the summary
        #[arg(long)]
        stats: bool,

        /// Emit stats summary as JSON (requires --stats)
        #[arg(long)]
        json: bool,

        /// Write stats summary to a JSON file
        #[arg(long, value_name = "JSON_FILE")]
        output_json: Option<PathBuf>,

        /// Limit parallel workers when rewriting files
        #[arg(long, value_name = "N")]
        jobs: Option<usize>,

        /// Exit with a non-zero status if any bailouts occur
        #[arg(long)]
        fail_on_bailout: bool,

        /// Exit with a non-zero status if any errors occur
        #[arg(long)]
        fail_on_error: bool,

        /// Exit with a non-zero status if any changes are made
        #[arg(long)]
        fail_on_change: bool,

        /// Show unified diffs for rewritten files
        #[arg(long)]
        diff: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let level = if cli.quiet {
        "warn"
    } else if cli.verbose >= 2 {
        "debug"
    } else {
        "info"
    };
    let env_filter = EnvFilter::new(level);

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    match cli.command {
        Commands::Analyze { venv_path } => {
            analyze(&venv_path)?;
        }
        Commands::Slim {
            code_path,
            venv_path,
            output,
        } => {
            slim(&code_path, &venv_path, output)?;
        }
        Commands::MinifyPlan { python_file } => {
            minify_plan(&python_file)?;
        }
        Commands::MinifyPlanDir {
            input_dir,
            out,
            include,
            exclude,
            jobs,
        } => {
            minify_plan_dir(&input_dir, &out, &include, &exclude, jobs, cli.quiet)?;
        }
        Commands::Minify {
            python_file,
            in_place,
            backup_ext,
            stats,
            json,
            output_json,
            fail_on_bailout,
            fail_on_error,
            fail_on_change,
            diff,
        } => {
            let stats_result = minify(
                &python_file,
                in_place,
                backup_ext.as_deref(),
                stats,
                json,
                cli.quiet,
                output_json.as_deref(),
                fail_on_bailout,
                fail_on_error,
                fail_on_change,
                diff,
            )?;

            if fail_on_bailout || fail_on_error || fail_on_change {
                let code = compute_exit_code(
                    &stats_result,
                    fail_on_bailout,
                    fail_on_error,
                    fail_on_change,
                );
                process::exit(code);
            }
        }
        Commands::ApplyPlan {
            python_file,
            plan,
            in_place,
            backup_ext,
            stats,
            json,
            output_json,
            fail_on_bailout,
            fail_on_error,
            fail_on_change,
            diff,
        } => {
            let stats_result = apply_plan(
                &python_file,
                &plan,
                in_place,
                backup_ext.as_deref(),
                stats,
                json,
                cli.quiet,
                output_json.as_deref(),
                fail_on_bailout,
                fail_on_error,
                fail_on_change,
                diff,
            )?;

            if fail_on_bailout || fail_on_error || fail_on_change {
                let code = compute_exit_code(
                    &stats_result,
                    fail_on_bailout,
                    fail_on_error,
                    fail_on_change,
                );
                process::exit(code);
            }
        }
        Commands::MinifyDir {
            input_dir,
            out_dir,
            in_place,
            dry_run,
            backup_ext,
            include,
            exclude,
            stats,
            json,
            output_json,
            jobs,
            fail_on_bailout,
            fail_on_error,
            fail_on_change,
            diff,
        } => {
            let stats_result = minify_dir(
                &input_dir,
                out_dir,
                &include,
                &exclude,
                backup_ext.as_deref(),
                in_place,
                dry_run,
                stats,
                json,
                cli.quiet,
                output_json.as_deref(),
                jobs,
                fail_on_bailout,
                fail_on_error,
                fail_on_change,
                diff,
            )?;

            if fail_on_bailout || fail_on_error || fail_on_change {
                let code = compute_exit_code(
                    &stats_result,
                    fail_on_bailout,
                    fail_on_error,
                    fail_on_change,
                );
                process::exit(code);
            }
        }
        Commands::ApplyPlanDir {
            input_dir,
            plan,
            out_dir,
            in_place,
            dry_run,
            backup_ext,
            include,
            exclude,
            stats,
            json,
            output_json,
            jobs,
            fail_on_bailout,
            fail_on_error,
            fail_on_change,
            diff,
        } => {
            let stats_result = apply_plan_dir(
                &input_dir,
                &plan,
                out_dir,
                &include,
                &exclude,
                backup_ext.as_deref(),
                in_place,
                dry_run,
                stats,
                json,
                cli.quiet,
                output_json.as_deref(),
                jobs,
                fail_on_bailout,
                fail_on_error,
                fail_on_change,
                diff,
            )?;

            if fail_on_bailout || fail_on_error || fail_on_change {
                let code = compute_exit_code(
                    &stats_result,
                    fail_on_bailout,
                    fail_on_error,
                    fail_on_change,
                );
                process::exit(code);
            }
        }
    }

    Ok(())
}

fn analyze(venv_path: &PathBuf) -> anyhow::Result<()> {
    println!("Analyzing venv at: {}", venv_path.display());

    let analyzer = VenvAnalyzer::new(venv_path)?;
    let info = analyzer.analyze()?;

    println!("\nVenv Information:");
    println!("  Path: {}", info.path.display());
    if let Some(version) = info.python_version {
        println!("  Python Version: {}", version);
    }
    println!("  Packages: {}", info.packages.len());
    println!("\nInstalled Packages:");
    for package in &info.packages {
        print!("  - {}", package.name);
        if let Some(version) = &package.version {
            print!(" ({})", version);
        }
        println!();
    }

    Ok(())
}

fn slim(code_path: &PathBuf, venv_path: &PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
    let output_path = output.unwrap_or_else(|| {
        let parent = venv_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let mut path = parent;
        path.push(".venv-slim");
        path
    });

    println!("Creating slim venv...");
    println!("  Code directory: {}", code_path.display());
    println!("  Source venv: {}", venv_path.display());
    println!("  Output venv: {}", output_path.display());

    let slimmer = VenvSlimmer::new_with_output(code_path, venv_path, &output_path)?;
    slimmer.slim()?;

    println!("\nSlim venv created successfully!");
    println!("Output: {}", output_path.display());

    Ok(())
}

fn minify_plan(file_path: &PathBuf) -> anyhow::Result<()> {
    let source = fs::read_to_string(file_path)?;
    let module_name = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_path.to_string_lossy().to_string());

    let plan = Minifier::plan_from_source(&module_name, &source)?;
    let plan_json = serde_json::to_string_pretty(&plan)?;
    println!("{}", plan_json);

    Ok(())
}

fn minify(
    file_path: &PathBuf,
    in_place: bool,
    backup_ext: Option<&str>,
    show_stats: bool,
    json_output: bool,
    quiet: bool,
    output_json: Option<&Path>,
    fail_on_bailout: bool,
    fail_on_error: bool,
    fail_on_change: bool,
    diff: bool,
) -> anyhow::Result<DirStats> {
    minify_file(
        file_path,
        in_place,
        backup_ext,
        show_stats,
        json_output,
        quiet,
        output_json,
        fail_on_bailout,
        fail_on_error,
        fail_on_change,
        diff,
    )
}

fn apply_plan(
    file_path: &PathBuf,
    plan_path: &PathBuf,
    in_place: bool,
    backup_ext: Option<&str>,
    show_stats: bool,
    json_output: bool,
    quiet: bool,
    output_json: Option<&Path>,
    fail_on_bailout: bool,
    fail_on_error: bool,
    fail_on_change: bool,
    diff: bool,
) -> anyhow::Result<DirStats> {
    if json_output && !show_stats {
        anyhow::bail!("--json requires --stats");
    }

    let plan_file = fs::read_to_string(plan_path)?;
    let plan: MinifyPlan = serde_json::from_str(&plan_file)?;

    apply_plan_to_file(
        file_path,
        &plan,
        in_place,
        backup_ext,
        show_stats,
        json_output,
        quiet,
        output_json,
        fail_on_bailout,
        fail_on_error,
        fail_on_change,
        diff,
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DirStats {
    processed: usize,
    rewritten: usize,
    skipped_no_change: usize,
    bailouts: usize,
    errors: usize,
    total_renames: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileStats>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    reasons: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileStats {
    path: String,
    renames: usize,
    status: String,
}

fn print_file_status(path: &str, status: &str, renames: usize, show_stats: bool, quiet: bool) {
    if quiet {
        return;
    }
    if show_stats {
        println!("• {} → {} (renames: {})", path, status, renames);
    } else {
        println!("• {} → {}", path, status);
    }
}

fn print_summary(
    stats: &DirStats,
    show_stats: bool,
    json_output: bool,
    dry_run: bool,
    output_label: &str,
    output_json: Option<&Path>,
) -> anyhow::Result<()> {
    let message = if dry_run {
        if show_stats {
            format!(
                "Dry run complete: {} files matched → {} minified, {} skipped, {} bailouts, {} errors, {} renames. Output: {}",
                stats.processed,
                stats.rewritten,
                stats.skipped_no_change,
                stats.bailouts,
                stats.errors,
                stats.total_renames,
                output_label,
            )
        } else {
            format!(
                "Dry run complete: {} files matched → {} minified, {} skipped, {} bailouts, {} errors. Output: {}",
                stats.processed,
                stats.rewritten,
                stats.skipped_no_change,
                stats.bailouts,
                stats.errors,
                output_label,
            )
        }
    } else if show_stats {
        format!(
            "Processed {} files → {} minified, {} skipped, {} bailouts, {} errors, {} renames. Output: {}",
            stats.processed,
            stats.rewritten,
            stats.skipped_no_change,
            stats.bailouts,
            stats.errors,
            stats.total_renames,
            output_label,
        )
    } else {
        format!(
            "Processed {} files → {} minified, {} skipped, {} bailouts, {} errors. Output: {}",
            stats.processed,
            stats.rewritten,
            stats.skipped_no_change,
            stats.bailouts,
            stats.errors,
            output_label,
        )
    };

    println!("{}", message);
    info!("{}", message);

    if show_stats && json_output {
        println!("{}", serde_json::to_string_pretty(stats)?);
    }

    if let Some(path) = output_json {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let file = fs::File::create(path)?;
        serde_json::to_writer_pretty(file, stats)?;
    }

    Ok(())
}

fn compute_exit_code(
    stats: &DirStats,
    fail_on_bailout: bool,
    fail_on_error: bool,
    fail_on_change: bool,
) -> i32 {
    let mut code = 0;
    if fail_on_error && stats.errors > 0 {
        code |= 1;
    }
    if fail_on_bailout && stats.bailouts > 0 {
        code |= 2;
    }
    if fail_on_change && stats.rewritten > 0 {
        code |= 4;
    }
    code
}

fn bump_reason(stats: &mut DirStats, reason: &str) {
    *stats.reasons.entry(reason.to_string()).or_insert(0) += 1;
}

fn detect_pep263_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    fn extract(line: &str) -> Option<&'static Encoding> {
        if !line.trim_start().starts_with('#') {
            return None;
        }
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("coding") {
            let mut rest = &line[idx + "coding".len()..];
            rest = rest.trim_start_matches(|c: char| matches!(c, ' ' | '\t' | ':' | '=' | '-' | '*'));
            let label: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !label.is_empty() {
                return Encoding::for_label(label.trim().as_bytes());
            }
        }
        None
    }

    let mut lines = bytes.split(|&b| b == b'\n');
    for _ in 0..2 {
        if let Some(line_bytes) = lines.next() {
            if let Ok(line_str) = std::str::from_utf8(line_bytes) {
                if let Some(enc) = extract(line_str) {
                    return Some(enc);
                }
            }
        }
    }
    None
}

fn read_python(path: &Path) -> anyhow::Result<(String, Option<&'static Encoding>)> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let mut encoding = if bytes.starts_with(b"\xEF\xBB\xBF") {
        Some(UTF_8)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some(UTF_16LE)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some(UTF_16BE)
    } else {
        detect_pep263_encoding(&bytes)
    };

    let effective = encoding.unwrap_or(UTF_8);
    let (decoded, had_errors) = effective.decode_without_bom_handling(&bytes);
    if had_errors {
        anyhow::bail!(
            "failed to decode {} using {}",
            path.display(),
            effective.name()
        );
    }

    Ok((decoded.into_owned(), encoding))
}

fn write_python(
    path: &Path,
    content: &str,
    encoding: Option<&'static Encoding>,
) -> anyhow::Result<()> {
    let encoder = encoding.unwrap_or(UTF_8);
    let (encoded, output_encoding, had_errors) = encoder.encode(content);
    if had_errors || !std::ptr::eq(output_encoding, encoder) {
        anyhow::bail!(
            "failed to encode {} using {}",
            path.display(),
            encoder.name()
        );
    }
    match encoded {
        Cow::Borrowed(bytes) => fs::write(path, bytes)?,
        Cow::Owned(buffer) => fs::write(path, buffer)?,
    }
    Ok(())
}

fn make_unified_diff(path: &str, original: &str, rewritten: &str) -> String {
    let diff = TextDiff::from_lines(original, rewritten);
    diff.unified_diff()
        .header(&format!("a/{}", path), &format!("b/{}", path))
        .to_string()
}

const PLAN_BUNDLE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct PlanBundle {
    #[serde(default = "default_plan_version")]
    version: u32,
    files: Vec<PlanFile>,
}

fn default_plan_version() -> u32 {
    PLAN_BUNDLE_VERSION
}

#[derive(Debug, Serialize, Deserialize)]
struct PlanFile {
    path: String,
    plan: MinifyPlan,
}

fn minify_file(
    file_path: &PathBuf,
    in_place: bool,
    backup_ext: Option<&str>,
    show_stats: bool,
    json_output: bool,
    quiet: bool,
    output_json: Option<&Path>,
    fail_on_bailout: bool,
    fail_on_error: bool,
    fail_on_change: bool,
    diff: bool,
) -> anyhow::Result<DirStats> {
    if json_output && !show_stats {
        anyhow::bail!("--json requires --stats");
    }

    let (source, _) = read_python(file_path)?;
    let module_name = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_path.to_string_lossy().to_string());

    let plan = Minifier::plan_from_source(&module_name, &source)?;
    apply_plan_to_file(
        file_path,
        &plan,
        in_place,
        backup_ext,
        show_stats,
        json_output,
        quiet,
        output_json,
        fail_on_bailout,
        fail_on_error,
        fail_on_change,
        diff,
    )
}

fn apply_plan_to_file(
    file_path: &PathBuf,
    plan: &MinifyPlan,
    in_place: bool,
    backup_ext: Option<&str>,
    show_stats: bool,
    json_output: bool,
    quiet: bool,
    output_json: Option<&Path>,
    fail_on_bailout: bool,
    fail_on_error: bool,
    fail_on_change: bool,
    diff: bool,
) -> anyhow::Result<DirStats> {
    if json_output && !show_stats {
        anyhow::bail!("--json requires --stats");
    }

    if backup_ext.is_some() && !in_place {
        anyhow::bail!("--backup-ext requires --in-place");
    }

    let (source, encoding) = read_python(file_path)?;
    let module_name = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_path.to_string_lossy().to_string());

    let rename_total: usize = plan.functions.iter().map(|f| f.renames.len()).sum();

    let mut status;
    let mut final_content: Cow<'_, str> = Cow::Borrowed(&source);

    if rename_total == 0 {
        status = "skipped (no renames)".to_string();
    } else {
        let rewritten = Minifier::rewrite_with_plan(&module_name, &source, plan)?;
        if rewritten == source {
            status = "skipped (rewrite aborted)".to_string();
        } else {
            status = "minified".to_string();
            final_content = Cow::Owned(rewritten);
        }
    }

    let display_path = file_path.display().to_string();

    if in_place {
        if let Some(ext) = backup_ext {
            let mut backup_os = file_path.as_os_str().to_os_string();
            backup_os.push(ext);
            let backup_path = PathBuf::from(backup_os);
            if backup_path.exists() {
                status = "skipped (backup exists)".to_string();
                final_content = Cow::Borrowed(&source);
            } else {
                write_python(&backup_path, &source, encoding)?;
            }
        }

        if let Cow::Owned(ref content) = final_content {
            write_python(file_path, content, encoding)?;
        }
    }

    let applied_renames = if matches!(status.as_str(), "minified") {
        rename_total
    } else {
        0
    };

    if show_stats {
        print_file_status(&display_path, &status, applied_renames, true, quiet);
    } else if in_place {
        print_file_status(&display_path, &status, applied_renames, false, quiet);
    }

    if diff && matches!(status.as_str(), "minified") {
        let diff_str = make_unified_diff(&display_path, &source, final_content.as_ref());
        println!("{}", diff_str);
    }

    if !in_place && !show_stats {
        println!("{}", final_content);
    }

    let mut stats = DirStats::default();
    stats.processed = 1;
    stats.total_renames = applied_renames;
    match status.as_str() {
        "minified" => {
            stats.rewritten = 1;
            bump_reason(&mut stats, "minified");
        }
        "skipped (no renames)" => {
            stats.skipped_no_change = 1;
            bump_reason(&mut stats, "no_renames");
        }
        "skipped (rewrite aborted)" => {
            stats.bailouts = 1;
            bump_reason(&mut stats, "rewrite_aborted");
        }
        "skipped (backup exists)" => {
            stats.bailouts = 1;
            bump_reason(&mut stats, "backup_exists");
        }
        _ => {
            stats.bailouts = 1;
        }
    }
    stats.files.push(FileStats {
        path: display_path.clone(),
        renames: applied_renames,
        status: status.clone(),
    });

    let summary_needed =
        show_stats || fail_on_bailout || fail_on_error || fail_on_change || output_json.is_some();
    if summary_needed {
        let output_target = if in_place {
            display_path.clone()
        } else {
            "stdout".to_string()
        };
        print_summary(
            &stats,
            show_stats,
            json_output,
            false,
            &output_target,
            output_json,
        )?;
    }

    Ok(stats)
}

fn minify_plan_dir(
    input_dir: &PathBuf,
    out_path: &PathBuf,
    includes: &[String],
    excludes: &[String],
    jobs: Option<usize>,
    quiet: bool,
) -> anyhow::Result<()> {
    let input_dir = input_dir.canonicalize()?;
    if !input_dir.is_dir() {
        anyhow::bail!("Input '{}' is not a directory", input_dir.display());
    }

    let include_patterns: Vec<String> = if includes.is_empty() {
        vec!["**/*.py".to_string()]
    } else {
        includes.to_vec()
    };
    let include_glob = build_globset(&include_patterns)?;
    let exclude_patterns = merged_exclude_patterns(excludes);
    let exclude_glob = build_globset(&exclude_patterns)?;

    let mut errors = 0usize;
    let mut candidates: Vec<Candidate> = Vec::new();

    for entry in WalkDir::new(&input_dir).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                errors += 1;
                warn!("walk error: {}", err);
                continue;
            }
        };

        if entry.file_type().is_dir() || entry.file_type().is_symlink() {
            continue;
        }

        let path = entry.path();
        let rel_path = match path.strip_prefix(&input_dir) {
            Ok(rel) => rel,
            Err(_) => continue,
        };

        let rel_norm = normalize_rel_path(rel_path);

        if rel_path
            .components()
            .any(|comp| matches!(comp, std::path::Component::Normal(os) if os.to_string_lossy().starts_with('.')))
        {
            debug!("• {} → skipped (hidden path)", rel_norm);
            continue;
        }

        if !include_glob.is_match(rel_norm.as_str()) {
            debug!("• {} → skipped (not included)", rel_norm);
            continue;
        }
        if exclude_glob.is_match(rel_norm.as_str()) {
            debug!("• {} → skipped (excluded)", rel_norm);
            continue;
        }

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("py"))
            != Some(true)
        {
            debug!("• {} → skipped (non-Python)", rel_norm);
            continue;
        }

        candidates.push(Candidate {
            abs_path: path.to_path_buf(),
            rel_path: rel_path.to_path_buf(),
            rel_norm,
        });
    }

    let jobs = resolve_jobs(jobs)?;

    #[derive(Debug)]
    enum PlanOutcome {
        Success { plan: MinifyPlan, renames: usize },
        ReadError(String),
        PlanError(String),
    }

    candidates.sort_by(|a, b| a.rel_norm.cmp(&b.rel_norm));

    let plan_results: Vec<(Candidate, PlanOutcome)> = if candidates.is_empty() {
        Vec::new()
    } else if jobs <= 1 {
        candidates
            .iter()
            .map(|candidate| (candidate.clone(), compute_plan(candidate)))
            .collect()
    } else {
        let pool = ThreadPoolBuilder::new().num_threads(jobs).build()?;
        pool.install(|| {
            candidates
                .par_iter()
                .map(|candidate| (candidate.clone(), compute_plan(candidate)))
                .collect()
        })
    };

    fn compute_plan(candidate: &Candidate) -> PlanOutcome {
        let source = match fs::read_to_string(&candidate.abs_path) {
            Ok(content) => content,
            Err(err) => return PlanOutcome::ReadError(err.to_string()),
        };

        let module_name = derive_module_name(&candidate.rel_path);
        let plan = match Minifier::plan_from_source(&module_name, &source) {
            Ok(plan) => plan,
            Err(err) => return PlanOutcome::PlanError(err.to_string()),
        };

        let renames = plan.functions.iter().map(|f| f.renames.len()).sum();
        PlanOutcome::Success { plan, renames }
    }

    let mut plans: Vec<PlanFile> = Vec::new();

    for (candidate, outcome) in plan_results {
        match outcome {
            PlanOutcome::Success { plan, renames } => {
                print_file_status(&candidate.rel_norm, "planned", renames, true, quiet);
                plans.push(PlanFile {
                    path: candidate.rel_norm,
                    plan,
                });
            }
            PlanOutcome::ReadError(message) => {
                errors += 1;
                error!(
                    "failed to read {}: {}",
                    candidate.abs_path.display(),
                    message
                );
            }
            PlanOutcome::PlanError(message) => {
                errors += 1;
                error!(
                    "failed to plan {}: {}",
                    candidate.abs_path.display(),
                    message
                );
            }
        }
    }

    plans.sort_by(|a, b| a.path.cmp(&b.path));
    let planned_count = plans.len();

    if planned_count == 0 {
        warn!("no files matched the provided filters; writing empty plan bundle");
    }

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let bundle = PlanBundle {
        version: PLAN_BUNDLE_VERSION,
        files: plans,
    };
    fs::write(out_path, serde_json::to_string_pretty(&bundle)?)?;

    println!(
        "Planned {} files ({} errors). Output: {}",
        planned_count,
        errors,
        out_path.display()
    );

    Ok(())
}

fn apply_plan_dir(
    input_dir: &PathBuf,
    plan_path: &PathBuf,
    out_dir: Option<PathBuf>,
    includes: &[String],
    excludes: &[String],
    backup_ext: Option<&str>,
    in_place: bool,
    dry_run: bool,
    show_stats: bool,
    json_output: bool,
    quiet: bool,
    output_json: Option<&Path>,
    jobs: Option<usize>,
    fail_on_bailout: bool,
    fail_on_error: bool,
    fail_on_change: bool,
    diff: bool,
) -> anyhow::Result<DirStats> {
    if json_output && !show_stats {
        anyhow::bail!("--json requires --stats");
    }

    let input_dir = input_dir.canonicalize()?;
    if !input_dir.is_dir() {
        anyhow::bail!("Input '{}' is not a directory", input_dir.display());
    }

    if backup_ext.is_some() && !in_place {
        anyhow::bail!("--backup-ext requires --in-place");
    }

    if in_place && out_dir.is_some() {
        anyhow::bail!("Cannot use --out-dir with --in-place");
    }

    let plan_contents = fs::read_to_string(plan_path)?;
    let bundle: PlanBundle = serde_json::from_str(&plan_contents)?;
    if bundle.version > PLAN_BUNDLE_VERSION {
        anyhow::bail!(
            "unsupported plan bundle version: {} (supported: {})",
            bundle.version,
            PLAN_BUNDLE_VERSION
        );
    }
    let mut plan_map: HashMap<String, MinifyPlan> = HashMap::new();
    for file_plan in bundle.files {
        plan_map.insert(file_plan.path, file_plan.plan);
    }

    if plan_map.is_empty() {
        anyhow::bail!("Plan bundle contains no files");
    }

    let plan_map = Arc::new(plan_map);

    let resolved_out_dir = if in_place {
        input_dir.clone()
    } else {
        out_dir.unwrap_or_else(|| default_output_dir(&input_dir))
    };

    if !in_place {
        let resolved_abs = if resolved_out_dir.is_absolute() {
            resolved_out_dir.clone()
        } else {
            std::env::current_dir()?.join(&resolved_out_dir)
        };

        if resolved_abs.starts_with(&input_dir) {
            anyhow::bail!("--out-dir cannot be inside the input directory");
        }

        if resolved_out_dir.exists() {
            if !resolved_out_dir.is_dir() {
                anyhow::bail!(
                    "Output '{}' exists and is not a directory",
                    resolved_out_dir.display()
                );
            }
            if !dry_run && resolved_out_dir.read_dir()?.next().is_some() {
                anyhow::bail!(
                    "Output directory '{}' already exists and is not empty",
                    resolved_out_dir.display()
                );
            }
        } else if !dry_run {
            fs::create_dir_all(&resolved_out_dir)?;
        }
    }

    let include_patterns: Vec<String> = if includes.is_empty() {
        vec!["**/*.py".to_string()]
    } else {
        includes.to_vec()
    };
    let include_glob = build_globset(&include_patterns)?;
    let exclude_patterns = merged_exclude_patterns(excludes);
    let exclude_glob = build_globset(&exclude_patterns)?;

    let jobs = resolve_jobs(jobs)?;

    let mut stats = DirStats::default();
    let mut candidates: Vec<Candidate> = Vec::new();

    for entry in WalkDir::new(&input_dir).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                stats.errors += 1;
                warn!("walk error: {}", err);
                continue;
            }
        };

        if entry.file_type().is_dir() || entry.file_type().is_symlink() {
            continue;
        }

        let path = entry.path();
        let rel_path = match path.strip_prefix(&input_dir) {
            Ok(rel) => rel,
            Err(_) => continue,
        };

        let rel_norm = normalize_rel_path(rel_path);

        if rel_path
            .components()
            .any(|comp| matches!(comp, std::path::Component::Normal(os) if os.to_string_lossy().starts_with('.')))
        {
            debug!("• {} → skipped (hidden path)", rel_norm);
            continue;
        }

        if !include_glob.is_match(rel_norm.as_str()) {
            debug!("• {} → skipped (not included)", rel_norm);
            continue;
        }
        if exclude_glob.is_match(rel_norm.as_str()) {
            debug!("• {} → skipped (excluded)", rel_norm);
            continue;
        }

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("py"))
            != Some(true)
        {
            debug!("• {} → skipped (non-Python)", rel_norm);
            continue;
        }

        if !plan_map.contains_key(&rel_norm) {
            debug!("• {} → skipped (no plan)", rel_norm);
            continue;
        }

        candidates.push(Candidate {
            abs_path: path.to_path_buf(),
            rel_path: rel_path.to_path_buf(),
            rel_norm,
        });
    }

    candidates.sort_by(|a, b| a.rel_norm.cmp(&b.rel_norm));

    stats.processed = candidates.len();

    let processor = {
        let plan_map = Arc::clone(&plan_map);
        move |candidate: &Candidate| -> FileResult {
            let candidate_clone = candidate.clone();
            let (source, encoding) = match read_python(&candidate.abs_path) {
                Ok(result) => result,
                Err(err) => {
                    return FileResult {
                        candidate: candidate_clone,
                        outcome: FileOutcome::ReadError {
                            message: err.to_string(),
                        },
                    }
                }
            };

            let plan = match plan_map.get(&candidate.rel_norm) {
                Some(plan) => plan,
                None => {
                    return FileResult {
                        candidate: candidate_clone,
                        outcome: FileOutcome::PlanError {
                            message: "plan missing".to_string(),
                        },
                    }
                }
            };

            let rename_total: usize = plan.functions.iter().map(|f| f.renames.len()).sum();
            let has_nested = plan.functions.iter().any(|f| f.has_nested_functions);

            if has_nested {
                return FileResult {
                    candidate: candidate_clone,
                    outcome: FileOutcome::SkippedNested {
                        original: source,
                        encoding,
                    },
                };
            }

            if rename_total == 0 {
                return FileResult {
                    candidate: candidate_clone,
                    outcome: FileOutcome::SkippedNoRenames {
                        original: source,
                        encoding,
                    },
                };
            }

            match Minifier::rewrite_with_plan(&plan.module, &source, plan) {
                Ok(rewritten) => {
                    if rewritten == source {
                        FileResult {
                            candidate: candidate_clone,
                            outcome: FileOutcome::SkippedRewriteAborted {
                                original: source,
                                encoding,
                            },
                        }
                    } else {
                        FileResult {
                            candidate: candidate_clone,
                            outcome: FileOutcome::Minified {
                                original: source,
                                rewritten,
                                renames: rename_total,
                                encoding,
                            },
                        }
                    }
                }
                Err(err) => FileResult {
                    candidate: candidate_clone,
                    outcome: FileOutcome::RewriteError {
                        message: err.to_string(),
                    },
                },
            }
        }
    };

    let results = execute_parallel_processing(&candidates, jobs, processor)?;

    finalize_file_results(
        results,
        &mut stats,
        &input_dir,
        &resolved_out_dir,
        in_place,
        dry_run,
        backup_ext,
        quiet,
        show_stats,
        diff,
    )?;

    let summary_needed =
        show_stats || fail_on_bailout || fail_on_error || fail_on_change || output_json.is_some();
    if summary_needed {
        let output_label = if in_place {
            input_dir.display().to_string()
        } else {
            resolved_out_dir.display().to_string()
        };
        print_summary(
            &stats,
            show_stats,
            json_output,
            dry_run,
            &output_label,
            output_json,
        )?;
    }

    Ok(stats)
}

fn minify_dir(
    input_dir: &PathBuf,
    out_dir: Option<PathBuf>,
    includes: &[String],
    excludes: &[String],
    backup_ext: Option<&str>,
    in_place: bool,
    dry_run: bool,
    show_stats: bool,
    json_output: bool,
    quiet: bool,
    output_json: Option<&Path>,
    jobs: Option<usize>,
    fail_on_bailout: bool,
    fail_on_error: bool,
    fail_on_change: bool,
    diff: bool,
) -> anyhow::Result<DirStats> {
    let input_dir = input_dir.canonicalize()?;
    if !input_dir.is_dir() {
        anyhow::bail!("Input '{}' is not a directory", input_dir.display());
    }

    if json_output && !show_stats {
        anyhow::bail!("--json requires --stats");
    }

    if in_place && out_dir.is_some() {
        anyhow::bail!("Cannot use --out-dir with --in-place");
    }

    if backup_ext.is_some() && !in_place {
        anyhow::bail!("--backup-ext requires --in-place");
    }

    let resolved_out_dir = if in_place {
        input_dir.clone()
    } else {
        out_dir.unwrap_or_else(|| default_output_dir(&input_dir))
    };

    if !in_place {
        let resolved_abs = if resolved_out_dir.is_absolute() {
            resolved_out_dir.clone()
        } else {
            std::env::current_dir()?.join(&resolved_out_dir)
        };

        if resolved_abs.starts_with(&input_dir) {
            anyhow::bail!("--out-dir cannot be inside the input directory");
        }

        if resolved_out_dir.exists() {
            if !resolved_out_dir.is_dir() {
                anyhow::bail!(
                    "Output '{}' exists and is not a directory",
                    resolved_out_dir.display()
                );
            }
            if !dry_run && resolved_out_dir.read_dir()?.next().is_some() {
                anyhow::bail!(
                    "Output directory '{}' already exists and is not empty",
                    resolved_out_dir.display()
                );
            }
        } else if !dry_run {
            fs::create_dir_all(&resolved_out_dir)?;
        }
    }

    let jobs = resolve_jobs(jobs)?;

    let mut stats = DirStats::default();

    let include_patterns: Vec<String> = if includes.is_empty() {
        vec!["**/*.py".to_string()]
    } else {
        includes.to_vec()
    };
    let include_glob = build_globset(&include_patterns)?;
    let exclude_patterns = merged_exclude_patterns(excludes);
    let exclude_glob = build_globset(&exclude_patterns)?;

    let mut candidates: Vec<Candidate> = Vec::new();

    for entry in WalkDir::new(&input_dir).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                stats.errors += 1;
                warn!("walk error: {}", err);
                continue;
            }
        };

        if entry.file_type().is_dir() || entry.file_type().is_symlink() {
            continue;
        }

        let path = entry.path();
        let rel_path = match path.strip_prefix(&input_dir) {
            Ok(rel) => rel,
            Err(_) => continue,
        };

        let rel_norm = normalize_rel_path(rel_path);

        if rel_path
            .components()
            .any(|comp| matches!(comp, std::path::Component::Normal(os) if os.to_string_lossy().starts_with('.')))
        {
            debug!("• {} → skipped (hidden path)", rel_norm);
            continue;
        }

        if !include_glob.is_match(rel_norm.as_str()) {
            debug!("• {} → skipped (not included)", rel_norm);
            continue;
        }
        if exclude_glob.is_match(rel_norm.as_str()) {
            debug!("• {} → skipped (excluded)", rel_norm);
            continue;
        }

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("py"))
            != Some(true)
        {
            debug!("• {} → skipped (non-Python)", rel_norm);
            continue;
        }

        candidates.push(Candidate {
            abs_path: path.to_path_buf(),
            rel_path: rel_path.to_path_buf(),
            rel_norm,
        });
    }

    candidates.sort_by(|a, b| a.rel_norm.cmp(&b.rel_norm));

    stats.processed = candidates.len();

    let processor = |candidate: &Candidate| -> FileResult {
        let candidate_clone = candidate.clone();
        let (source, encoding) = match read_python(&candidate.abs_path) {
            Ok(result) => result,
            Err(err) => {
                return FileResult {
                    candidate: candidate_clone,
                    outcome: FileOutcome::ReadError {
                        message: err.to_string(),
                    },
                }
            }
        };

        let module_name = derive_module_name(&candidate.rel_path);
        let plan = match Minifier::plan_from_source(&module_name, &source) {
            Ok(plan) => plan,
            Err(err) => {
                return FileResult {
                    candidate: candidate_clone,
                    outcome: FileOutcome::PlanError {
                        message: err.to_string(),
                    },
                }
            }
        };

        let rename_total: usize = plan.functions.iter().map(|f| f.renames.len()).sum();
        let has_nested = plan.functions.iter().any(|f| f.has_nested_functions);

        if has_nested {
            return FileResult {
                candidate: candidate_clone,
                outcome: FileOutcome::SkippedNested {
                    original: source,
                    encoding,
                },
            };
        }

        if rename_total == 0 {
            return FileResult {
                candidate: candidate_clone,
                outcome: FileOutcome::SkippedNoRenames {
                    original: source,
                    encoding,
                },
            };
        }

        match Minifier::rewrite_with_plan(&module_name, &source, &plan) {
            Ok(rewritten) => {
                if rewritten == source {
                    FileResult {
                        candidate: candidate_clone,
                        outcome: FileOutcome::SkippedRewriteAborted {
                            original: source,
                            encoding,
                        },
                    }
                } else {
                    FileResult {
                        candidate: candidate_clone,
                        outcome: FileOutcome::Minified {
                            original: source,
                            rewritten,
                            renames: rename_total,
                            encoding,
                        },
                    }
                }
            }
            Err(err) => FileResult {
                candidate: candidate_clone,
                outcome: FileOutcome::RewriteError {
                    message: err.to_string(),
                },
            },
        }
    };

    let results = execute_parallel_processing(&candidates, jobs, processor)?;

    finalize_file_results(
        results,
        &mut stats,
        &input_dir,
        &resolved_out_dir,
        in_place,
        dry_run,
        backup_ext,
        quiet,
        show_stats,
        diff,
    )?;

    let summary_needed =
        show_stats || fail_on_bailout || fail_on_error || fail_on_change || output_json.is_some();
    if summary_needed {
        let output_label = if in_place {
            input_dir.display().to_string()
        } else {
            resolved_out_dir.display().to_string()
        };
        print_summary(
            &stats,
            show_stats,
            json_output,
            dry_run,
            &output_label,
            output_json,
        )?;
    }

    Ok(stats)
}

fn default_output_dir(input_dir: &Path) -> PathBuf {
    let parent = input_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let name = input_dir
        .file_name()
        .map(|os| os.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "minified".to_string());

    parent.join(format!("{}-min", name))
}

fn derive_module_name(rel_path: &Path) -> String {
    let without_ext = rel_path.with_extension("");
    let mut parts: Vec<String> = without_ext
        .iter()
        .map(|component| component.to_string_lossy().replace('-', "_"))
        .collect();

    if parts.last().map(|part| part == "__init__").unwrap_or(false) {
        parts.pop();
    }

    if parts.is_empty() {
        rel_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "module".to_string())
    } else {
        parts.join(".")
    }
}

fn merged_exclude_patterns(extras: &[String]) -> Vec<String> {
    let mut patterns: Vec<String> = DEFAULT_EXCLUDES
        .iter()
        .map(|pattern| pattern.to_string())
        .collect();
    patterns.extend(extras.iter().cloned());
    patterns
}

fn build_globset(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

fn normalize_rel_path(rel_path: &Path) -> String {
    let mut parts = Vec::new();
    for component in rel_path.iter() {
        parts.push(component.to_string_lossy());
    }
    parts.join("/")
}

#[derive(Clone)]
struct Candidate {
    abs_path: PathBuf,
    rel_path: PathBuf,
    rel_norm: String,
}

struct FileResult {
    candidate: Candidate,
    outcome: FileOutcome,
}

enum FileOutcome {
    Minified {
        original: String,
        rewritten: String,
        renames: usize,
        encoding: Option<&'static Encoding>,
    },
    SkippedNoRenames {
        original: String,
        encoding: Option<&'static Encoding>,
    },
    SkippedNested {
        original: String,
        encoding: Option<&'static Encoding>,
    },
    SkippedRewriteAborted {
        original: String,
        encoding: Option<&'static Encoding>,
    },
    ReadError {
        message: String,
    },
    PlanError {
        message: String,
    },
    RewriteError {
        message: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FinalStatusKind {
    Minified,
    SkippedNoRenames,
    SkippedNested,
    SkippedRewriteAborted,
    SkippedBackupExists,
}

impl FinalStatusKind {
    fn label(self) -> &'static str {
        match self {
            FinalStatusKind::Minified => "minified",
            FinalStatusKind::SkippedNoRenames => "skipped (no renames)",
            FinalStatusKind::SkippedNested => "skipped (nested scopes)",
            FinalStatusKind::SkippedRewriteAborted => "skipped (rewrite aborted)",
            FinalStatusKind::SkippedBackupExists => "skipped (backup exists)",
        }
    }

    fn is_bailout(self) -> bool {
        matches!(
            self,
            FinalStatusKind::SkippedNested
                | FinalStatusKind::SkippedRewriteAborted
                | FinalStatusKind::SkippedBackupExists
        )
    }
}

fn resolve_jobs(jobs: Option<usize>) -> anyhow::Result<usize> {
    match jobs {
        Some(0) => anyhow::bail!("--jobs must be at least 1"),
        Some(value) => Ok(value),
        None => Ok(std::cmp::max(1, num_cpus::get())),
    }
}

fn execute_parallel_processing<F>(
    candidates: &[Candidate],
    jobs: usize,
    processor: F,
) -> anyhow::Result<Vec<FileResult>>
where
    F: Fn(&Candidate) -> FileResult + Sync,
{
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    if jobs <= 1 {
        Ok(candidates
            .iter()
            .map(|candidate| processor(candidate))
            .collect())
    } else {
        let pool = ThreadPoolBuilder::new().num_threads(jobs).build()?;
        Ok(pool.install(|| {
            candidates
                .par_iter()
                .map(|candidate| processor(candidate))
                .collect()
        }))
    }
}

fn finalize_file_results(
    results: Vec<FileResult>,
    stats: &mut DirStats,
    input_dir: &Path,
    resolved_out_dir: &Path,
    in_place: bool,
    dry_run: bool,
    backup_ext: Option<&str>,
    quiet: bool,
    show_stats: bool,
    diff: bool,
) -> anyhow::Result<()> {
    for result in results {
        let candidate = result.candidate;
        match result.outcome {
            FileOutcome::ReadError { message } => {
                stats.errors += 1;
                error!(
                    "failed to read {}: {}",
                    candidate.abs_path.display(),
                    message
                );
                bump_reason(stats, "read_error");
            }
            FileOutcome::PlanError { message } => {
                stats.errors += 1;
                error!(
                    "failed to plan {}: {}",
                    candidate.abs_path.display(),
                    message
                );
                bump_reason(stats, "plan_error");
            }
            FileOutcome::RewriteError { message } => {
                stats.errors += 1;
                error!(
                    "failed to rewrite {}: {}",
                    candidate.abs_path.display(),
                    message
                );
                debug!("• {} → skipped (rewrite error)", candidate.rel_norm);
                bump_reason(stats, "rewrite_error");
            }
            FileOutcome::Minified {
                original,
                rewritten,
                renames,
                encoding,
            } => {
                process_ready_file(
                    candidate,
                    original,
                    Some(rewritten),
                    renames,
                    FinalStatusKind::Minified,
                    stats,
                    input_dir,
                    resolved_out_dir,
                    in_place,
                    dry_run,
                    backup_ext,
                    encoding,
                    quiet,
                    show_stats,
                    diff,
                )?;
            }
            FileOutcome::SkippedNoRenames { original, encoding } => {
                process_ready_file(
                    candidate,
                    original,
                    None,
                    0,
                    FinalStatusKind::SkippedNoRenames,
                    stats,
                    input_dir,
                    resolved_out_dir,
                    in_place,
                    dry_run,
                    backup_ext,
                    encoding,
                    quiet,
                    show_stats,
                    diff,
                )?;
            }
            FileOutcome::SkippedNested { original, encoding } => {
                process_ready_file(
                    candidate,
                    original,
                    None,
                    0,
                    FinalStatusKind::SkippedNested,
                    stats,
                    input_dir,
                    resolved_out_dir,
                    in_place,
                    dry_run,
                    backup_ext,
                    encoding,
                    quiet,
                    show_stats,
                    diff,
                )?;
            }
            FileOutcome::SkippedRewriteAborted { original, encoding } => {
                process_ready_file(
                    candidate,
                    original,
                    None,
                    0,
                    FinalStatusKind::SkippedRewriteAborted,
                    stats,
                    input_dir,
                    resolved_out_dir,
                    in_place,
                    dry_run,
                    backup_ext,
                    encoding,
                    quiet,
                    show_stats,
                    diff,
                )?;
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_ready_file(
    candidate: Candidate,
    original: String,
    rewritten: Option<String>,
    renames: usize,
    mut status_kind: FinalStatusKind,
    stats: &mut DirStats,
    input_dir: &Path,
    resolved_out_dir: &Path,
    in_place: bool,
    dry_run: bool,
    backup_ext: Option<&str>,
    encoding: Option<&'static Encoding>,
    quiet: bool,
    show_stats: bool,
    diff: bool,
) -> anyhow::Result<()> {
    let mut applied_renames = renames;
    let target_path = if in_place {
        input_dir.join(&candidate.rel_path)
    } else {
        resolved_out_dir.join(&candidate.rel_path)
    };

    if !dry_run {
        if in_place {
            if status_kind == FinalStatusKind::Minified {
                if let Some(ext) = backup_ext {
                    let mut backup_os: OsString = target_path.as_os_str().to_os_string();
                    backup_os.push(ext);
                    let backup_path = PathBuf::from(backup_os);
                    if backup_path.exists() {
                        status_kind = FinalStatusKind::SkippedBackupExists;
                        applied_renames = 0;
                        debug!("• {} → skipped (backup exists)", candidate.rel_norm);
                    } else if let Err(err) = fs::write(&backup_path, &original) {
                        stats.errors += 1;
                        error!("failed to write backup {}: {}", backup_path.display(), err);
                        debug!("• {} → skipped (backup failed)", candidate.rel_norm);
                        bump_reason(stats, "backup_failed");
                        return Ok(());
                    }
                }

                if status_kind == FinalStatusKind::Minified {
                    if let Some(ref content) = rewritten {
                        if let Err(err) = fs::write(&target_path, content) {
                            stats.errors += 1;
                            error!("failed to write {}: {}", target_path.display(), err);
                            debug!("• {} → skipped (write failed)", candidate.rel_norm);
                            bump_reason(stats, "write_failed");
                            return Ok(());
                        }
                    }
                }
            }
        } else {
            if let Some(parent) = target_path.parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    stats.errors += 1;
                    error!("failed to create directory {}: {}", parent.display(), err);
                    debug!("• {} → skipped (mkdir failed)", candidate.rel_norm);
                    bump_reason(stats, "mkdir_failed");
                    return Ok(());
                }
            }

            let content = if status_kind == FinalStatusKind::Minified {
                rewritten
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or_else(|| original.as_str())
            } else {
                original.as_str()
            };

            if let Err(err) = fs::write(&target_path, content) {
                stats.errors += 1;
                error!("failed to write {}: {}", target_path.display(), err);
                debug!("• {} → skipped (write failed)", candidate.rel_norm);
                bump_reason(stats, "write_failed");
                return Ok(());
            }
        }
    }

    match status_kind {
        FinalStatusKind::Minified => {
            stats.rewritten += 1;
            stats.total_renames += applied_renames;
            bump_reason(stats, "minified");
        }
        FinalStatusKind::SkippedNoRenames => {
            stats.skipped_no_change += 1;
            bump_reason(stats, "no_renames");
        }
        _ => {
            if status_kind.is_bailout() {
                stats.bailouts += 1;
            }
            let reason = match status_kind {
                FinalStatusKind::SkippedNested => "nested_scopes",
                FinalStatusKind::SkippedRewriteAborted => "rewrite_aborted",
                FinalStatusKind::SkippedBackupExists => "backup_exists",
                _ => "unknown",
            };
            if reason != "unknown" {
                bump_reason(stats, reason);
            }
        }
    }

    if show_stats {
        stats.files.push(FileStats {
            path: candidate.rel_norm.clone(),
            renames: applied_renames,
            status: status_kind.label().to_string(),
        });
    }

    if diff && status_kind == FinalStatusKind::Minified {
        if let Some(ref new_content) = rewritten {
            let diff_str = make_unified_diff(&candidate.rel_norm, &original, new_content);
            println!("{}", diff_str);
        }
    }

    print_file_status(
        &candidate.rel_norm,
        status_kind.label(),
        applied_renames,
        show_stats,
        quiet,
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result as AnyResult;
    use serde_json;
    use tempfile::tempdir;

    #[test]
    fn unified_diff_smoke() {
        let diff = make_unified_diff("example.py", "a = 1\n", "a = 2\n");
        assert!(diff.contains("a/example.py"));
        assert!(diff.contains("b/example.py"));
        assert!(diff.contains("-a = 1"));
        assert!(diff.contains("+a = 2"));
    }

    #[test]
    fn minify_dir_preserves_structure() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("input");
        let nested = input_dir.join("pkg");
        fs::create_dir_all(&nested)?;

        let module_source = "\
def sample(value):
    temp = value + 1
    return temp
";
        fs::write(input_dir.join("module.py"), module_source)?;
        fs::write(nested.join("__init__.py"), "")?;

        let output_dir = tmp.path().join("output");
        let _stats = minify_dir(
            &input_dir,
            Some(output_dir.clone()),
            &[],
            &[],
            None,
            false,
            false,
            false,
            false,
            false,
            None,
            None,
            false,
            false,
            false,
            false,
        )?;

        let rewritten = fs::read_to_string(output_dir.join("module.py"))?;
        assert!(rewritten.contains("def sample(a):"));
        assert!(output_dir.join("pkg/__init__.py").exists());
        Ok(())
    }

    #[test]
    fn minify_dir_respects_include_exclude() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        let pkg_a = input_dir.join("pkg_a");
        let pkg_b = input_dir.join("pkg_b");
        fs::create_dir_all(&pkg_a)?;
        fs::create_dir_all(&pkg_b)?;

        fs::write(
            pkg_a.join("mod.py"),
            "def foo(x):\n    y = x + 1\n    return y\n",
        )?;
        fs::write(
            pkg_b.join("mod.py"),
            "def bar(y):\n    z = y - 1\n    return z\n",
        )?;

        let output_dir = tmp.path().join("out");
        let _stats = minify_dir(
            &input_dir,
            Some(output_dir.clone()),
            &["pkg_a/**".to_string()],
            &[],
            None,
            false,
            false,
            false,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )?;

        assert!(output_dir.join("pkg_a/mod.py").exists());
        assert!(!output_dir.join("pkg_b/mod.py").exists());
        Ok(())
    }

    #[test]
    fn minify_dir_dry_run_creates_no_output() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(
            input_dir.join("example.py"),
            "def foo(x):\n    y = x + 1\n    return y\n",
        )?;

        let output_dir = tmp.path().join("out");
        let _stats = minify_dir(
            &input_dir,
            Some(output_dir.clone()),
            &[],
            &[],
            None,
            false,
            true,
            true,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )?;

        assert!(!output_dir.exists());
        Ok(())
    }

    #[test]
    fn minify_dir_in_place_updates_files() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        let file_path = input_dir.join("example.py");
        fs::write(
            &file_path,
            "def foo(value):\n    temp = value + 1\n    return temp\n",
        )?;

        let _stats = minify_dir(
            &input_dir,
            None,
            &[],
            &[],
            None,
            true,
            false,
            false,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )?;

        let rewritten = fs::read_to_string(&file_path)?;
        assert!(rewritten.contains("def foo(a):"));
        Ok(())
    }

    #[test]
    fn minify_dir_in_place_writes_backup() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        let file_path = input_dir.join("example.py");
        let original = "def foo(value):\n    temp = value + 1\n    return temp\n";
        fs::write(&file_path, original)?;

        let _stats = minify_dir(
            &input_dir,
            None,
            &[],
            &[],
            Some(".bak"),
            true,
            false,
            false,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )?;

        let rewritten = fs::read_to_string(&file_path)?;
        assert!(rewritten.contains("def foo(a):"));

        let backup_path = input_dir.join("example.py.bak");
        assert!(backup_path.exists());
        let backup_contents = fs::read_to_string(backup_path)?;
        assert_eq!(backup_contents, original);

        Ok(())
    }

    #[test]
    fn minify_dir_stats_json_runs() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(
            input_dir.join("example.py"),
            "def foo(x):\n    y = x + 1\n    return y\n",
        )?;

        let output_dir = tmp.path().join("out");
        let _stats = minify_dir(
            &input_dir,
            Some(output_dir),
            &[],
            &[],
            None,
            false,
            true,
            true,
            true,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )?;

        Ok(())
    }

    #[test]
    fn minify_file_output_json_writes_file() -> AnyResult<()> {
        let tmp = tempdir()?;
        let file_path = tmp.path().join("example.py");
        fs::write(
            &file_path,
            "def foo(value):\n    temp = value + 1\n    return temp\n",
        )?;

        let json_path = tmp.path().join("file.json");
        let stats = minify_file(
            &file_path,
            false,
            None,
            false,
            false,
            true,
            Some(json_path.as_path()),
            false,
            false,
            false,
            false,
        )?;

        let written: DirStats = serde_json::from_str(&fs::read_to_string(&json_path)?)?;
        assert_eq!(written.processed, stats.processed);
        assert_eq!(written.rewritten, stats.rewritten);
        Ok(())
    }

    #[test]
    fn minify_file_reasons_noop() -> AnyResult<()> {
        let tmp = tempdir()?;
        let file_path = tmp.path().join("example.py");
        fs::write(&file_path, "def foo():\n    return 42\n")?;

        let json_path = tmp.path().join("reasons.json");
        let stats = minify_file(
            &file_path,
            false,
            None,
            false,
            false,
            true,
            Some(json_path.as_path()),
            false,
            false,
            false,
            false,
        )?;

        assert_eq!(stats.reasons.get("no_renames"), Some(&1));

        let written: DirStats = serde_json::from_str(&fs::read_to_string(&json_path)?)?;
        assert_eq!(written.reasons.get("no_renames"), Some(&1));

        Ok(())
    }

    #[test]
    fn minify_dir_output_json_writes_file() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(
            input_dir.join("example.py"),
            "def foo(x):\n    y = x + 1\n    return y\n",
        )?;

        let output_dir = tmp.path().join("out");
        let json_path = tmp.path().join("dir.json");
        let stats = minify_dir(
            &input_dir,
            Some(output_dir),
            &[],
            &[],
            None,
            false,
            false,
            false,
            false,
            true,
            Some(json_path.as_path()),
            None,
            false,
            false,
            false,
            false,
        )?;

        let written: DirStats = serde_json::from_str(&fs::read_to_string(&json_path)?)?;
        assert_eq!(written.processed, stats.processed);
        assert_eq!(written.rewritten, stats.rewritten);
        Ok(())
    }

    #[test]
    fn minify_dir_rejects_output_inside_input() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(
            input_dir.join("example.py"),
            "def foo(x):\n    y = x + 1\n    return y\n",
        )?;

        let out_dir = input_dir.join("out");
        let err = minify_dir(
            &input_dir,
            Some(out_dir),
            &[],
            &[],
            None,
            false,
            false,
            false,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )
        .expect_err("out dir under input should error");
        let message = err.to_string();
        assert!(
            message.contains("--out-dir cannot be inside the input directory"),
            "unexpected error: {}",
            message
        );
        Ok(())
    }

    #[test]
    fn apply_plan_dir_output_json_writes_file() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(
            input_dir.join("example.py"),
            "def foo(x):\n    y = x + 1\n    return y\n",
        )?;

        let plan_path = tmp.path().join("plan.json");
        minify_plan_dir(&input_dir, &plan_path, &[], &[], None, true)?;
        assert!(plan_path.exists());

        let output_dir = tmp.path().join("out");
        let json_path = tmp.path().join("apply-dir.json");
        let stats = apply_plan_dir(
            &input_dir,
            &plan_path,
            Some(output_dir),
            &[],
            &[],
            None,
            false,
            false,
            false,
            false,
            true,
            Some(json_path.as_path()),
            None,
            false,
            false,
            false,
            false,
        )?;

        let written: DirStats = serde_json::from_str(&fs::read_to_string(&json_path)?)?;
        assert_eq!(written.processed, stats.processed);
        assert_eq!(written.rewritten, stats.rewritten);
        Ok(())
    }

    #[test]
    fn apply_plan_dir_rejects_output_inside_input() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(
            input_dir.join("example.py"),
            "def foo(x):\n    y = x + 1\n    return y\n",
        )?;

        let plan_path = tmp.path().join("plan.json");
        minify_plan_dir(&input_dir, &plan_path, &[], &[], None, true)?;

        let out_dir = input_dir.join("out");
        let err = apply_plan_dir(
            &input_dir,
            &plan_path,
            Some(out_dir),
            &[],
            &[],
            None,
            false,
            false,
            false,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )
        .expect_err("out dir under input should error");
        let message = err.to_string();
        assert!(
            message.contains("--out-dir cannot be inside the input directory"),
            "unexpected error: {}",
            message
        );
        Ok(())
    }

    #[test]
    fn minify_file_in_place_writes_backup() -> AnyResult<()> {
        let tmp = tempdir()?;
        let file_path = tmp.path().join("example.py");
        let original = "def foo(value):\n    temp = value + 1\n    return temp\n";
        fs::write(&file_path, original)?;

        let _stats = minify_file(
            &file_path,
            true,
            Some(".bak"),
            false,
            false,
            true,
            None,
            false,
            false,
            false,
            false,
        )?;

        let rewritten = fs::read_to_string(&file_path)?;
        assert!(rewritten.contains("def foo(a):"));

        let backup_path = tmp.path().join("example.py.bak");
        assert!(backup_path.exists());
        let backup_contents = fs::read_to_string(backup_path)?;
        assert_eq!(backup_contents, original);

        Ok(())
    }

    #[test]
    fn minify_file_stats_json_runs() -> AnyResult<()> {
        let tmp = tempdir()?;
        let file_path = tmp.path().join("example.py");
        fs::write(
            &file_path,
            "def foo(value):\n    temp = value + 1\n    return temp\n",
        )?;

        let _stats = minify_file(
            &file_path, false, None, true, true, true, None, false, false, false, false,
        )?;

        Ok(())
    }

    #[test]
    fn apply_plan_in_place_writes_backup() -> AnyResult<()> {
        let tmp = tempdir()?;
        let file_path = tmp.path().join("example.py");
        let source = "def foo(value):\n    temp = value + 1\n    return temp\n";
        fs::write(&file_path, source)?;

        let plan = Minifier::plan_from_source("module", source)?;
        let plan_path = tmp.path().join("plan.json");
        fs::write(&plan_path, serde_json::to_string(&plan)?)?;

        let _stats = apply_plan(
            &file_path,
            &plan_path,
            true,
            Some(".bak"),
            false,
            false,
            true,
            None,
            false,
            false,
            false,
            false,
        )?;

        let rewritten = fs::read_to_string(&file_path)?;
        assert!(rewritten.contains("def foo(a):"));

        let backup_path = tmp.path().join("example.py.bak");
        assert!(backup_path.exists());
        let backup_contents = fs::read_to_string(backup_path)?;
        assert_eq!(backup_contents, source);

        Ok(())
    }

    #[test]
    fn apply_plan_stats_json_runs() -> AnyResult<()> {
        let tmp = tempdir()?;
        let file_path = tmp.path().join("example.py");
        let source = "def foo(value):\n    temp = value + 1\n    return temp\n";
        fs::write(&file_path, source)?;

        let plan = Minifier::plan_from_source("module", source)?;
        let plan_path = tmp.path().join("plan.json");
        fs::write(&plan_path, serde_json::to_string(&plan)?)?;

        let _stats = apply_plan(
            &file_path, &plan_path, false, None, true, true, true, None, false, false, false, false,
        )?;

        Ok(())
    }

    #[test]
    fn compute_exit_code_flags() {
        let mut stats = DirStats::default();
        assert_eq!(compute_exit_code(&stats, false, false, false), 0);

        stats.errors = 1;
        assert_eq!(compute_exit_code(&stats, false, true, false), 1);

        stats.errors = 0;
        stats.bailouts = 2;
        assert_eq!(compute_exit_code(&stats, true, false, false), 2);

        stats.bailouts = 0;
        stats.rewritten = 3;
        assert_eq!(compute_exit_code(&stats, false, false, true), 4);

        stats.errors = 1;
        stats.bailouts = 1;
        stats.rewritten = 1;
        assert_eq!(compute_exit_code(&stats, true, true, true), 7);
    }

    #[test]
    fn minify_plan_dir_round_trip() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        let nested = input_dir.join("pkg");
        fs::create_dir_all(&nested)?;

        fs::write(
            input_dir.join("module.py"),
            "def foo(value):\n    temp = value + 1\n    return temp\n",
        )?;
        fs::write(
            nested.join("helpers.py"),
            "def helper(value):\n    result = value * 2\n    return result\n",
        )?;

        let plan_path = tmp.path().join("plan.json");
        minify_plan_dir(&input_dir, &plan_path, &[], &[], None, true)?;
        assert!(plan_path.exists());

        let plan_bundle: PlanBundle = serde_json::from_str(&fs::read_to_string(&plan_path)?)?;
        assert_eq!(plan_bundle.files.len(), 2);

        let output_dir = tmp.path().join("out");
        let _stats = apply_plan_dir(
            &input_dir,
            &plan_path,
            Some(output_dir.clone()),
            &[],
            &[],
            None,
            false,
            false,
            false,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )?;

        let rewritten_module = fs::read_to_string(output_dir.join("module.py"))?;
        assert!(rewritten_module.contains("def foo(a):"));

        let rewritten_helper = fs::read_to_string(output_dir.join("pkg/helpers.py"))?;
        assert!(rewritten_helper.contains("def helper(a):"));

        Ok(())
    }

    #[test]
    fn minify_plan_dir_includes_version() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(input_dir.join("example.py"), "def foo(x):\n    return x\n")?;

        let plan_path = tmp.path().join("plan.json");
        minify_plan_dir(&input_dir, &plan_path, &[], &[], None, true)?;

        let plan_bundle: PlanBundle = serde_json::from_str(&fs::read_to_string(&plan_path)?)?;
        assert_eq!(plan_bundle.version, PLAN_BUNDLE_VERSION);

        Ok(())
    }

    #[test]
    fn apply_plan_dir_rejects_future_version() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;
        fs::write(input_dir.join("example.py"), "def foo(x):\n    return x\n")?;

        let plan_path = tmp.path().join("plan.json");
        minify_plan_dir(&input_dir, &plan_path, &[], &[], None, true)?;

        let mut bundle_value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&plan_path)?)?;
        if let serde_json::Value::Object(ref mut obj) = bundle_value {
            obj.insert(
                "version".to_string(),
                serde_json::Value::Number(serde_json::Number::from(
                    (PLAN_BUNDLE_VERSION + 1) as u64,
                )),
            );
        }
        fs::write(&plan_path, serde_json::to_string_pretty(&bundle_value)?)?;

        let output_dir = tmp.path().join("out");
        let err = apply_plan_dir(
            &input_dir,
            &plan_path,
            Some(output_dir),
            &[],
            &[],
            None,
            false,
            false,
            false,
            false,
            true,
            None,
            None,
            false,
            false,
            false,
            false,
        )
        .expect_err("future plan version should be rejected");

        let message = err.to_string();
        assert!(
            message.contains("unsupported plan bundle version"),
            "unexpected error: {}",
            message
        );

        Ok(())
    }

    #[test]
    fn minify_plan_dir_deterministic_order() -> AnyResult<()> {
        let tmp = tempdir()?;
        let input_dir = tmp.path().join("src");
        fs::create_dir_all(&input_dir)?;

        fs::write(input_dir.join("b.py"), "def foo(x):\n    return x\n")?;
        fs::write(input_dir.join("a.py"), "def bar(y):\n    return y\n")?;

        let plan_path = tmp.path().join("plan.json");

        minify_plan_dir(&input_dir, &plan_path, &[], &[], None, true)?;
        let bundle_one: PlanBundle = serde_json::from_str(&fs::read_to_string(&plan_path)?)?;

        minify_plan_dir(&input_dir, &plan_path, &[], &[], None, true)?;
        let bundle_two: PlanBundle = serde_json::from_str(&fs::read_to_string(&plan_path)?)?;

        let expected = vec!["a.py", "b.py"];
        let paths_one: Vec<_> = bundle_one.files.iter().map(|f| f.path.as_str()).collect();
        let paths_two: Vec<_> = bundle_two.files.iter().map(|f| f.path.as_str()).collect();

        assert_eq!(paths_one, expected);
        assert_eq!(paths_two, expected);

        Ok(())
    }
}
