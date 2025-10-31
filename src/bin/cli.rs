//! CLI for tree-shaking operations

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::filter::EnvFilter;
use tsrs::{VenvAnalyzer, VenvSlimmer};

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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let env_filter = if cli.debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .init();

    match cli.command {
        Commands::Analyze { venv_path } => {
            analyze(&venv_path)?;
        }
        Commands::Slim { code_path, venv_path, output } => {
            slim(&code_path, &venv_path, output)?;
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
        let parent = venv_path.parent()
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
