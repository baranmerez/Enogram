//! Stable Rust does not expose PROC_THREAD_ATTRIBUTE_JOB_LIST. Use CreateProcessW
//! with both JOB_LIST and HANDLE_LIST: ownership precedes execution, and only the
//! three standard handles are inherited. Never fall back to spawn-then-assign.
//!
//! Parent pipe endpoints use synchronous, nonblocking polling (PIPE_NOWAIT), not
//! pending/overlapped operations. Child endpoints retain blocking byte-stream
//! semantics. No worker, join, flush, or pipe operation can wait on a helper.

use super::{io_error, spawn_error, timed_out, DrainedStream};
use crate::{CapturedOutput, GitError, MAX_STDERR_BYTES};
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter};
use std::marker::PhantomData;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::ptr::{null, null_mut};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_BROKEN_PIPE, ERROR_NO_DATA, HANDLE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::{SetNamedPipeHandleState, PIPE_NOWAIT};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

pub(super) fn run(
    command: &mut Command,
    stdout_limit: usize,
    input: Option<&[u8]>,
    timeout: Duration,
    started: Instant,
) -> Result<CapturedOutput, GitError> {
    let (stdout_reader, stdout_writer) = io::pipe().map_err(spawn_error)?;
    let (stderr_reader, stderr_writer) = io::pipe().map_err(spawn_error)?;
    let mut stdout = Drain::new(stdout_reader, stdout_limit).map_err(spawn_error)?;
    let mut stderr = Drain::new(stderr_reader, MAX_STDERR_BYTES).map_err(spawn_error)?;
    let (stdin_child, mut stdin) = if input.is_some() {
        let (reader, writer) = io::pipe().map_err(spawn_error)?;
        set_nonblocking(&writer).map_err(spawn_error)?;
        (OwnedHandle::from(reader), Some(writer))
    } else {
        (
            OwnedHandle::from(File::open(r"\\.\NUL").map_err(spawn_error)?),
            None,
        )
    };
    let child_handles = [
        duplicate_inheritable(&stdin_child).map_err(spawn_error)?,
        duplicate_inheritable(&stdout_writer).map_err(spawn_error)?,
        duplicate_inheritable(&stderr_writer).map_err(spawn_error)?,
    ];
    #[cfg(test)]
    run_spawn_probe(&child_handles);
    let process = Process::spawn(command, &child_handles).map_err(spawn_error)?;
    // Parent copies of every child endpoint must close before testing for EOF.
    drop(child_handles);
    drop(stdin_child);
    drop(stdout_writer);
    drop(stderr_writer);

    let input = input.unwrap_or_default();
    let mut written = 0;
    let mut write_chunk = 8192usize;
    let mut input_error = None;
    let mut status = None;
    let status = loop {
        if started.elapsed() >= timeout {
            return Err(timed_out(timeout));
        }
        // Exactly one bounded operation per stream per turn. An output flood
        // cannot starve stderr, stdin, process observation, or the deadline.
        let mut progressed = stdout
            .poll()
            .map_err(|error| io_error("Could not read Git stdout", error))?;
        progressed |= stderr
            .poll()
            .map_err(|error| io_error("Could not read Git stderr", error))?;
        if written == input.len() {
            // Closing the writer delivers EOF after its buffered bytes. Never
            // FlushFileBuffers here: that waits for the child to consume input.
            stdin.take();
        } else if let Some(writer) = &stdin {
            // A nonblocking write can return zero if the request does not fit.
            // Adapt down to one byte without assuming a pipe buffer capacity;
            // retain an offset for partial writes instead of write_all.
            let end = input.len().min(written + write_chunk);
            match write_nowait(writer, &input[written..end]) {
                Ok(count) => {
                    written += count;
                    progressed |= count != 0;
                    if count == 0 && write_chunk > 1 {
                        write_chunk /= 2;
                        progressed = true;
                    } else if count != 0 {
                        write_chunk = (write_chunk * 2).min(8192);
                    }
                }
                Err(error) => {
                    input_error = Some(error);
                    stdin.take();
                }
            }
        }
        if status.is_none() {
            status = process
                .try_wait()
                .map_err(|error| io_error("Could not wait for Git", error))?;
        }
        if let Some(status) = status {
            if stdout.reader.is_none() && stderr.reader.is_none() && stdin.is_none() {
                break status;
            }
        }
        if !progressed {
            thread::sleep(Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())));
        }
    };
    if status.success() {
        if let Some(error) = input_error {
            return Err(io_error("Git did not consume its complete input", error));
        }
    }
    Ok(CapturedOutput {
        stdout: stdout.stream.bytes,
        stderr: stderr.stream.bytes,
        stdout_truncated: stdout.stream.truncated,
        stderr_truncated: stderr.stream.truncated,
        status,
    })
    // process owns the last (noninheritable) job handle. Closing it terminates
    // any remaining helpers on success, error, timeout, or panic. The parent
    // app and unrelated Git processes have never been assigned to this job.
}

struct Drain {
    reader: Option<PipeReader>,
    stream: DrainedStream,
}

impl Drain {
    fn new(reader: PipeReader, limit: usize) -> io::Result<Self> {
        set_nonblocking(&reader)?;
        Ok(Self {
            reader: Some(reader),
            stream: DrainedStream::new(limit),
        })
    }

    fn poll(&mut self) -> io::Result<bool> {
        let Some(reader) = &self.reader else {
            return Ok(false);
        };
        let mut buffer = [0u8; 8192];
        let mut count = 0;
        // SAFETY: the live handle is a nonblocking, synchronous pipe; the
        // writable buffer and count outlive the call. No I/O remains pending.
        let ok = unsafe {
            ReadFile(
                reader.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut count,
                null_mut(),
            )
        };
        if ok != 0 {
            self.stream.append(&buffer[..count as usize]);
            return Ok(count != 0);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error().map(|code| code as u32) {
            // Read::read on a std pipe also maps NO_DATA to EOF. Native
            // ReadFile is necessary to distinguish "empty now" from closed.
            Some(ERROR_NO_DATA) => Ok(false),
            Some(ERROR_BROKEN_PIPE) => {
                self.reader.take();
                Ok(true)
            }
            _ => Err(error),
        }
    }
}

fn write_nowait(writer: &PipeWriter, bytes: &[u8]) -> io::Result<usize> {
    let mut count = 0;
    // SAFETY: the live pipe is synchronous/nonblocking; bytes and the count
    // remain valid throughout the call. There is no pending I/O on return.
    let ok = unsafe {
        WriteFile(
            writer.as_raw_handle(),
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut count,
            null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(count as usize)
    }
}

fn set_nonblocking(pipe: &impl AsRawHandle) -> io::Result<()> {
    // SAFETY: std::io::pipe provides anonymous pipe handles. Windows explicitly
    // permits SetNamedPipeHandleState on them. Only our endpoint is changed.
    let ok = unsafe { SetNamedPipeHandleState(pipe.as_raw_handle(), &PIPE_NOWAIT, null(), null()) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn duplicate_inheritable(handle: &impl AsRawHandle) -> io::Result<OwnedHandle> {
    let mut duplicate = null_mut();
    // SAFETY: both process pseudo-handles refer to this process; the source
    // handle is borrowed and valid. Ownership of the new handle is transferred
    // to OwnedHandle exactly once. It is listed explicitly in HANDLE_LIST.
    let ok = unsafe {
        let current = GetCurrentProcess();
        DuplicateHandle(
            current,
            handle.as_raw_handle(),
            current,
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(duplicate) })
    }
}

#[cfg(test)]
type SpawnProbe = Box<dyn FnOnce(&[OwnedHandle; 3])>;

#[cfg(test)]
thread_local! {
    static SPAWN_PROBE: std::cell::RefCell<Option<SpawnProbe>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn with_spawn_probe<T>(
    probe: impl FnOnce(&[OwnedHandle; 3]) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct ClearProbe;

    impl Drop for ClearProbe {
        fn drop(&mut self) {
            SPAWN_PROBE.with(|slot| slot.borrow_mut().take());
        }
    }

    SPAWN_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(
            slot.is_none(),
            "a spawn probe is already installed on this thread"
        );
        *slot = Some(Box::new(probe));
    });
    let _clear = ClearProbe;
    action()
}

#[cfg(test)]
fn run_spawn_probe(handles: &[OwnedHandle; 3]) {
    let probe = SPAWN_PROBE.with(|slot| slot.borrow_mut().take());
    if let Some(probe) = probe {
        probe(handles);
    }
}

#[cfg(test)]
pub(super) fn spawn_unrelated(command: &Command) -> io::Result<OwnedHandle> {
    let program = wide_null(command.get_program())?;
    let mut command_line = command_line(command)?;
    let environment = environment_block(command)?;
    let current_dir = command
        .get_current_dir()
        .map(|path| wide_null(path.as_os_str()))
        .transpose()?;
    let mut startup = windows_sys::Win32::System::Threading::STARTUPINFOW::default();
    startup.cb = size_of::<windows_sys::Win32::System::Threading::STARTUPINFOW>() as u32;
    let mut info = PROCESS_INFORMATION::default();
    // The unrelated fixture has no streams and must not receive another test's
    // temporary inheritable endpoints. It deliberately belongs to no command job.
    // SAFETY: buffers are NUL-terminated and live throughout this synchronous call.
    let ok = unsafe {
        CreateProcessW(
            program.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            current_dir.as_ref().map_or(null(), |path| path.as_ptr()),
            &startup,
            &mut info,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful creation returns two owned handles; only the process
    // handle is retained so the test can prove liveness and clean up this fixture.
    let handle = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
    Ok(handle)
}

struct Process {
    handle: OwnedHandle,
    _job: OwnedHandle,
}

impl Process {
    fn spawn(command: &Command, child_handles: &[OwnedHandle; 3]) -> io::Result<Self> {
        let program = wide_null(command.get_program())?;
        let mut command_line = command_line(command)?;
        let environment = environment_block(command)?;
        let current_dir = command
            .get_current_dir()
            .map(|path| wide_null(path.as_os_str()))
            .transpose()?;
        let job = create_job()?;
        let jobs = [job.as_raw_handle()];
        let inherited = child_handles.each_ref().map(AsRawHandle::as_raw_handle);
        let attributes = Attributes::new(&jobs, &inherited)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = inherited[0];
        startup.StartupInfo.hStdOutput = inherited[1];
        startup.StartupInfo.hStdError = inherited[2];
        startup.lpAttributeList = attributes.as_ptr();
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: all UTF-16 buffers are NUL-terminated; the command line is
        // writable; attribute values and inheritable handles outlive the call.
        // JOB_LIST assigns the job before the initial thread can run. Failure
        // returns an error, with no unowned spawn or reduced-safety fallback.
        let ok = unsafe {
            CreateProcessW(
                program.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr().cast(),
                current_dir.as_ref().map_or(null(), |path| path.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful CreateProcessW returns two owned handles. The
        // primary thread is already running; we only need the process handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
        Ok(Self { handle, _job: job })
    }

    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: the owned process handle is valid. A zero timeout never waits.
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut exit_code = 0;
                // A signaled process has exited, even if its real exit code is
                // 259 (STILL_ACTIVE). Do not use the exit code as a liveness test.
                let ok = unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &mut exit_code) };
                if ok == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Some(ExitStatus::from_raw(exit_code)))
                }
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
}

fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes/name create an unnamed, noninheritable job.
    let raw = unsafe { CreateJobObjectW(null(), null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // Do not permit breakaway. The app never enters this job: only this command
    // and its descendants do. Dropping the only job handle cleans every path.
    let ok = unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(job)
    }
}

struct Attributes<'a> {
    // usize storage provides the alignment required by the opaque Windows list.
    storage: Vec<usize>,
    _values: PhantomData<&'a [HANDLE]>,
}

impl<'a> Attributes<'a> {
    fn new(jobs: &'a [HANDLE], inherited: &'a [HANDLE]) -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: the first call asks for the required size for two attributes.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let raw = storage.as_mut_ptr().cast();
        let ok = unsafe { InitializeProcThreadAttributeList(raw, 2, 0, &mut bytes) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let attributes = Self {
            storage,
            _values: PhantomData,
        };
        for (attribute, values) in [
            (PROC_THREAD_ATTRIBUTE_JOB_LIST, jobs),
            (PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherited),
        ] {
            // SAFETY: list is initialized; the borrowed slices are valid and
            // stay alive until this list is destroyed (including on errors).
            let ok = unsafe {
                UpdateProcThreadAttribute(
                    attributes.as_ptr(),
                    0,
                    attribute as usize,
                    values.as_ptr().cast(),
                    std::mem::size_of_val(values),
                    null_mut(),
                    null(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(attributes)
    }

    fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_ptr().cast_mut().cast()
    }
}

impl Drop for Attributes<'_> {
    fn drop(&mut self) {
        // SAFETY: storage still owns the initialized list and is freed after
        // DeleteProcThreadAttributeList; its borrowed values also remain alive.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn wide_null(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "embedded NUL"));
    }
    wide.push(0);
    Ok(wide)
}

fn command_line(command: &Command) -> io::Result<Vec<u16>> {
    let mut line = Vec::new();
    for argument in std::iter::once(command.get_program()).chain(command.get_args()) {
        let wide = wide_null(argument)?;
        if !line.is_empty() {
            line.push(b' ' as u16);
        }
        line.push(b'"' as u16);
        let mut slashes = 0;
        for &unit in &wide[..wide.len() - 1] {
            if unit == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            // CRT quoting: double backslashes before a quote, then escape the
            // quote; otherwise preserve every backslash literally.
            line.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            if unit == b'"' as u16 {
                line.extend(std::iter::repeat_n(b'\\' as u16, slashes + 1));
            }
            line.push(unit);
            slashes = 0;
        }
        line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        line.push(b'"' as u16);
    }
    line.push(0);
    if line.len() > 32767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows command line exceeds 32767 UTF-16 units",
        ));
    }
    Ok(line)
}

fn environment_block(command: &Command) -> io::Result<Vec<u16>> {
    // This private runner receives only program/args/cwd/env/env_remove from
    // run_git_with_input, never env_clear, raw_arg, or custom spawn attributes.
    let inherited = std::env::vars_os().map(|(name, value)| (name, Some(value)));
    let changed = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)));
    let mut entries: Vec<(Vec<u16>, Vec<u16>)> = Vec::new();
    for (name, value) in inherited.chain(changed) {
        let mut name = wide_null(&name)?;
        name.pop();
        if name.is_empty() || name.len() > i32::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid environment name",
            ));
        }
        // Replace/remove keys using the OS's ordinal case-insensitive ordering
        // (including Unicode), while retaining their original UTF-16 spelling.
        entries.retain(|(existing, _)| compare_names(existing, &name) != Ordering::Equal);
        if let Some(value) = value {
            let value = wide_null(&value)?;
            entries.push((name, value));
        }
    }
    entries.sort_unstable_by(|(left, _), (right, _)| compare_names(left, right));
    let mut block = Vec::new();
    for (name, value) in entries {
        block.extend_from_slice(&name);
        block.push(b'=' as u16);
        block.extend(value); // value includes the entry's terminating NUL
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn compare_names(left: &[u16], right: &[u16]) -> Ordering {
    // SAFETY: both names are validated, nonempty UTF-16 slices whose lengths
    // fit i32. Ordinal comparison supports unpaired surrogates (OsStr values).
    let comparison = unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        )
    };
    debug_assert!(comparison != 0);
    comparison.cmp(&CSTR_EQUAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn command_line_preserves_empty_arguments_quotes_and_utf16() {
        let mut command = Command::new(r"C:\Program Files\Git\git.exe");
        command.args(["", r"tail\", "quoted \"word\""]);
        let line = command_line(&command).unwrap();
        assert_eq!(
            String::from_utf16(&line[..line.len() - 1]).unwrap(),
            r#""C:\Program Files\Git\git.exe" "" "tail\\" "quoted \"word\"""#
        );
        let mut command = Command::new("git.exe");
        command.arg(OsString::from_wide(&[0xd800, b'a' as u16]));
        let line = command_line(&command).unwrap();
        assert!(line.windows(2).any(|units| units == [0xd800, b'a' as u16]));
        command.arg("embedded\0nul");
        assert_eq!(
            command_line(&command).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
