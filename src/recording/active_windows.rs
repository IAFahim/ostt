use crate::windows_ipc::{user_sid, wide, UserSecurity};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        CreateEventW, OpenEventW, OpenProcess, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        PROCESS_SYNCHRONIZE, SYNCHRONIZATION_SYNCHRONIZE,
    },
};

fn event_name(pid: u32) -> io::Result<Vec<u16>> {
    Ok(wide(&format!(
        "Local\\ostt-recording-{}-{pid}",
        user_sid()?
    )))
}

pub(super) struct SignalGuard {
    worker: Option<thread::JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn register_transcription_signal(term: Arc<AtomicBool>) -> anyhow::Result<SignalGuard> {
    let security = UserSecurity::new()?;
    let attrs = security.attributes();
    let name = event_name(std::process::id())?;
    let handle = unsafe { CreateEventW(&attrs, 0, 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    // Transfer sole ownership of the handle to the waiter thread.
    let raw = handle as usize;
    let worker = thread::Builder::new()
        .name("recording-toggle".into())
        .spawn(move || {
            let handle = raw as HANDLE;
            while !worker_stopped.load(Ordering::SeqCst) {
                let result = unsafe { WaitForSingleObject(handle, 100) };
                if result == WAIT_OBJECT_0 {
                    term.store(true, Ordering::SeqCst);
                    break;
                }
                if result != WAIT_TIMEOUT {
                    tracing::error!(
                        "recording toggle wait failed: {}",
                        io::Error::last_os_error()
                    );
                    break;
                }
            }
            unsafe {
                CloseHandle(handle);
            }
        });
    match worker {
        Ok(worker) => Ok(SignalGuard {
            worker: Some(worker),
            stopped,
        }),
        Err(error) => {
            unsafe {
                CloseHandle(handle);
            }
            Err(error.into())
        }
    }
}

pub(crate) fn signal_running_recorder(pid: u32) -> anyhow::Result<()> {
    let name = event_name(pid)?;
    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    let result = unsafe { SetEvent(handle) };
    let error = io::Error::last_os_error();
    unsafe {
        CloseHandle(handle);
    }
    if result == 0 {
        return Err(error.into());
    }
    Ok(())
}

pub(super) fn process_exists(pid: u32) -> bool {
    // An alive PID alone can belong to another process after reuse; require its recorder event.
    let Ok(name) = event_name(pid) else {
        return false;
    };
    unsafe {
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if process.is_null() {
            return false;
        }
        let alive = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
        CloseHandle(process);
        let event = OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr());
        let recording = !event.is_null();
        if recording {
            CloseHandle(event);
        }
        alive && recording
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn toggle_stops_recording_and_guard_releases_event() {
        // The test process has its own PID: no recorder files or user state are touched.
        let pid = std::process::id();
        let term = Arc::new(AtomicBool::new(false));
        let guard = register_transcription_signal(Arc::clone(&term)).unwrap();
        assert!(
            process_exists(pid),
            "an active recording must be discoverable"
        );
        signal_running_recorder(pid).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !term.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            term.load(Ordering::SeqCst),
            "the second invocation must request recording stop"
        );
        drop(guard);
        assert!(
            !process_exists(pid),
            "a live PID must not count as a recording after cleanup"
        );
        assert!(signal_running_recorder(pid).is_err());

        let term = Arc::new(AtomicBool::new(false));
        drop(register_transcription_signal(Arc::clone(&term)).unwrap());
        assert!(
            !term.load(Ordering::SeqCst),
            "dropping the guard must not request transcription"
        );
        assert!(!process_exists(pid));
    }
}
