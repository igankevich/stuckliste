use std::fs::File;
use std::io::Error;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::SystemTime;

use bitflags::bitflags;
use chrono::DateTime;
use chrono::Local;
use clap::Parser;
use stuckliste::receipt::FileType;
use stuckliste::receipt::Metadata;
use stuckliste::receipt::Receipt;

#[derive(Parser)]
#[clap(arg_required_else_help = true, about = "List contents of a BOM file")]
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
    paths_only: bool,
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

impl Args {
    fn list(&self) -> List {
        let mut list = List::empty();
        if self.list_block_devices {
            list.insert(List::BlockDevices);
        }
        if self.list_character_devices {
            list.insert(List::CharDevices);
        }
        if self.list_files {
            list.insert(List::Files);
        }
        if self.list_directories {
            list.insert(List::Directories);
        }
        if self.list_symlinks {
            list.insert(List::Symlinks);
        }
        if list.is_empty() {
            list = List::all();
        }
        list
    }
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
    if args.format.is_some() {
        return Err(Error::other("`-p` option is not supported"));
    }
    for path in args.files.iter() {
        print_bom(path, &args)
            .map_err(|e| Error::other(format!("failed to read {}: {}", path.display(), e)))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn print_bom(path: &Path, args: &Args) -> Result<(), Error> {
    let file = File::open(path)?;
    let bom = Receipt::read(file)?;
    let entries = bom.entries()?;
    let list = args.list();
    let mut line = String::with_capacity(4096);
    for (path, metadata) in entries.iter() {
        line.clear();
        let print = write_entry(&mut line, path, metadata, args, &list)?;
        if print {
            println!("{line}");
        }
    }
    Ok(())
}

#[inline]
fn write_entry(
    line: &mut String,
    path: &Path,
    metadata: &Metadata,
    args: &Args,
    list: &List,
) -> Result<bool, Error> {
    use std::fmt::Write;
    match &metadata {
        Metadata::File(file) if list.contains(List::Files) => {
            let _ = write!(line, "{}", path.display());
            if !args.paths_only {
                write_common(line, metadata, false);
                write!(line, "\t{}\t{}", metadata.size(), file.checksum()).map_err(Error::other)?;
                if args.print_mtime {
                    let timestamp: DateTime<Local> =
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH).into();
                    write!(line, "\t{}", timestamp.format(LSBOM_TIME)).map_err(Error::other)?;
                }
            }
            Ok(true)
        }
        Metadata::Executable(exe) if list.contains(List::Files) => {
            let _ = write!(line, "{}", path.display());
            let cpu_type = args
                .arch
                .as_ref()
                .map(|arch| arch_to_cpu_type(arch))
                .transpose()?;
            let arch = cpu_type
                .and_then(|cpu_type| exe.arches().iter().find(|arch| arch.cpu_type() == cpu_type));
            let print = cpu_type.is_none() || arch.is_some();
            if !args.paths_only {
                write_common(line, metadata, false);
                if let Some(arch) = arch {
                    let _ = write!(line, "\t{}\t{}", arch.size(), arch.checksum());
                } else if cpu_type.is_none() {
                    let _ = write!(line, "\t{}\t{}", metadata.size(), exe.checksum());
                }
                if print && args.print_mtime {
                    let timestamp: DateTime<Local> =
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH).into();
                    write!(line, "\t{}", timestamp.format(LSBOM_TIME)).map_err(Error::other)?;
                }
            }
            Ok(print)
        }
        Metadata::Link(link) if list.contains(List::Symlinks) => {
            let _ = write!(line, "{}", path.display());
            if !args.paths_only {
                write_common(line, metadata, args.exclude_modes);
                write!(
                    line,
                    "\t{}\t{}\t{}",
                    metadata.size(),
                    link.checksum(),
                    link.target().display()
                )
                .map_err(Error::other)?;
            }
            Ok(true)
        }
        Metadata::Device(dev) => {
            let file_type = FileType::new(metadata.mode())?;
            let print = match file_type {
                FileType::BlockDevice if list.contains(List::BlockDevices) => true,
                FileType::CharDevice if list.contains(List::CharDevices) => true,
                _ => false,
            };
            if print {
                let _ = write!(line, "{}", path.display());
                if !args.paths_only {
                    write_common(line, metadata, false);
                    write!(line, "\t{}", dev.rdev()).map_err(Error::other)?;
                }
            }
            Ok(print)
        }
        Metadata::Entry(..) => {
            write!(line, "{}", path.display()).map_err(Error::other)?;
            Ok(true)
        }
        Metadata::Directory(..) if list.contains(List::Directories) => {
            let _ = write!(line, "{}", path.display());
            if !args.paths_only {
                write_common(line, metadata, args.exclude_modes);
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn write_common(line: &mut String, metadata: &Metadata, exclude_modes: bool) {
    use std::fmt::Write;
    if exclude_modes {
        let _ = write!(line, "\t{}/{}", metadata.uid(), metadata.gid());
    } else {
        let _ = write!(
            line,
            "\t{:o}\t{}/{}",
            metadata.mode(),
            metadata.uid(),
            metadata.gid()
        );
    }
}

bitflags! {
    struct List: u8 {
        const Files        = 0b00000001;
        const BlockDevices = 0b00000010;
        const CharDevices  = 0b00000100;
        const Directories  = 0b00001000;
        const Symlinks     = 0b00010000;
    }
}

// See mach/machine.h
fn arch_to_cpu_type(s: &str) -> Result<u32, Error> {
    let s = s.to_ascii_lowercase();
    match s.as_str() {
        "hppa" => Ok(CPU_TYPE_HPPA),
        "arm" => Ok(CPU_TYPE_ARM),
        "arm64" => Ok(CPU_TYPE_ARM | CPU_ARCH_ABI64),
        "arm64_32" => Ok(CPU_TYPE_ARM | CPU_ARCH_ABI64_32),
        "sparc" => Ok(CPU_TYPE_SPARC),
        "x86" | "i386" => Ok(CPU_TYPE_X86),
        "x86_64" => Ok(CPU_TYPE_X86 | CPU_ARCH_ABI64),
        "powerpc" | "ppc" => Ok(CPU_TYPE_POWERPC),
        "powerpc64" | "ppc64" => Ok(CPU_TYPE_POWERPC | CPU_ARCH_ABI64),
        other => Err(Error::other(format!("unknown arch: {}", other))),
    }
}

const CPU_TYPE_X86: u32 = 7;
const CPU_TYPE_HPPA: u32 = 11;
const CPU_TYPE_ARM: u32 = 12;
const CPU_TYPE_SPARC: u32 = 14;
const CPU_TYPE_POWERPC: u32 = 18;
const CPU_ARCH_ABI64: u32 = 0x01000000;
const CPU_ARCH_ABI64_32: u32 = 0x02000000;

const LSBOM_TIME: &str = "%a %b %d %H:%M:%S %Y";
