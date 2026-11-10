//! Command-scoped Git supervision. A timeout is never a rollback.
//!
//! On Windows the job owns Git before its first instruction and owns the helpers
//! it creates. Completion means the root exited and stdin/stdout/stderr finished;
//! helpers still alive then are terminated when the job closes. Background work
//! deliberately outliving a hook is not part of a successful command result.

use crate::{CapturedOutput, GitError, GitErrorKind};
use std::io;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(not(windows))]
#[path = "git_process/portable.rs"]
mod platform;
#[cfg(windows)]
#[path = "git_process/windows.rs"]
mod platform;

pub(super) fn run(
    command: &mut Command,
    stdout_limit: usize,
    allow_stdout_truncation: bool,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<CapturedOutput, GitError> {
    let output = platform::run(command, stdout_limit, input, timeout, Instant::now())?;
    if output.stdout_truncated && !allow_stdout_truncation {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Git command output exceeded the configured limit.",
        ));
    }
    Ok(output)
}

struct DrainedStream {
    bytes: Vec<u8>,
    truncated: bool,
    limit: usize,
}

impl DrainedStream {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(64 * 1024)),
            truncated: false,
            limit,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let retain = self.limit.saturating_sub(self.bytes.len()).min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..retain]);
        self.truncated |= retain < bytes.len();
    }
}

fn timed_out(timeout: Duration) -> GitError {
    GitError::new(
        GitErrorKind::TimedOut,
        format!(
            "Git command timed out after {} seconds.",
            timeout.as_secs_f64()
        ),
    )
}

fn io_error(operation: &str, error: io::Error) -> GitError {
    GitError::new(
        GitErrorKind::GitCommandFailed,
        format!("{operation}: {error}"),
    )
}

fn spawn_error(error: io::Error) -> GitError {
    if error.kind() == io::ErrorKind::NotFound {
        GitError::new(
            GitErrorKind::GitUnavailable,
            "Git executable could not be started.",
        )
    } else {
        io_error("Could not start Git", error)
    }
}

#[cfg(all(test, windows))]
mod tests;
