//! Disposable Windows process fixtures. No user repository/configuration or
//! index locks are touched. Each case supplies its own short deadline.

use super::run;
use crate::{CapturedOutput, GitError, GitErrorKind, MAX_STDERR_BYTES};
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{
    CompareObjectHandles, DuplicateHandle, SetHandleInformation, DUPLICATE_SAME_ACCESS,
    ERROR_INVALID_HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE,
};

const FIXTURE_TEST: &str = "git_process::tests::fixture_process";
const MODE: &str = "ENOGRAM_PROCESS_FIXTURE_MODE";
const DIRECTORY: &str = "ENOGRAM_PROCESS_FIXTURE_DIRECTORY";
const DEADLINE: Duration = Duration::from_millis(1200);
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "enogram-process-{}-{time}-{id} ıslak çay",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn command(&self, mode: &str) -> Command {
        fixture_command(mode, &self.path)
    }

    fn observe(&self, name: &str) -> ProcessProbe {
        let path = self.path.join(name);
        let started = Instant::now();
        loop {
            if let Ok(pid) = fs::read_to_string(&path) {
                if let Ok(pid) = pid.parse::<u32>() {
                    // Own a process handle while it is still alive; no PID/name
                    // scans or risk of killing a reused PID during cleanup.
                    let raw =
                        unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
                    assert!(
                        !raw.is_null(),
                        "open {name} process {pid}: {}",
                        std::io::Error::last_os_error()
                    );
                    fs::write(self.path.join(format!("observed-{name}")), "observed").unwrap();
                    return ProcessProbe(unsafe { OwnedHandle::from_raw_handle(raw) });
                }
            }
            assert!(
                started.elapsed() < DEADLINE,
                "fixture did not publish {name}"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn start(
        &self,
        mode: &str,
        input: Option<Vec<u8>>,
    ) -> mpsc::Receiver<Result<CapturedOutput, GitError>> {
        let mut command = self.command(mode);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = run(&mut command, 1024, true, input.as_deref(), DEADLINE);
            let _ = sender.send(result);
        });
        receiver
    }

    fn assert_no_marker(&self) {
        assert!(
            !self.path.join("late-marker").exists(),
            "helper wrote after timeout/return"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct ProcessProbe(OwnedHandle);

impl ProcessProbe {
    fn assert_exited(&self) {
        // Killing the job can complete asynchronously; this bounded OS wait
        // proves the exact process exited, rather than only checking a marker.
        assert_eq!(
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 1000) },
            WAIT_OBJECT_0
        );
    }
}

impl Drop for ProcessProbe {
    fn drop(&mut self) {
        // Ensure a failed regression test cannot leak one of its own fixtures.
        // The retained handle identifies exactly the observed test process.
        if unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } != WAIT_OBJECT_0 {
            unsafe {
                TerminateProcess(self.0.as_raw_handle(), 1);
                WaitForSingleObject(self.0.as_raw_handle(), 1000);
            }
        }
    }
}

struct Unrelated {
    process: ProcessProbe,
}

impl Unrelated {
    fn spawn(command: &Command) -> Self {
        // This sleeper must be unrelated to every supervised command, including
        // concurrent tests' temporary inheritable pipe endpoints.
        Self {
            process: ProcessProbe(super::platform::spawn_unrelated(command).unwrap()),
        }
    }

    fn assert_running(&self) {
        assert_eq!(
            unsafe { WaitForSingleObject(self.process.0.as_raw_handle(), 0) },
            WAIT_TIMEOUT,
            "unrelated process was killed"
        );
    }
}

fn receive(
    receiver: mpsc::Receiver<Result<CapturedOutput, GitError>>,
) -> Result<CapturedOutput, GitError> {
    // A regression must fail the test instead of hanging the test suite.
    receiver
        .recv_timeout(Duration::from_secs(4))
        .expect("supervisor did not return within bounded test time")
}

#[test]
fn timeout_kills_sleeping_tree_without_killing_unrelated_process() {
    let fixture = Fixture::new();
    let unrelated = Unrelated::spawn(&fixture.command("unrelated"));
    let started = Instant::now();
    let result = fixture.start("tree", None);
    let root = fixture.observe("root.pid");
    let helper = fixture.observe("helper.pid");
    let error = receive(result).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(4));
    root.assert_exited();
    helper.assert_exited();
    fixture.assert_no_marker();
    unrelated.assert_running();
}

#[test]
fn deadline_covers_pipes_inherited_after_root_exits() {
    let fixture = Fixture::new();
    let result = fixture.start("orphan-output", None);
    let root = fixture.observe("root.pid");
    let helper = fixture.observe("helper.pid");
    root.assert_exited();
    let error = receive(result).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::TimedOut);
    assert!(fixture.path.join("root-exited").exists());
    helper.assert_exited();
    fixture.assert_no_marker();
}

#[test]
fn deadline_covers_blocked_stdin_after_root_and_output_exit() {
    let fixture = Fixture::new();
    let result = fixture.start("orphan-input", Some(vec![b'i'; 2 * 1024 * 1024]));
    let root = fixture.observe("root.pid");
    let helper = fixture.observe("helper.pid");
    root.assert_exited();
    let error = receive(result).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::TimedOut);
    assert!(fixture.path.join("root-exited").exists());
    helper.assert_exited();
    fixture.assert_no_marker();
}

#[test]
fn output_flood_and_blocked_stdin_do_not_starve_deadline() {
    let fixture = Fixture::new();
    let result = fixture.start("flood-blocked-input", Some(vec![b'i'; 2 * 1024 * 1024]));
    let root = fixture.observe("root.pid");
    let error = receive(result).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::TimedOut);
    root.assert_exited();
}

#[test]
fn normal_return_closes_command_scoped_background_helpers() {
    let fixture = Fixture::new();
    let result = fixture.start("background", None);
    let root = fixture.observe("root.pid");
    let helper = fixture.observe("helper.pid");
    assert!(receive(result).unwrap().status.success());
    root.assert_exited();
    helper.assert_exited();
    fixture.assert_no_marker();
}

#[test]
fn unrelated_fixture_spawn_cannot_hold_command_pipes_open() {
    let fixture = Fixture::new();
    let unrelated_fixture = Fixture::new();
    let mut command = fixture.command("background");
    let unrelated_command = unrelated_fixture.command("unrelated");
    let (unrelated_sender, unrelated_receiver) = mpsc::channel();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        super::platform::with_spawn_probe(
            move |handles| {
                // Force the overlap that ordinary parallel test execution only
                // sometimes hits: the runner's inheritable copies exist when
                // the unrelated sleeper starts. Retain cleanup before probing.
                let unrelated = Unrelated::spawn(&unrelated_command);
                for (name, handle) in ["stdin", "stdout", "stderr"].iter().zip(handles) {
                    let mut copy = std::ptr::null_mut();
                    let copied = unsafe {
                        DuplicateHandle(
                            unrelated.process.0.as_raw_handle(),
                            handle.as_raw_handle(),
                            GetCurrentProcess(),
                            &mut copy,
                            0,
                            0,
                            DUPLICATE_SAME_ACCESS,
                        )
                    };
                    if copied != 0 {
                        // The same numeric value can identify a different
                        // object in the peer; compare actual kernel objects.
                        let copy = unsafe { OwnedHandle::from_raw_handle(copy) };
                        assert_eq!(
                            unsafe {
                                CompareObjectHandles(copy.as_raw_handle(), handle.as_raw_handle())
                            },
                            0,
                            "unrelated fixture inherited command {name}"
                        );
                    } else {
                        assert_eq!(
                            std::io::Error::last_os_error().raw_os_error(),
                            Some(ERROR_INVALID_HANDLE as i32),
                            "could not inspect unrelated fixture's {name} handle"
                        );
                    }
                }
                unrelated_sender.send(unrelated).unwrap();
            },
            || {
                let result = run(&mut command, 1024, true, None, DEADLINE);
                let _ = sender.send(result);
            },
        );
    });
    let unrelated = unrelated_receiver
        .recv_timeout(Duration::from_secs(4))
        .expect("unrelated fixture did not start without inheriting command handles");
    let root = fixture.observe("root.pid");
    let helper = fixture.observe("helper.pid");
    root.assert_exited();
    let result = receive(receiver);
    unrelated.assert_running();
    assert!(result.unwrap().status.success());
    helper.assert_exited();
    fixture.assert_no_marker();
}

#[test]
fn bounded_output_keeps_draining_and_preserves_nonzero_exit() {
    let fixture = Fixture::new();
    let output = run(
        &mut fixture.command("finite-flood"),
        257,
        true,
        None,
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(output.stdout.len(), 257);
    assert_eq!(output.stderr.len(), MAX_STDERR_BYTES);
    assert!(output.stdout_truncated);
    assert!(output.stderr_truncated);

    let error = run(
        &mut fixture.command("finite-flood"),
        257,
        false,
        None,
        Duration::from_secs(10),
    )
    .unwrap_err();
    assert_eq!(error.kind, GitErrorKind::OutputLimit);
}

#[test]
fn real_exit_code_259_is_not_treated_as_still_running() {
    let fixture = Fixture::new();
    let output = run(
        &mut fixture.command("exit259"),
        4096,
        false,
        Some(&vec![b'i'; 1024 * 1024]),
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(output.status.code(), Some(259));
    assert!(String::from_utf8_lossy(&output.stderr).contains("fixture exit diagnostic"));
}

#[test]
fn stdin_and_unicode_arguments_environment_and_working_directory_round_trip() {
    let fixture = Fixture::new();
    let mut command = fixture.command("echo");
    let arguments = [
        "space separated",
        "ıslak çay 🦀",
        "one\"quote",
        "\\before\\\"quote",
        "ends with slash\\",
        "line\nfeed",
        "%PATH% & | > < ^ $()`",
    ];
    // libtest accepts arbitrary --skip values. The helper inspects its real
    // argv to verify the native Windows quoting without using a shell.
    for argument in arguments {
        command.arg("--skip").arg(argument);
    }
    command
        .env("pAtH", "fixture-path-value")
        .env_remove("COMSPEC");
    command
        .env("ENOGRAM_CASE_Å", "first")
        .env("enogram_case_å", "second");
    let input: Vec<u8> = (0..1024 * 1024 + 117)
        .map(|index| (index % 251) as u8)
        .collect();
    let output = run(
        &mut command,
        16 * 1024,
        false,
        Some(&input),
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let json = text
        .split_once("ENOGRAM_ECHO=")
        .expect("fixture response")
        .1
        .trim();
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(value["length"], input.len());
    assert_eq!(value["checksum"], checksum(&input));
    assert_eq!(value["arguments"], serde_json::json!(arguments));
    assert_eq!(value["path"], "fixture-path-value");
    assert_eq!(value["comspec"], serde_json::Value::Null);
    assert_eq!(value["caseValue"], "second");
    assert_eq!(value["caseCount"], 1);
    assert_eq!(value["directory"], fixture.path.to_str().unwrap());
}

#[test]
fn spawn_failure_returns_promptly_without_starting_a_process() {
    let fixture = Fixture::new();
    let mut command = Command::new(fixture.path.join("does-not-exist.exe"));
    let error = run(&mut command, 4096, false, None, DEADLINE).unwrap_err();
    assert_eq!(error.kind, GitErrorKind::GitUnavailable);
    fixture.assert_no_marker();
}

fn fixture_command(mode: &str, path: &PathBuf) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", FIXTURE_TEST, "--ignored", "--nocapture"]);
    command
        .env(MODE, mode)
        .env(DIRECTORY, path)
        .current_dir(path);
    command
}

fn checksum(input: &[u8]) -> u64 {
    input.iter().fold(0u64, |value, byte| {
        value.wrapping_mul(31).wrapping_add(*byte as u64)
    })
}

#[test]
#[ignore = "subprocess fixture entry point, invoked by supervisor tests"]
fn fixture_process() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let path = PathBuf::from(std::env::var_os(DIRECTORY).unwrap());
    let pid_name = if mode == "helper" {
        "helper.pid"
    } else if mode == "unrelated" {
        "unrelated.pid"
    } else {
        "root.pid"
    };
    fs::write(path.join(pid_name), std::process::id().to_string()).unwrap();
    match mode.as_str() {
        "tree" | "orphan-output" | "orphan-input" | "background" => {
            let mut child = fixture_command("helper", &path);
            child.stdin(Stdio::inherit());
            if mode == "orphan-input" || mode == "background" {
                // std::Command can otherwise inherit these original handles
                // in addition to its new NUL standard handles. Close that
                // inheritance route so only stdin (or no stream) stays open.
                for handle in [
                    std::io::stdout().as_raw_handle(),
                    std::io::stderr().as_raw_handle(),
                ] {
                    assert_ne!(
                        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) },
                        0
                    );
                }
                child.stdout(Stdio::null()).stderr(Stdio::null());
            } else {
                child.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            }
            // Dropping Child intentionally leaves a helper for the production
            // job to own. The test observer retains a handle for failure cleanup.
            let _child = child.spawn().unwrap();
            let started = Instant::now();
            while !path.join("helper.pid").exists() {
                assert!(started.elapsed() < Duration::from_secs(2));
                thread::sleep(Duration::from_millis(1));
            }
            // Handshake with the test observer so it retains the exact handles
            // before a successful root return immediately closes the job.
            while !path.join("observed-root.pid").exists()
                || !path.join("observed-helper.pid").exists()
            {
                assert!(started.elapsed() < Duration::from_secs(2));
                thread::sleep(Duration::from_millis(1));
            }
            if mode == "tree" {
                thread::sleep(Duration::from_secs(60));
            } else {
                fs::write(path.join("root-exited"), "done").unwrap();
            }
        }
        "helper" => {
            thread::sleep(Duration::from_secs(5));
            fs::write(path.join("late-marker"), "helper survived").unwrap();
            thread::sleep(Duration::from_secs(55));
        }
        "unrelated" => thread::sleep(Duration::from_secs(60)),
        "flood-blocked-input" | "finite-flood" => {
            let buffer = [b'x'; 8192];
            let mut stdout = std::io::stdout().lock();
            let mut stderr = std::io::stderr().lock();
            for _ in 0..256 {
                stdout.write_all(&buffer).unwrap();
                stderr.write_all(&buffer).unwrap();
            }
            if mode == "flood-blocked-input" {
                loop {
                    stdout.write_all(&buffer).unwrap();
                    stderr.write_all(&buffer).unwrap();
                }
            }
            stdout.flush().unwrap();
            stderr.flush().unwrap();
            std::process::exit(37);
        }
        "exit259" => {
            eprintln!("fixture exit diagnostic");
            std::process::exit(259);
        }
        "echo" => {
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            let args: Vec<OsString> = std::env::args_os().collect();
            let arguments: Vec<_> = args
                .windows(2)
                .filter(|pair| pair[0] == "--skip")
                .map(|pair| pair[1].to_str().unwrap())
                .collect();
            let case_count = std::env::vars_os()
                .filter(|(name, _)| name.to_string_lossy().to_uppercase() == "ENOGRAM_CASE_Å")
                .count();
            let value = serde_json::json!({
                "length": input.len(), "checksum": checksum(&input), "arguments": arguments,
                "path": std::env::var("PATH").unwrap(), "comspec": std::env::var("COMSPEC").ok(),
                "caseValue": std::env::var("ENOGRAM_CASE_Å").unwrap(), "caseCount": case_count,
                "directory": std::env::current_dir().unwrap().to_str().unwrap(),
            });
            println!("ENOGRAM_ECHO={value}");
            std::io::stdout().flush().unwrap();
        }
        _ => panic!("unknown process fixture mode {mode}"),
    }
    std::process::exit(0);
}
