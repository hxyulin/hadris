use clap::Parser;
use hadris_iso::write::options::BaseIsoLevel;
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Clone, Parser)]
#[command(name = "hadris-iso")]
#[command(author, version, about = "ISO 9660 filesystem utility", long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Display information about an ISO image
    Info(InfoArgs),
    /// List directory contents
    Ls(LsArgs),
    /// Display directory tree
    Tree(TreeArgs),
    /// Extract files from an ISO image
    Extract(ExtractArgs),
    /// Create a new ISO image
    Create(CreateArgs),
    /// Verify ISO image integrity
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
    /// Path within ISO to extract (default: extract all)
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
    /// Output ISO file path
    #[arg(short, long)]
    pub output: PathBuf,
    /// Volume name (max 32 characters)
    #[arg(short = 'V', long, default_value = "CDROM")]
    pub volume_name: String,
    /// ISO level (1, 2, 1l, 2l for lowercase support)
    #[arg(short, long, default_value = "1")]
    pub level: ArgLevel,
    /// Enable Joliet extension for Windows compatibility
    #[arg(short = 'J', long)]
    pub joliet: bool,
    /// Enable Rock Ridge extension for Unix compatibility
    #[arg(short = 'R', long)]
    pub rock_ridge: bool,
    /// Boot image path for BIOS boot (El-Torito)
    #[arg(short, long)]
    pub boot: Option<String>,
    /// Boot image path for UEFI boot
    #[arg(long)]
    pub efi_boot: Option<String>,
    /// Number of 512-byte sectors to load for boot image
    #[arg(long, default_value = "4")]
    pub boot_load_size: u16,
    /// Enable boot info table in boot image
    #[arg(long)]
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
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
    /// Dry run: estimate size without creating the ISO
    #[arg(long)]
    pub dry_run: bool,
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
}

/// Print file contents to stdout
#[derive(Debug, Clone, Parser)]
pub struct CatArgs {
    /// Path to ISO image
    pub input: PathBuf,
    /// File path within ISO (e.g., /SUBDIR/FILE.TXT)
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct ArgLevel(pub BaseIsoLevel);

impl Default for ArgLevel {
    fn default() -> Self {
        Self(BaseIsoLevel::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        })
    }
}

impl FromStr for ArgLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "1" => Self(BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            }),
            "2" => Self(BaseIsoLevel::Level2 {
                supports_lowercase: false,
                supports_rrip: false,
            }),
            "1l" => Self(BaseIsoLevel::Level1 {
                supports_lowercase: true,
                supports_rrip: false,
            }),
            "2l" => Self(BaseIsoLevel::Level2 {
                supports_lowercase: true,
                supports_rrip: false,
            }),
            "3" => Self(BaseIsoLevel::Level2 {
                supports_lowercase: true,
                supports_rrip: false,
            }),
            _ => return Err("invalid level (use 1, 2, 1l, 2l, or 3)"),
        })
    }
}
