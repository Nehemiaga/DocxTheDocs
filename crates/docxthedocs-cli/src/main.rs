use clap::{Parser, Subcommand};
use docxthedocs::{ConvertOptions, convert_file};
use docxthedocs_ir::Status;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "docxthedocs",
    version,
    about = "Native DOC to DOCX converter with first-class Hebrew/RTL support"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Convert {
        source: PathBuf,
        destination: PathBuf,
        #[arg(long)]
        report: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Convert {
            source,
            destination,
            report,
        } => match convert_file(source, destination, ConvertOptions) {
            Ok(result) => {
                emit_report(&result.report, report.as_deref());
                exit_code(result.status)
            }
            Err(error) => {
                emit_report(&error.report, report.as_deref());
                eprintln!("ERROR: {error}");
                exit_code(error.report.status)
            }
        },
    };
    std::process::exit(code);
}

fn emit_report(report: &docxthedocs_ir::CapabilityReport, path: Option<&Path>) {
    let json = serde_json::to_string_pretty(report)
        .unwrap_or_else(|error| format!("{{\"status\":\"INTERNAL_ERROR\",\"error\":{error:?}}}"));
    println!("{json}");
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                eprintln!("ERROR: could not create report directory: {error}");
                return;
            }
        }
        let temp = path.with_extension("json.tmp");
        if let Err(error) = fs::write(&temp, json.as_bytes()).and_then(|_| fs::rename(&temp, path))
        {
            let _ = fs::remove_file(temp);
            eprintln!("ERROR: could not write report {}: {error}", path.display());
        }
    }
}

fn exit_code(status: Status) -> i32 {
    match status {
        Status::Converted => 0,
        Status::ConvertedWithWarnings => 10,
        Status::UnsupportedSource => 20,
        Status::InvalidSource => 21,
        Status::InternalError => 30,
    }
}
