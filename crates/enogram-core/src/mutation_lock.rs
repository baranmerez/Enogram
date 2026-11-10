//! Coordinate cooperating desktop and MCP writers within one Windows session.
//! External Git and older Enogram builds do not take this lock. It complements
//! snapshot checks and Git's own locks; it does not make a write transactional.

use crate::{GitError, GitErrorKind};
use std::marker::PhantomData;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

const MUTEX_NAME: &str = "Local\\Enogram.GitMutation.v1";

pub(super) struct SharedWriteGuard {
    handle: OwnedHandle,
    // A Windows mutex belongs to the acquiring thread, so the guard cannot move
    // to another thread even though OwnedHandle itself is Send + Sync.
    _thread_bound: PhantomData<Rc<()>>,
}

pub(super) fn acquire() -> Result<SharedWriteGuard, GitError> {
    acquire_named(MUTEX_NAME)
}

fn open_named(name: &str) -> Result<OwnedHandle, GitError> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // Default security comes from the current Windows token. A null attributes
    // pointer makes the handle non-inheritable; Git helpers must not retain it.
    let handle = unsafe { CreateMutexW(null(), 0, wide.as_ptr()) };
    if handle.is_null() {
        return Err(GitError::new(
            GitErrorKind::Busy,
            "Cannot access Enogram's shared Windows write guard. No Git write was started.",
        ));
    }
    // SAFETY: successful CreateMutexW returns one owned kernel handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn acquire_named(name: &str) -> Result<SharedWriteGuard, GitError> {
    let handle = open_named(name)?;
    // Fail immediately instead of queueing an operation against an old snapshot.
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        WAIT_OBJECT_0 => Ok(SharedWriteGuard { handle, _thread_bound: PhantomData }),
        WAIT_ABANDONED => {
            // An abandoned wait grants ownership. Release it before returning;
            // the caller must inspect possible effects of the interrupted write.
            drop(SharedWriteGuard { handle, _thread_bound: PhantomData });
            Err(GitError::new(
                GitErrorKind::StaleState,
                "An Enogram writer was interrupted. Refresh the repository before starting another write; the previous operation may have changed it.",
            ))
        }
        WAIT_TIMEOUT => Err(GitError::new(
            GitErrorKind::Busy,
            "Another Enogram desktop or MCP write is still running. Inspect the repository after it finishes before starting another write.",
        )),
        _ => Err(GitError::new(
            GitErrorKind::Busy,
            "Could not acquire Enogram's shared Windows write guard. No Git write was started.",
        )),
    }
}

impl Drop for SharedWriteGuard {
    fn drop(&mut self) {
        // SAFETY: this guard owns the mutex on this thread, and is neither Send
        // nor Sync. OwnedHandle closes the handle after ownership is released.
        unsafe { ReleaseMutex(self.handle.as_raw_handle()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    const FIXTURE_TEST: &str = "mutation_lock::tests::lock_fixture_process";

    struct Fixture {
        path: PathBuf,
        name: String,
        worker: Option<thread::JoinHandle<Result<crate::CapturedOutput, GitError>>>,
    }

    impl Fixture {
        fn start(mode: &str) -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let unique = format!("enogram-write-lock-{}-{id}", std::process::id());
            let path = std::env::temp_dir().join(&unique);
            fs::create_dir(&path).unwrap();
            let name = format!("Local\\{unique}");
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--ignored", "--exact", FIXTURE_TEST, "--nocapture"])
                .env("ENOGRAM_LOCK_FIXTURE_NAME", &name)
                .env("ENOGRAM_LOCK_FIXTURE_PATH", &path)
                .env("ENOGRAM_LOCK_FIXTURE_MODE", mode);
            // The production runner restricts inherited handles. A broad std
            // spawn here could steal another parallel fixture's pipe endpoints.
            let worker = thread::spawn(move || {
                crate::git_process::run(&mut command, 4096, false, None, Duration::from_secs(5))
            });
            let fixture = Self {
                path,
                name,
                worker: Some(worker),
            };
            let started = Instant::now();
            while !fixture.path.join("ready").exists() {
                assert!(
                    started.elapsed() < Duration::from_secs(4),
                    "mutex fixture did not become ready"
                );
                thread::sleep(Duration::from_millis(2));
            }
            fixture
        }

        fn finish(&mut self) {
            fs::write(self.path.join("finish"), b"finish").unwrap();
            let output = self.worker.take().unwrap().join().unwrap().unwrap();
            assert!(
                output.status.success(),
                "fixture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::write(self.path.join("finish"), b"finish");
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn cross_process_writer_is_busy_then_released() {
        let mut fixture = Fixture::start("release");
        let error = acquire_named(&fixture.name)
            .err()
            .expect("other process owns mutex");
        assert_eq!(error.kind, GitErrorKind::Busy);
        fixture.finish();
        let guard = acquire_named(&fixture.name).expect("mutex released by other process");
        let mut flags = 0;
        assert_ne!(
            unsafe {
                windows_sys::Win32::Foundation::GetHandleInformation(
                    guard.handle.as_raw_handle(),
                    &mut flags,
                )
            },
            0
        );
        assert_eq!(
            flags & windows_sys::Win32::Foundation::HANDLE_FLAG_INHERIT,
            0
        );
    }

    #[test]
    fn abandoned_writer_requires_refresh_then_releases_ownership() {
        let mut fixture = Fixture::start("abandon");
        // Keep the object alive after the child exits so Windows can report its
        // abandoned state instead of destroying the last handle with the child.
        let _keep_alive = open_named(&fixture.name).unwrap();
        fixture.finish();
        let error = acquire_named(&fixture.name)
            .err()
            .expect("abandoned owner rejected");
        assert_eq!(error.kind, GitErrorKind::StaleState);
        let _guard = acquire_named(&fixture.name).expect("abandoned guard released");
    }

    #[test]
    #[ignore = "entry point only for the bounded mutex fixture subprocess"]
    fn lock_fixture_process() {
        let name = std::env::var("ENOGRAM_LOCK_FIXTURE_NAME").unwrap();
        let path = PathBuf::from(std::env::var_os("ENOGRAM_LOCK_FIXTURE_PATH").unwrap());
        let guard = acquire_named(&name).unwrap();
        fs::write(path.join("ready"), b"ready").unwrap();
        let started = Instant::now();
        while !path.join("finish").exists() {
            assert!(started.elapsed() < Duration::from_secs(4));
            thread::sleep(Duration::from_millis(2));
        }
        if std::env::var("ENOGRAM_LOCK_FIXTURE_MODE").unwrap() == "abandon" {
            std::mem::forget(guard);
            std::process::exit(0);
        }
    }
}
