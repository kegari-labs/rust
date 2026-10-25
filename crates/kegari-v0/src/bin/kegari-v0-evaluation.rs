//! Explicit read-only maintainer tooling; this entry never starts agent work.
use clap::{Parser, Subcommand};
use kegari_v0::{
    error::{Error, Result},
    evaluation::SuiteManifest,
    evaluation_reports::{ReportArchive, compare},
};
use std::{io::Write, path::PathBuf};
#[derive(Parser)]
#[command(about = "Inspect declared Kegari evaluation plans and retained evidence")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Plan {
        manifest: PathBuf,
    },
    Inspect {
        reports: PathBuf,
    },
    Compare {
        baseline: PathBuf,
        candidate: PathBuf,
    },
}
fn run(command: Command) -> Result<serde_json::Value> {
    match command {
        Command::Plan { manifest } => {
            let suite = SuiteManifest::load(&manifest)?;
            Ok(
                serde_json::json!({"suite_digest":suite.digest()?,"trials":suite.plan()?,"max_trials":suite.max_trials,
                "max_seconds":suite.max_seconds,"profiles":suite.profiles,"dispatch_authorized":false}),
            )
        }
        Command::Inspect { reports } => ReportArchive::open(&reports)?.inspection(),
        Command::Compare {
            baseline,
            candidate,
        } => compare(
            &ReportArchive::open(&baseline)?,
            &ReportArchive::open(&candidate)?,
        ),
    }
}
fn main() {
    let result = run(Cli::parse().command);
    let (exit, output) = match result {
        Ok(data) => (0, serde_json::json!({"ok":true,"data":data,"error":null})),
        Err(error) => (
            error.exit_code(),
            serde_json::json!({"ok":false,"data":null,"error":error}),
        ),
    };
    let stdout = std::io::stdout();
    let mut output_stream = stdout.lock();
    if serde_json::to_writer(&mut output_stream, &output).is_err()
        || output_stream.write_all(b"\n").is_err()
    {
        eprintln!(
            "{}",
            Error::new("output_error", "cannot write evaluation inspection")
        );
        std::process::exit(3);
    }
    std::process::exit(exit);
}
