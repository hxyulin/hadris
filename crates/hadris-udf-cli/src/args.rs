use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
#[command(name = "hadris-udf")]
#[command(author, version, about = "UDF filesystem utility", long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Display information about a UDF image
    Info(InfoArgs),
    /// List directory contents
    Ls(LsArgs),
    /// Display directory tree
    Tree(TreeArgs),
    /// Create a new UDF image
    Create(CreateArgs),
}

/// Display information about a UDF image
#[derive(Debug, Clone, Parser)]
pub struct InfoArgs {
    /// Path to UDF image
    pub input: PathBuf,
    /// Show detailed information
    #[arg(short, long)]
    pub verbose: bool,
}

/// List directory contents
#[derive(Debug, Clone, Parser)]
pub struct LsArgs {
    /// Path to UDF image
    pub input: PathBuf,
    /// Directory path within the image (default: root)
    #[arg(default_value = "/")]
    pub path: String,
    /// Use long listing format
    #[arg(short, long)]
    pub long: bool,
    /// Show all entries including hidden
    #[arg(short, long)]
    pub all: bool,
}

/// Display directory tree
#[derive(Debug, Clone, Parser)]
pub struct TreeArgs {
    /// Path to UDF image
    pub input: PathBuf,
    /// Starting directory path within the image
    #[arg(default_value = "/")]
    pub path: String,
    /// Maximum depth to display
    #[arg(short, long)]
    pub depth: Option<usize>,
}

/// Create a new UDF image
#[derive(Debug, Clone, Parser)]
pub struct CreateArgs {
    /// Directory containing files to include
    pub source: PathBuf,
    /// Output UDF image file path
    #[arg(short, long)]
    pub output: PathBuf,
    /// Volume name
    #[arg(short = 'V', long, default_value = "UDF_VOLUME")]
    pub volume_name: String,
    /// UDF revision (e.g. 1.02, 1.50, 2.01, 2.50)
    #[arg(short, long, default_value = "1.02")]
    pub revision: String,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
    /// Dry run: estimate size without creating the image
    #[arg(long)]
    pub dry_run: bool,
}
