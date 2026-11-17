//! Run an external CLI (`glab`, `kubectl`, `aws`, `az`) with a time limit.
//!
//! A scheduled pipeline must not hang for ever because a login prompt or a
//! stuck API call never returns. After the limit the child is killed and the
//! caller gets a timeout error that names no output.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Default limit for one external command. `DEPGRAPH_CLI_TIMEOUT_SECS` overrides it.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// The limit in force: the environment variable when it is a positive number.
pub fn timeout() -> Duration {
    std::env::var("DEPGRAPH_CLI_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or(DEFAULT_TIMEOUT, Duration::from_secs)
}

/// Like `Command::output`, but kills the child after `limit`.
///
/// Standard input is closed, so a command that asks a question fails instead
/// of waiting for a person.
///
/// # Errors
///
/// Returns the spawn error, or `ErrorKind::TimedOut` after the limit.
pub fn output_within(command: &mut Command, limit: Duration) -> std::io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = std::thread::spawn(move || drain(stdout.as_mut()));
    let err = std::thread::spawn(move || drain(stderr.as_mut()));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "command timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// `output_within` with the configured limit.
///
/// # Errors
///
/// See [`output_within`].
pub fn output(command: &mut Command) -> std::io::Result<Output> {
    output_within(command, timeout())
}

fn drain<R: Read>(pipe: Option<&mut R>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
    }
    bytes
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn output_is_collected() {
        let result = output_within(
            Command::new("sh").args(["-c", "echo ok; echo bad >&2"]),
            Duration::from_secs(5),
        )
        .expect("runs");
        assert!(result.status.success());
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "ok");
        assert_eq!(String::from_utf8_lossy(&result.stderr).trim(), "bad");
    }

    #[test]
    fn a_hung_command_is_killed_at_the_limit() {
        let started = Instant::now();
        let err = output_within(
            Command::new("sh").args(["-c", "sleep 30"]),
            Duration::from_millis(200),
        )
        .expect_err("times out");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_command_that_reads_input_does_not_wait_for_a_person() {
        let result = output_within(&mut Command::new("cat"), Duration::from_secs(5)).expect("runs");
        assert!(result.status.success());
        assert!(result.stdout.is_empty());
    }
}
