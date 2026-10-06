use clap::Parser;
use std::path::PathBuf;

use crate::common::{RevisionArg, Target};
use crate::iso::IsoFlags;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Display information about a UDF image
    Info(InfoArgs),
    /// List directory contents
    #[command(alias = "list")]
    Ls(LsArgs),
    /// Display directory tree
    Tree(TreeArgs),
    /// Print file contents to stdout
    Cat(CatArgs),
    /// Extract files from a UDF image
    Extract(ExtractArgs),
    /// Create a new UDF image
    Create(CreateArgs),
    /// Verify UDF image structural integrity
    #[command(alias = "check")]
    Verify(VerifyArgs),
    /// Create an ISO 9660 and UDF bridge image
    Bridge(BridgeArgs),
    /// Compare the ISO 9660 and UDF trees of a bridge image
    Compare(CompareArgs),
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
    /// Show all entries including . and ..
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

/// Print file contents to stdout
#[derive(Debug, Clone, Parser)]
pub struct CatArgs {
    /// Path to UDF image
    pub input: PathBuf,
    /// Path within the image
    pub path: String,
}

/// Extract files from a UDF image
#[derive(Debug, Clone, Parser)]
pub struct ExtractArgs {
    /// Path to UDF image
    pub input: PathBuf,
    /// Output directory for extracted files
    #[arg(short, long, default_value = ".")]
    pub output: PathBuf,
    /// Path within the image to extract (default: extract all); a path
    /// other than the root is written to `<output>/<name>`
    #[arg(short, long)]
    pub path: Option<String>,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

/// Create a new UDF image
#[derive(Debug, Clone, Parser)]
pub struct CreateArgs {
    /// Directory containing files to include
    pub source: PathBuf,
    #[command(flatten)]
    pub target: Target,
    /// Volume name
    #[arg(short = 'V', long, default_value = "UDF_VOLUME")]
    pub volume_name: String,
    /// UDF revision (1.02, 1.50, 2.00, 2.01, 2.50, or 2.60)
    #[arg(short, long, default_value = "1.02")]
    pub revision: RevisionArg,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
    /// Dry run: estimate size without creating the image
    #[arg(long)]
    pub dry_run: bool,
}

/// Verify UDF image structural integrity
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    /// Path to UDF image
    pub input: PathBuf,
    /// Print every path as it is checked
    #[arg(short, long)]
    pub verbose: bool,
}

/// Create an ISO 9660 and UDF bridge image
#[derive(Debug, Clone, Parser)]
pub struct BridgeArgs {
    /// Directory containing files to include
    pub source: PathBuf,
    #[command(flatten)]
    pub target: Target,
    /// ISO 9660 options; the volume name names both volumes
    #[command(flatten)]
    pub iso: IsoFlags,
    /// UDF revision (1.02, 1.50, 2.00, 2.01, 2.50, or 2.60)
    #[arg(short, long, default_value = "1.02")]
    pub revision: RevisionArg,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
    /// Dry run: estimate size without creating the image
    #[arg(long)]
    pub dry_run: bool,
}

/// Compare the ISO 9660 and UDF trees of a bridge image
#[derive(Debug, Clone, Parser)]
pub struct CompareArgs {
    /// Path to the bridge image
    pub input: PathBuf,
}
