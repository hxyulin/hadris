use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::ExitCode;

use fuser::{Config, MountOption};
use hadris_fat::sync::{FatFs, format};
use hadris_fat::{FatKind, FormatOptions, MountOptions};
use hadris_fs::sync::{StdMutex, Volume};
use hadris_fs::{HeapTable, SystemClock};
use hadris_fuse_prototype::{Adapter, Fuse};

const USAGE: &str = "usage:
  hadris-fuse-prototype format <image> <bytes> [fat12|fat16|fat32]
  hadris-fuse-prototype mount <image> <mountpoint> [--threads N] [--ro]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("format") if args.len() >= 3 => cmd_format(&args[1..]),
        Some("mount") if args.len() >= 3 => cmd_mount(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_format(args: &[String]) -> Result<(), String> {
    let size: u64 = args[1].parse().map_err(|e| format!("bad size: {e}"))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&args[0])
        .map_err(|e| e.to_string())?;
    file.set_len(size).map_err(|e| e.to_string())?;
    let mut options = FormatOptions::new().with_clock(SystemClock);
    options = match args.get(2).map(String::as_str) {
        Some("fat12") => options.with_kind(FatKind::Fat12),
        Some("fat16") => options.with_kind(FatKind::Fat16),
        Some("fat32") => options.with_kind(FatKind::Fat32),
        None => options,
        Some(other) => return Err(format!("unknown FAT kind {other}")),
    };
    let fs = format(file, options).map_err(|e| e.to_string())?;
    println!("formatted {} as {:?}", args[0], fs.kind());
    Ok(())
}

fn cmd_mount(args: &[String]) -> Result<(), String> {
    let image = PathBuf::from(&args[0]);
    let mountpoint = PathBuf::from(&args[1]);
    let mut threads = 1;
    let mut read_only = false;
    let mut rest = args[2..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--threads" => {
                threads = rest
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or("--threads needs a number")?;
            }
            "--ro" => read_only = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .write(!read_only)
        .open(&image)
        .map_err(|e| e.to_string())?;
    // A FUSE mount holds a pin on every inode the kernel caches, so the
    // default FixedTable<64> is far too small.
    let options = MountOptions::new()
        .with_table(HeapTable::new())
        .with_clock(SystemClock);
    let options = if read_only {
        options.with_read_only()
    } else {
        options
    };
    let fs = FatFs::open_with(file, options).map_err(|e| e.to_string())?;
    let volume: Volume<_, StdMutex> = Volume::new(fs);
    // SAFETY: getuid and getgid have no preconditions.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let adapter = Fuse(Adapter::new(volume, uid, gid));
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName(image.display().to_string()),
        MountOption::Subtype("hadris-fat".into()),
        MountOption::DefaultPermissions,
        if read_only {
            MountOption::RO
        } else {
            MountOption::RW
        },
    ];
    config.n_threads = Some(threads);
    fuser::mount(adapter, &mountpoint, &config).map_err(|e| format!("mount failed: {e}"))
}
