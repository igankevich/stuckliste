use std::fs::File;
use std::io::Error;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use stuckliste::receipt::Receipt;

#[derive(Parser)]
struct Args {
    /// List block devices.
    #[arg(short = 'b')]
    list_block_devices: bool,
    /// List character devices.
    #[arg(short = 'c')]
    list_character_devices: bool,
    /// List directories.
    #[arg(short = 'd')]
    list_directories: bool,
    /// List files.
    #[arg(short = 'f')]
    list_files: bool,
    /// List symbolic links.
    #[arg(short = 'l')]
    list_symlinks: bool,
    /// Print modified time for regular files.
    #[arg(short = 'm')]
    print_mtime: bool,
    /// Print the paths only.
    #[arg(short = 's')]
    simple: bool,
    /// Suppress modes for directories and symbolic links.
    #[arg(short = 'x')]
    exclude_modes: bool,
    /// Print the size and the checksum for each executable file for the specified architecture.
    #[arg(long = "arch", value_name = "architecture")]
    arch: Option<String>,
    /// Format the output according to the supplied string.
    #[arg(short = 'p', value_name = "parameters")]
    format: Option<String>,
    /// BOM files.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "FILE"
    )]
    files: Vec<PathBuf>,
}

fn main() -> ExitCode {
    match do_main() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn do_main() -> Result<ExitCode, Error> {
    let args = Args::parse();
    if args.files.is_empty() {
        return Err(Error::other("no files specified"));
    }
    for path in args.files.into_iter() {
        print_bom(&path)
            .map_err(|e| Error::other(format!("failed to read {}: {}", path.display(), e)))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn print_bom(path: &Path) -> Result<(), Error> {
    let file = File::open(path)?;
    let bom = Receipt::read(file)?;
    for (path, _metadata) in bom.paths()? {
        println!("{}", path.display());
    }
    Ok(())
}
