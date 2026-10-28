use kegari_cli::local_cli;
use kegari_types::error::Result;
use kegari_v0::platform;
use std::io::Write;

/// Hidden process entrypoints are closed argument forms; public commands always
/// use the same typed local/HTTPS client path.
#[tokio::main]
async fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    let hidden = if arguments.len() == 2 {
        arguments[1].to_str()
    } else {
        None
    };
    let exit = match hidden {
        Some("__build-info") => {
            let mut stdout = std::io::stdout().lock();
            if serde_json::to_writer(&mut stdout, &platform::build_info()).is_ok()
                && stdout.write_all(b"\n").is_ok()
            {
                0
            } else {
                3
            }
        }
        Some("__handover-info") => match kegari_v0::housekeeping::handover_info() {
            Ok(info) => {
                let mut stdout = std::io::stdout().lock();
                if serde_json::to_writer(&mut stdout, &info).is_ok()
                    && stdout.write_all(b"\n").is_ok()
                {
                    0
                } else {
                    3
                }
            }
            Err(error) => {
                eprintln!("{error}");
                kegari_v0::error::exit_code(&error)
            }
        },
        Some("__orchestrate") => {
            internal_result(kegari_controller::orchestration::run_child().await, false)
        }
        Some("__control-host") => internal_result(
            kegari_controller::host::run_native(&kegari_v0::runtime_native::NativeRuntimeFactory)
                .await,
            true,
        ),
        _ => local_cli::run_cli(arguments).await,
    };
    std::process::exit(exit);
}
fn internal_result(result: Result<()>, control_host: bool) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) if control_host && error.code == "service_stopped" => 0,
        Err(error) => {
            // Stable safe Error messages never contain provider secrets or the
            // inherited orchestration handshake. No input bytes are logged.
            eprintln!("{error}");
            kegari_v0::error::exit_code(&error)
        }
    }
}
