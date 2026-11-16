#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    match depgraph::cli::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            if let depgraph::Error::Usage(message) = &err {
                eprint!("{message}");
            } else {
                eprintln!("error: {}", depgraph::redact::redact(&err.to_string()));
            }
            std::process::ExitCode::from(err.exit_code())
        }
    }
}
