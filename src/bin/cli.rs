//! CLI for tree-shaking operations

use clap::{Parser, Subcommand};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use tracing_subscriber::filter::EnvFilter;
use tsrs::{Minifier, MinifyPlan, VenvAnalyzer, VenvSlimmer};
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "tsrs")]
#[command(about = "Tree-shaking in Rust for Python", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable debug logging
    #[arg(global = true, short, long)]
    debug: bool,
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
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let env_filter = if cli.debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::fmt().with_env_filter(env_filter).init();

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
        } => {
            minify_plan_dir(&input_dir, &out, &include, &exclude, cli.debug)?;
        }
        Commands::Minify {
            python_file,
            in_place,
            backup_ext,
            stats,
            json,
        } => {
            minify(
                &python_file,
                in_place,
                backup_ext.as_deref(),
                stats,
                json,
            )?;
        }
        Commands::ApplyPlan {
            python_file,
            plan,
            in_place,
            backup_ext,
            stats,
            json,
        } => {
            apply_plan(
                &python_file,
                &plan,
                in_place,
                backup_ext.as_deref(),
                stats,
                json,
            )?;
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
        } => {
            minify_dir(
                &input_dir,
                out_dir,
                &include,
                &exclude,
                backup_ext.as_deref(),
                in_place,
                dry_run,
                cli.debug,
                stats,
                json,
            )?;
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
        } => {
            apply_plan_dir(
                &input_dir,
                &plan,
                out_dir,
                &include,
                &exclude,
                backup_ext.as_deref(),
                in_place,
                dry_run,
                cli.debug,
                stats,
                json,
            )?;
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
) -> anyhow::Result<()> {
    minify_file(file_path, in_place, backup_ext, show_stats, json_output)
}

fn apply_plan(
    file_path: &PathBuf,
    plan_path: &PathBuf,
    in_place: bool,
    backup_ext: Option<&str>,
    show_stats: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    if json_output && !show_stats {
        anyhow::bail!("--json requires --stats");
    }

    let plan_file = fs::read_to_string(plan_path)?;
    let plan: MinifyPlan = serde_json::from_str(&plan_file)?;

    apply_plan_to_file(file_path, &plan, in_place, backup_ext, show_stats, json_output)
}

#[derive(Debug, Default, Serialize)]
struct DirStats {
    processed: usize,
    rewritten: usize,
    skipped_no_change: usize,
    bailouts: usize,
    errors: usize,
    total_renames: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileStats>,
}

#[derive(Debug, Serialize)]
struct FileStats {
    path: String,
    renames: usize,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PlanBundle {
    files: Vec<PlanFile>,
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
) -> anyhow::Result<()> {
    let source = fs::read_to_string(file_path)?;
    let module_name = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_path.to_string_lossy().to_string());

    let plan = Minifier::plan_from_source(&module_name, &source)?;
    apply_plan_to_file(file_path, &plan, in_place, backup_ext, show_stats, json_output)
}

fn apply_plan_to_file(
    file_path: &PathBuf,
    plan: &MinifyPlan,
    in_place: bool,
    backup_ext: Option<&str>,
    show_stats: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    if json_output && !show_stats {
        anyhow::bail!("--json requires --stats");
    }

    if backup_ext.is_some() && !in_place {
        anyhow::bail!("--backup-ext requires --in-place");
    }

    let source = fs::read_to_string(file_path)?;
    let module_name = file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_path.to_string_lossy().to_string());

    let rename_total: usize = plan.functions.iter().map(|f| f.renames.len()).sum();
    let has_nested = plan
        .functions
        .iter()
        .any(|f| f.has_nested_functions);

    let mut status;
    let mut final_content: Cow<'_, str> = Cow::Borrowed(&source);
    let mut rewrote = false;

    if has_nested {
        status = "skipped (nested scopes)".to_string();
    } else if rename_total == 0 {
        status = "skipped (no renames)".to_string();
    } else {
        let rewritten = Minifier::rewrite_with_plan(&module_name, &source, plan)?;
        if rewritten == source {
            status = "skipped (rewrite aborted)".to_string();
        } else {
            status = "minified".to_string();
            final_content = Cow::Owned(rewritten);
            rewrote = true;
        }
    }

    let display_path = file_path.display().to_string();

    if in_place && rewrote {
        if let Some(ext) = backup_ext {
            let mut backup_os = file_path.as_os_str().to_os_string();
            backup_os.push(ext);
            let backup_path = PathBuf::from(backup_os);
            if backup_path.exists() {
                status = "skipped (backup exists)".to_string();
                rewrote = false;
                final_content = Cow::Borrowed(&source);
            } else {
                fs::write(&backup_path, &source)?;
            }
        }

        if rewrote {
            if let Cow::Owned(ref content) = final_content {
                fs::write(file_path, content)?;
            }
        }
    }

    let mut applied_renames = if rewrote { rename_total } else { 0 };

    if status == "skipped (backup exists)" {
        applied_renames = 0;
    }

    if show_stats {
        println!("• {} → {} (renames: {})", display_path, status, applied_renames);
    } else if in_place {
        println!("• {} → {}", display_path, status);
    }

    if !in_place && !show_stats {
        println!("{}", final_content);
    }

    if show_stats {
        let mut stats = DirStats::default();
        stats.processed = 1;
        stats.total_renames = applied_renames;
        match status.as_str() {
            "minified" => stats.rewritten = 1,
            "skipped (no renames)" => stats.skipped_no_change = 1,
            _ => stats.bailouts = 1,
        }
        stats.files.push(FileStats {
            path: display_path.clone(),
            renames: applied_renames,
            status: status.clone(),
        });

        let output_target = if in_place {
            display_path.clone()
        } else {
            "stdout".to_string()
        };

        if json_output {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        } else {
            println!(
                "Processed 1 file → {} minified, {} skipped, {} bailouts, {} errors, {} renames. Output: {}",
                stats.rewritten,
                stats.skipped_no_change,
                stats.bailouts,
                stats.errors,
                stats.total_renames,
                output_target,
            );
        }
    }

    Ok(())
}

fn minify_plan_dir(
    input_dir: &PathBuf,
    out_path: &PathBuf,
    includes: &[String],
    excludes: &[String],
    debug: bool,
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
    let exclude_glob = if excludes.is_empty() {
        None
    } else {
        Some(build_globset(excludes)?)
    };

    let mut plans: Vec<PlanFile> = Vec::new();
    let mut errors = 0usize;

    for entry in WalkDir::new(&input_dir).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                errors += 1;
                eprintln!("walk error: {}", err);
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
            if debug {
                println!("• {} → skipped (hidden path)", rel_norm);
            }
            continue;
        }

        if !include_glob.is_match(rel_norm.as_str()) {
            if debug {
                println!("• {} → skipped (not included)", rel_norm);
            }
            continue;
        }
        if let Some(exclude_glob) = &exclude_glob {
            if exclude_glob.is_match(rel_norm.as_str()) {
                if debug {
                    println!("• {} → skipped (excluded)", rel_norm);
                }
                continue;
            }
        }

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("py"))
            != Some(true)
        {
            if debug {
                println!("• {} → skipped (non-Python)", rel_norm);
            }
            continue;
        }

        let source = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) => {
                errors += 1;
                eprintln!("failed to read {}: {}", path.display(), err);
                continue;
            }
        };

        let module_name = derive_module_name(rel_path);
        let plan = match Minifier::plan_from_source(&module_name, &source) {
            Ok(plan) => plan,
            Err(err) => {
                errors += 1;
                eprintln!("failed to plan {}: {}", path.display(), err);
                continue;
            }
        };

        let rename_total: usize = plan.functions.iter().map(|f| f.renames.len()).sum();
        println!("• {} → planned (renames: {})", rel_norm, rename_total);

        plans.push(PlanFile {
            path: rel_norm,
            plan,
        });
    }

    plans.sort_by(|a, b| a.path.cmp(&b.path));
    let planned_count = plans.len();

    if planned_count == 0 {
        eprintln!("warning: no files matched the provided filters; writing empty plan bundle");
    }

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let bundle = PlanBundle { files: plans };
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
    debug: bool,
    show_stats: bool,
    json_output: bool,
) -> anyhow::Result<()> {
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
    let mut plan_map: HashMap<String, MinifyPlan> = HashMap::new();
    for file_plan in bundle.files {
        plan_map.insert(file_plan.path, file_plan.plan);
    }

    if plan_map.is_empty() {
        anyhow::bail!("Plan bundle contains no files");
    }

    let resolved_out_dir = if in_place {
        input_dir.clone()
    } else {
        out_dir.unwrap_or_else(|| default_output_dir(&input_dir))
    };

    if !in_place && resolved_out_dir.starts_with(&input_dir) {
        anyhow::bail!(
            "Output directory '{}' must not be inside the input directory",
            resolved_out_dir.display()
        );
    }

    if !in_place {
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
    let exclude_glob = if excludes.is_empty() {
        None
    } else {
        Some(build_globset(excludes)?)
    };

    let mut stats = DirStats::default();

    for entry in WalkDir::new(&input_dir).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                stats.errors += 1;
                eprintln!("walk error: {}", err);
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
            if debug {
                println!("• {} → skipped (hidden path)", rel_norm);
            }
            continue;
        }

        if !include_glob.is_match(rel_norm.as_str()) {
            if debug {
                println!("• {} → skipped (not included)", rel_norm);
            }
            continue;
        }
        if let Some(exclude_glob) = &exclude_glob {
            if exclude_glob.is_match(rel_norm.as_str()) {
                if debug {
                    println!("• {} → skipped (excluded)", rel_norm);
                }
                continue;
            }
        }

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("py"))
            != Some(true)
        {
            if debug {
                println!("• {} → skipped (non-Python)", rel_norm);
            }
            continue;
        }

        let plan = match plan_map.get(&rel_norm) {
            Some(plan) => plan,
            None => {
                if debug {
                    println!("• {} → skipped (no plan)", rel_norm);
                }
                continue;
            }
        };

        stats.processed += 1;

        let source = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) => {
                stats.errors += 1;
                eprintln!("failed to read {}: {}", path.display(), err);
                continue;
            }
        };

        let rename_total: usize = plan.functions.iter().map(|f| f.renames.len()).sum();
        let has_nested = plan
            .functions
            .iter()
            .any(|f| f.has_nested_functions);

        let mut status;
        let mut final_content: Cow<'_, str> = Cow::Borrowed(&source);
        let mut rewrote = false;

        if has_nested {
            stats.bailouts += 1;
            status = "skipped (nested scopes)".to_string();
        } else if rename_total == 0 {
            stats.skipped_no_change += 1;
            status = "skipped (no renames)".to_string();
        } else {
            match Minifier::rewrite_with_plan(&plan.module, &source, plan) {
                Ok(rewritten) => {
                    if rewritten == source {
                        stats.bailouts += 1;
                        status = "skipped (rewrite aborted)".to_string();
                    } else {
                        stats.rewritten += 1;
                        status = "minified".to_string();
                        final_content = Cow::Owned(rewritten);
                        rewrote = true;
                    }
                }
                Err(err) => {
                    stats.errors += 1;
                    eprintln!("failed to rewrite {}: {}", path.display(), err);
                    if debug {
                        println!("• {} → skipped (rewrite error)", rel_norm);
                    }
                    continue;
                }
            }
        }

        let mut applied_renames = if rewrote { rename_total } else { 0 };

        let target_path = if in_place {
            input_dir.join(rel_path)
        } else {
            resolved_out_dir.join(rel_path)
        };

        if !dry_run {
            if in_place {
                if rewrote {
                    if let Some(ext) = backup_ext {
                        let mut backup_os: OsString = target_path.as_os_str().to_os_string();
                        backup_os.push(ext);
                        let backup_path = PathBuf::from(backup_os);
                        if backup_path.exists() {
                            stats.bailouts += 1;
                            if stats.rewritten > 0 {
                                stats.rewritten -= 1;
                            }
                            status = "skipped (backup exists)".to_string();
                            applied_renames = 0;
                            final_content = Cow::Borrowed(&source);
                            if debug {
                                println!("• {} → skipped (backup exists)", rel_norm);
                            }
                        } else {
                            fs::write(&backup_path, &source)?;
                        }
                    }

                    if let Cow::Owned(ref content) = final_content {
                        fs::write(&target_path, content)?;
                    }
                }
            } else {
                if let Some(parent) = target_path.parent() {
                    if let Err(err) = fs::create_dir_all(parent) {
                        stats.errors += 1;
                        eprintln!("failed to create directory {}: {}", parent.display(), err);
                        if debug {
                            println!("• {} → skipped (mkdir failed)", rel_norm);
                        }
                        continue;
                    }
                }

                if let Err(err) = fs::write(&target_path, final_content.as_ref()) {
                    stats.errors += 1;
                    eprintln!("failed to write {}: {}", target_path.display(), err);
                    if debug {
                        println!("• {} → skipped (write failed)", rel_norm);
                    }
                    continue;
                }
            }
        }

        stats.total_renames += applied_renames;

        if show_stats {
            stats.files.push(FileStats {
                path: rel_norm.clone(),
                renames: applied_renames,
                status: status.clone(),
            });
        }

        if show_stats {
            println!("• {} → {} (renames: {})", rel_norm, status, applied_renames);
        } else {
            println!("• {} → {}", rel_norm, status);
        }
    }

    if dry_run {
        if show_stats {
            println!(
                "Dry run complete: {} files matched → {} minified, {} skipped, {} bailouts, {} errors, {} renames. Output: {}",
                stats.processed,
                stats.rewritten,
                stats.skipped_no_change,
                stats.bailouts,
                stats.errors,
                stats.total_renames,
                if in_place {
                    input_dir.display().to_string()
                } else {
                    resolved_out_dir.display().to_string()
                }
            );
        } else {
            println!(
                "Dry run complete: {} files matched → {} minified, {} skipped, {} bailouts, {} errors. Output: {}",
                stats.processed,
                stats.rewritten,
                stats.skipped_no_change,
                stats.bailouts,
                stats.errors,
                if in_place {
                    input_dir.display().to_string()
                } else {
                    resolved_out_dir.display().to_string()
                }
            );
        }
    } else if show_stats {
        println!(
            "Processed {} files → {} minified, {} skipped, {} bailouts, {} errors, {} renames. Output: {}",
            stats.processed,
            stats.rewritten,
            stats.skipped_no_change,
            stats.bailouts,
            stats.errors,
            stats.total_renames,
            if in_place {
                input_dir.display().to_string()
            } else {
                resolved_out_dir.display().to_string()
            }
        );
    } else {
        println!(
            "Processed {} files → {} minified, {} skipped, {} bailouts, {} errors. Output: {}",
            stats.processed,
            stats.rewritten,
            stats.skipped_no_change,
            stats.bailouts,
            stats.errors,
            if in_place {
                input_dir.display().to_string()
            } else {
                resolved_out_dir.display().to_string()
            }
        );
    }

    if show_stats && json_output {
        let json = serde_json::to_string_pretty(&stats)?;
        println!("{}", json);
    }

    Ok(())
}

fn minify_dir(
    input_dir: &PathBuf,
    out_dir: Option<PathBuf>,
    includes: &[String],
    excludes: &[String],
    backup_ext: Option<&str>,
    in_place: bool,
    dry_run: bool,
    debug: bool,
    show_stats: bool,
    json_output: bool,
) -> anyhow::Result<()> {
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

    if !in_place && resolved_out_dir.starts_with(&input_dir) {
        anyhow::bail!(
            "Output directory '{}' must not be inside the input directory",
            resolved_out_dir.display()
        );
    }

    if !in_place {
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

    let mut stats = DirStats::default();

    let include_patterns: Vec<String> = if includes.is_empty() {
        vec!["**/*.py".to_string()]
    } else {
        includes.to_vec()
    };
    let include_glob = build_globset(&include_patterns)?;
    let exclude_glob = if excludes.is_empty() {
        None
    } else {
        Some(build_globset(excludes)?)
    };

    for entry in WalkDir::new(&input_dir).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                stats.errors += 1;
                eprintln!("walk error: {}", err);
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
            if debug {
                println!("• {} → skipped (hidden path)", rel_norm);
            }
            continue;
        }

        if !include_glob.is_match(rel_norm.as_str()) {
            if debug {
                println!("• {} → skipped (not included)", rel_norm);
            }
            continue;
        }
        if let Some(exclude_glob) = &exclude_glob {
            if exclude_glob.is_match(rel_norm.as_str()) {
                if debug {
                    println!("• {} → skipped (excluded)", rel_norm);
                }
                continue;
            }
        }

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("py"))
            != Some(true)
        {
            if debug {
                println!("• {} → skipped (non-Python)", rel_norm);
            }
            continue;
        }

        stats.processed += 1;

        let source = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) => {
                stats.errors += 1;
                eprintln!("failed to read {}: {}", path.display(), err);
                continue;
            }
        };

        let module_name = derive_module_name(rel_path);
        let plan = match Minifier::plan_from_source(&module_name, &source) {
            Ok(plan) => plan,
            Err(err) => {
                stats.errors += 1;
                eprintln!("failed to plan {}: {}", path.display(), err);
                continue;
            }
        };

        let rename_total: usize = plan.functions.iter().map(|f| f.renames.len()).sum();
        let has_nested = plan
            .functions
            .iter()
            .any(|f| f.has_nested_functions);

        let mut status;
        let mut final_content: Cow<'_, str>;
        let mut is_rewrite = false;

        if has_nested {
            stats.bailouts += 1;
            status = "skipped (nested scopes)".to_string();
            final_content = Cow::Borrowed(&source);
        } else if rename_total == 0 {
            stats.skipped_no_change += 1;
            status = "skipped (no renames)".to_string();
            final_content = Cow::Borrowed(&source);
        } else {
            match Minifier::rewrite_source(&module_name, &source) {
                Ok(rewritten) => {
                    if rewritten == source {
                        stats.bailouts += 1;
                        status = "skipped (rewrite aborted)".to_string();
                        final_content = Cow::Borrowed(&source);
                    } else {
                        stats.rewritten += 1;
                        status = "minified".to_string();
                        final_content = Cow::Owned(rewritten);
                        is_rewrite = true;
                    }
                }
                Err(err) => {
                    stats.errors += 1;
                    eprintln!("failed to rewrite {}: {}", path.display(), err);
                    if debug {
                        println!("• {} → skipped (rewrite error)", rel_norm);
                    }
                    continue;
                }
            }
        }

        let target_path = if in_place {
            input_dir.join(rel_path)
        } else {
            resolved_out_dir.join(rel_path)
        };

        if !dry_run {
            if in_place {
                if is_rewrite {
                    if let Some(ext) = backup_ext {
                        let mut backup_os: OsString = target_path.as_os_str().to_os_string();
                        backup_os.push(ext);
                        let backup_path = PathBuf::from(backup_os);
                        if backup_path.exists() {
                            if stats.rewritten > 0 {
                                stats.rewritten -= 1;
                            }
                            stats.bailouts += 1;
                            status = "skipped (backup exists)".to_string();
                            final_content = Cow::Owned(source.clone());
                            is_rewrite = false;
                            if debug {
                                println!("• {} → skipped (backup exists)", rel_norm);
                            }
                        } else if let Err(err) = fs::write(&backup_path, &source) {
                            if stats.rewritten > 0 {
                                stats.rewritten -= 1;
                            }
                            stats.errors += 1;
                            eprintln!("failed to write backup {}: {}", backup_path.display(), err);
                            if debug {
                                println!("• {} → skipped (backup failed)", rel_norm);
                            }
                            continue;
                        }
                    }
                }

                if is_rewrite {
                    if let Cow::Owned(ref content) = final_content {
                        if let Err(err) = fs::write(&target_path, content) {
                            if stats.rewritten > 0 {
                                stats.rewritten -= 1;
                            }
                            stats.errors += 1;
                            eprintln!("failed to write {}: {}", target_path.display(), err);
                            if debug {
                                println!("• {} → skipped (write failed)", rel_norm);
                            }
                            continue;
                        }
                    }
                }
            } else {
                if let Some(parent) = target_path.parent() {
                    if let Err(err) = fs::create_dir_all(parent) {
                        stats.errors += 1;
                        eprintln!("failed to create directory {}: {}", parent.display(), err);
                        if debug {
                            println!("• {} → skipped (mkdir failed)", rel_norm);
                        }
                        continue;
                    }
                }

                if let Err(err) = fs::write(&target_path, final_content.as_ref()) {
                    stats.errors += 1;
                    eprintln!("failed to write {}: {}", target_path.display(), err);
                    if debug {
                        println!("• {} → skipped (write failed)", rel_norm);
                    }
                    continue;
                }
            }
        }

        let applied_renames = if is_rewrite { rename_total } else { 0 };
        stats.total_renames += applied_renames;

        if show_stats {
            stats.files.push(FileStats {
                path: rel_norm.clone(),
                renames: applied_renames,
                status: status.clone(),
            });
        }

        if show_stats {
            println!("• {} → {} (renames: {})", rel_norm, status, applied_renames);
        } else {
            println!("• {} → {}", rel_norm, status);
        }
    }

    if dry_run {
        if show_stats {
            println!(
                "Dry run complete: {} files matched → {} minified, {} skipped, {} bailouts, {} errors, {} renames. Output: {}",
                stats.processed,
                stats.rewritten,
                stats.skipped_no_change,
                stats.bailouts,
                stats.errors,
                stats.total_renames,
                if in_place {
                    input_dir.display().to_string()
                } else {
                    resolved_out_dir.display().to_string()
                }
            );
        } else {
            println!(
                "Dry run complete: {} files matched → {} minified, {} skipped, {} bailouts, {} errors. Output: {}",
                stats.processed,
                stats.rewritten,
                stats.skipped_no_change,
                stats.bailouts,
                stats.errors,
                if in_place {
                    input_dir.display().to_string()
                } else {
                    resolved_out_dir.display().to_string()
                }
            );
        }
    } else if show_stats {
        println!(
            "Processed {} files → {} minified, {} skipped, {} bailouts, {} errors, {} renames. Output: {}",
            stats.processed,
            stats.rewritten,
            stats.skipped_no_change,
            stats.bailouts,
            stats.errors,
            stats.total_renames,
            if in_place {
                input_dir.display().to_string()
            } else {
                resolved_out_dir.display().to_string()
            }
        );
    } else {
        println!(
            "Processed {} files → {} minified, {} skipped, {} bailouts, {} errors. Output: {}",
            stats.processed,
            stats.rewritten,
            stats.skipped_no_change,
            stats.bailouts,
            stats.errors,
            if in_place {
                input_dir.display().to_string()
            } else {
                resolved_out_dir.display().to_string()
            }
        );
    }

    if show_stats && json_output {
        let json = serde_json::to_string_pretty(&stats)?;
        println!("{}", json);
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result as AnyResult;
    use serde_json;
    use tempfile::tempdir;

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
        minify_dir(
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
        minify_dir(
            &input_dir,
            Some(output_dir.clone()),
            &["pkg_a/**".to_string()],
            &[],
            None,
            false,
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
        minify_dir(
            &input_dir,
            Some(output_dir.clone()),
            &[],
            &[],
            None,
            false,
            true,
            false,
            true,
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

        minify_dir(
            &input_dir,
            None,
            &[],
            &[],
            None,
            true,
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

        minify_dir(
            &input_dir,
            None,
            &[],
            &[],
            Some(".bak"),
            true,
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
        minify_dir(
            &input_dir,
            Some(output_dir),
            &[],
            &[],
            None,
            false,
            true,
            false,
            true,
            true,
        )?;

        Ok(())
    }

    #[test]
    fn minify_file_in_place_writes_backup() -> AnyResult<()> {
        let tmp = tempdir()?;
        let file_path = tmp.path().join("example.py");
        let original = "def foo(value):\n    temp = value + 1\n    return temp\n";
        fs::write(&file_path, original)?;

        minify_file(&file_path, true, Some(".bak"), false, false)?;

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

        minify_file(&file_path, false, None, true, true)?;

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

        apply_plan(&file_path, &plan_path, true, Some(".bak"), false, false)?;

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

        apply_plan(&file_path, &plan_path, false, None, true, true)?;

        Ok(())
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
        minify_plan_dir(&input_dir, &plan_path, &[], &[], false)?;
        assert!(plan_path.exists());

        let plan_bundle: PlanBundle = serde_json::from_str(&fs::read_to_string(&plan_path)?)?;
        assert_eq!(plan_bundle.files.len(), 2);

        let output_dir = tmp.path().join("out");
        apply_plan_dir(
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
            false,
        )?;

        let rewritten_module = fs::read_to_string(output_dir.join("module.py"))?;
        assert!(rewritten_module.contains("def foo(a):"));

        let rewritten_helper = fs::read_to_string(output_dir.join("pkg/helpers.py"))?;
        assert!(rewritten_helper.contains("def helper(a):"));

        Ok(())
    }
}
