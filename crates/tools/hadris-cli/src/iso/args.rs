use clap::Parser;
use hadris_iso::{IsoLevel, NameCase};
use std::{path::PathBuf, str::FromStr};

use crate::common::Target;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Display information about an ISO image
    Info(InfoArgs),
    /// List directory contents
    #[command(alias = "list")]
    Ls(LsArgs),
    /// Display directory tree
    Tree(TreeArgs),
    /// Extract files from an ISO image
    Extract(ExtractArgs),
    /// Create a new ISO image
    Create(CreateArgs),
    /// Verify ISO image integrity
    #[command(alias = "check")]
    Verify(VerifyArgs),
    /// xorriso-compatible mkisofs mode
    #[command(name = "mkisofs", alias = "xorriso")]
    Mkisofs(MkisofsArgs),
    /// Print file contents to stdout
    Cat(CatArgs),
}

/// Display information about an ISO image
#[derive(Debug, Clone, Parser)]
pub struct InfoArgs {
    /// Path to ISO image
    pub input: PathBuf,
    /// Show detailed volume descriptor information
    #[arg(short, long)]
    pub verbose: bool,
}

/// List directory contents
#[derive(Debug, Clone, Parser)]
pub struct LsArgs {
    /// Path to ISO image
    pub input: PathBuf,
    /// Directory path within ISO (default: root)
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
    /// Path to ISO image
    pub input: PathBuf,
    /// Starting directory path within ISO
    #[arg(default_value = "/")]
    pub path: String,
    /// Maximum depth to display
    #[arg(short, long)]
    pub depth: Option<usize>,
}

/// Extract files from an ISO image
#[derive(Debug, Clone, Parser)]
pub struct ExtractArgs {
    /// Path to ISO image
    pub input: PathBuf,
    /// Output directory for extracted files
    #[arg(short, long, default_value = ".")]
    pub output: PathBuf,
    /// Path within ISO to extract (default: extract all); a path other than
    /// the root is written to `<output>/<name>`
    #[arg(short, long)]
    pub path: Option<String>,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

/// Create a new ISO image
#[derive(Debug, Clone, Parser)]
pub struct CreateArgs {
    /// Directory containing files to include
    pub source: PathBuf,
    #[command(flatten)]
    pub target: Target,
    #[command(flatten)]
    pub iso: IsoFlags,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
    /// Dry run: estimate size without creating the ISO
    #[arg(long)]
    pub dry_run: bool,
}

/// The ISO 9660 options of `iso create` and `udf bridge`.
#[derive(Debug, Clone, clap::Args)]
pub struct IsoFlags {
    /// Volume name (max 32 characters)
    #[arg(short = 'V', long, default_value = "CDROM")]
    pub volume_name: String,
    /// ISO level (1, 2, 3, 1l, or 2l; the `l` variants preserve lowercase)
    #[arg(short, long, default_value = "1")]
    pub level: ArgLevel,
    /// Enable Joliet extension for Windows compatibility
    #[arg(short = 'J', long)]
    pub joliet: bool,
    /// Enable Rock Ridge extension for Unix compatibility
    #[arg(short = 'R', long)]
    pub rock_ridge: bool,
    /// Image path of the El Torito BIOS boot image
    #[arg(short, long)]
    pub boot: Option<String>,
    /// Image path of the El Torito UEFI boot image
    #[arg(long)]
    pub efi_boot: Option<String>,
    /// Number of 512-byte sectors to load for the BIOS boot image
    #[arg(long, default_value = "4")]
    pub boot_load_size: u16,
    /// Add a boot information table to the BIOS boot image
    #[arg(long, requires = "boot")]
    pub boot_info_table: bool,
    /// Enable MBR hybrid boot for USB booting
    #[arg(long)]
    pub hybrid_mbr: bool,
    /// Enable GPT hybrid boot for UEFI USB booting
    #[arg(long)]
    pub hybrid_gpt: bool,
    /// System identifier (max 32 characters)
    #[arg(long, alias = "sysid")]
    pub system_id: Option<String>,
    /// Volume set identifier (max 128 characters)
    #[arg(long, alias = "volset")]
    pub volume_set_id: Option<String>,
    /// Publisher identifier (max 128 characters)
    #[arg(long, alias = "publisher")]
    pub publisher_id: Option<String>,
    /// Data preparer identifier (max 128 characters)
    #[arg(long, alias = "preparer")]
    pub preparer_id: Option<String>,
    /// Application identifier (max 128 characters)
    #[arg(long, alias = "appid")]
    pub application_id: Option<String>,
    /// Auto-uppercase and fix invalid characters for ECMA-119 compliance
    #[arg(long)]
    pub strict_charset: bool,
}

/// Verify ISO image integrity
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    /// Path to ISO image
    pub input: PathBuf,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
    /// Enable strict checks (path table consistency, extent bounds, RRIP field validation)
    #[arg(short, long)]
    pub strict: bool,
}

/// xorriso-compatible mkisofs mode
#[derive(Debug, Clone, Parser)]
pub struct MkisofsArgs {
    /// Source directory
    pub source: PathBuf,
    /// Output file
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Volume name
    #[arg(short = 'V')]
    pub volume_name: Option<String>,
    /// Enable Joliet extension
    #[arg(short = 'J')]
    pub joliet: bool,
    /// Enable Rock Ridge extension
    #[arg(short = 'R')]
    pub rock_ridge: bool,
    /// Boot image (El-Torito)
    #[arg(short = 'b')]
    pub boot_image: Option<String>,
    /// No emulation boot
    #[arg(long = "no-emul-boot")]
    pub no_emul_boot: bool,
    /// Boot load size in sectors
    #[arg(long = "boot-load-size")]
    pub boot_load_size: Option<u16>,
    /// Boot info table
    #[arg(long = "boot-info-table")]
    pub boot_info_table: bool,
    /// EFI boot image
    #[arg(short = 'e', long = "efi-boot")]
    pub efi_boot: Option<String>,
    /// Hybrid MBR
    #[arg(long = "isohybrid-mbr")]
    pub isohybrid_mbr: Option<PathBuf>,
    /// Replace an existing output file, or write over an existing device
    #[arg(long)]
    pub force: bool,
}

/// Print file contents to stdout
#[derive(Debug, Clone, Parser)]
pub struct CatArgs {
    /// Path to ISO image
    pub input: PathBuf,
    /// File path within ISO (e.g., /SUBDIR/FILE.TXT)
    pub path: String,
}

/// An ISO level, and whether names keep their case.
#[derive(Debug, Clone, Copy)]
pub struct ArgLevel {
    pub level: IsoLevel,
    pub name_case: NameCase,
}

impl Default for ArgLevel {
    fn default() -> Self {
        Self {
            level: IsoLevel::L1,
            name_case: NameCase::Upper,
        }
    }
}

impl FromStr for ArgLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (level, name_case) = match s {
            "1" => (IsoLevel::L1, NameCase::Upper),
            "2" => (IsoLevel::L2, NameCase::Upper),
            "1l" => (IsoLevel::L1, NameCase::Preserve),
            "2l" => (IsoLevel::L2, NameCase::Preserve),
            "3" => (IsoLevel::L3, NameCase::Preserve),
            _ => return Err("invalid level (use 1, 2, 1l, 2l, or 3)"),
        };
        Ok(Self { level, name_case })
    }
}
