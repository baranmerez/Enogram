//! The portable fallback owns the direct child, not its process tree. It bounds
//! the wait for inherited pipes but cannot promise descendant cleanup off Windows.
//! Never join an unfinished I/O worker after the deadline.

use super::{io_error, spawn_error, timed_out, DrainedStream};
use crate::{CapturedOutput, GitError, GitErrorKind, MAX_STDERR_BYTES};
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) fn run(
    command: &mut Command,
    stdout_limit: usize,
    input: Option<&[u8]>,
    timeout: Duration,
    started: Instant,
) -> Result<CapturedOutput, GitError> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard(command.spawn().map_err(spawn_error)?);
    let stdout = child.0.stdout.take().expect("piped stdout");
    let stderr = child.0.stderr.take().expect("piped stderr");
    let stdout_reader = spawn_drain(stdout, stdout_limit)
        .map_err(|error| io_error("Could not drain Git stdout", error))?;
    let stderr_reader = spawn_drain(stderr, MAX_STDERR_BYTES)
        .map_err(|error| io_error("Could not drain Git stderr", error))?;
    let input_writer = if let Some(input) = input {
        let mut stdin = child.0.stdin.take().expect("piped stdin");
        let bytes = input.to_vec();
        Some(
            thread::Builder::new()
                .name("enogram-git-input".to_owned())
                .spawn(move || stdin.write_all(&bytes))
                .map_err(|error| io_error("Could not write Git stdin", error))?,
        )
    } else {
        None
    };
    let mut status = None;
    let status = loop {
        if started.elapsed() >= timeout {
            return Err(timed_out(timeout));
        }
        if status.is_none() {
            status = child
                .0
                .try_wait()
                .map_err(|error| io_error("Could not wait for Git", error))?;
        }
        if let Some(status) = status {
            if stdout_reader.is_finished()
                && stderr_reader.is_finished()
                && input_writer.as_ref().is_none_or(JoinHandle::is_finished)
            {
                break status;
            }
        }
        thread::sleep(Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())));
    };
    let stdout = join_drain(stdout_reader)?;
    let stderr = join_drain(stderr_reader)?;
    if let Some(writer) = input_writer {
        let written = writer.join().map_err(|_| {
            GitError::new(GitErrorKind::GitCommandFailed, "Git input writer failed.")
        })?;
        if status.success() {
            written.map_err(|error| io_error("Git did not consume its complete input", error))?;
        }
    }
    Ok(CapturedOutput {
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        status,
    })
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_drain<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> io::Result<JoinHandle<io::Result<DrainedStream>>> {
    thread::Builder::new()
        .name("enogram-git-output".to_owned())
        .spawn(move || {
            let mut stream = DrainedStream::new(limit);
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => return Ok(stream),
                    Ok(count) => stream.append(&buffer[..count]),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
        })
}

fn join_drain(handle: JoinHandle<io::Result<DrainedStream>>) -> Result<DrainedStream, GitError> {
    handle
        .join()
        .map_err(|_| GitError::new(GitErrorKind::GitCommandFailed, "Git output reader failed."))?
        .map_err(|error| io_error("Could not read Git output", error))
}
