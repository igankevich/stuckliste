use std::fs::File;
use std::io::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use stuckliste::receipt::Receipt;

#[derive(Parser)]
struct Args {
    /// Create simplified BOM that contains only paths.
    #[arg(short = 's')]
    simple: bool,
    /// File list.
    #[arg(short = 'i', value_name = "file")]
    file_list: Option<PathBuf>,
    /// Input directory.
    #[arg(value_name = "directory")]
    directory: Option<PathBuf>,
    /// Output file.
    #[arg(value_name = "bom")]
    bom: Option<PathBuf>,
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
    if args.directory.is_none() && args.file_list.is_none() {
        return Err(Error::other("neither directory nor file list is specified"));
    }
    let Some(output_path) = args.bom else {
        return Err(Error::other("output file is not specified"));
    };
    if let Some(directory) = args.directory {
        let bom = Receipt::from_directory(&directory)?;
        let file = File::create(&output_path)?;
        bom.write(file)?;
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}
