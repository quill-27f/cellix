use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut paths = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("cellix {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    match cellix::run(paths) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cellix: {e}");
            ExitCode::FAILURE
        }
    }
}
