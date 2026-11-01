//! Runs a long job (install, boot manager, recovery, downloads) on a worker thread, renders its event stream, and handles Ctrl-C:
//! before the point of no return the first Ctrl-C asks the job to stop at the next safe point; after it, Ctrl-C only says that it is too
//! late. A second Ctrl-C stops *watching* (the job on the NAS carries on) and returns control.

use super::exit::CliError;
use super::facts::config_dir;
use super::out::{Out, Reporter};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, TryRecvError, channel};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use synofurb_core::logs::{Event, Logger};

static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);
static HANDLER: OnceLock<bool> = OnceLock::new();

/// Install the Ctrl-C / SIGTERM / SIGHUP handler (once). Without it the default action kills the process.
pub fn install_signal_handler() -> bool {
    *HANDLER.get_or_init(|| {
        ctrlc::set_handler(|| {
            INTERRUPTS.fetch_add(1, Ordering::SeqCst);
        })
        .is_ok()
    })
}

pub fn interrupts() -> usize {
    INTERRUPTS.load(Ordering::SeqCst)
}

/// Progress callback the core expects, feeding the event stream.
pub fn progress_fn(logger: &Logger) -> impl FnMut(&str, f32) + use<> {
    let lg = logger.clone();
    move |what, fraction| lg.emit(Event::Progress { what: what.to_string(), fraction })
}

pub struct RunReport<T> {
    pub result: Result<T>,
    pub past_ponr: bool,
    /// The user stopped watching (second Ctrl-C); the job may still be running.
    pub detached: bool,
    pub cancel_requested: bool,
    pub log_path: Option<PathBuf>,
    pub installer_failed: Option<String>,
}

impl<T> RunReport<T> {
    /// The error to exit with: 6 after the point of no return, 7 before it.
    pub fn failure(&self) -> Option<CliError> {
        let e = self.result.as_ref().err()?;
        let mut message = format!("{e:#}");
        if self.past_ponr {
            message.push_str(". The point of no return had passed: the disks were changed. Look at the log, then see docs/RECOVERY.md");
        } else if self.cancel_requested || self.detached {
            message.push_str(". No disk was changed");
        }
        if let Some(p) = &self.log_path {
            message.push_str(&format!(". Log: {}", p.display()));
        }
        Some(CliError::InstallFailed { after_ponr: self.past_ponr, message })
    }
}

/// Run `work` on a thread. `secrets` are redacted from every log line. The closure receives the logger and the cancel flag.
pub fn run_job<T, F>(out: &Out, secrets: &[String], work: F) -> RunReport<T>
where
    T: Send + 'static,
    F: FnOnce(&Logger, Arc<AtomicBool>) -> Result<T> + Send + 'static,
{
    if !install_signal_handler() {
        out.warn("could not install the Ctrl-C handler; Ctrl-C will end the program immediately instead of cancelling cleanly");
    }
    let baseline = interrupts();
    let (tx, rx) = channel::<Event>();
    let logger = match Logger::to_dir(&config_dir().join("logs"), Some(tx.clone())) {
        Ok(l) => l,
        Err(e) => {
            out.warn(&format!("cannot write a log file ({e}); continuing without one"));
            Logger::memory(Some(tx.clone()))
        }
    };
    for s in secrets {
        logger.add_secret(s);
    }
    let log_path = logger.path().map(|p| p.to_path_buf());
    if let Some(p) = &log_path {
        out.detail(&format!("log file: {}", p.display()));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = channel::<Result<T>>();
    {
        let (lg, c) = (logger.clone(), cancel.clone());
        std::thread::spawn(move || {
            let r = work(&lg, c);
            let _ = done_tx.send(r);
        });
    }
    drop(tx);

    let mut rep = Reporter::new(out);
    let (mut handled, mut cancel_requested) = (0usize, false);
    let mut result: Option<Result<T>> = None;
    let mut detached = false;
    while result.is_none() {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(e) => rep.handle(&e),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        }
        match done_rx.try_recv() {
            Ok(r) => {
                result = Some(r);
                continue;
            }
            Err(TryRecvError::Disconnected) => {
                result = Some(Err(anyhow!("internal error: the worker thread ended without a result")));
                continue;
            }
            Err(TryRecvError::Empty) => {}
        }
        let n = interrupts().saturating_sub(baseline);
        if n > handled {
            handled = n;
            rep.finish();
            if !rep.past_ponr {
                if n == 1 {
                    cancel_requested = true;
                    cancel.store(true, Ordering::Relaxed);
                    out.warn("Ctrl-C: cancelling. No disk has been changed so far; the job stops at the next safe point. Press Ctrl-C again to stop waiting.");
                } else {
                    out.warn("Stopped waiting for the job (it may not have finished cleanly; no point of no return was seen). Check the log and the NAS console.");
                    detached = true;
                    result = Some(Err(anyhow!("interrupted by the user")));
                }
            } else if n == 1 {
                out.warn("Too late to cancel: the point of no return has passed, the disks have been changed and the installer on the NAS keeps running by itself. Press Ctrl-C again to stop watching it.");
            } else {
                out.warn("Stopped watching. The installer on the NAS carries on and will reboot the unit when it is done.");
                detached = true;
                result = Some(Err(anyhow!("stopped watching after the point of no return (the installer on the NAS keeps running)")));
            }
        }
    }
    for e in rx.try_iter() {
        rep.handle(&e);
    }
    rep.finish();
    let past_ponr = rep.past_ponr;
    let installer_failed = rep.installer_failed.clone();
    let result = result.unwrap_or_else(|| Err(anyhow!("no result")));
    RunReport { result, past_ponr, detached, cancel_requested, log_path, installer_failed }
}

/// NDJSON failure event for any error (text mode prints the message on stderr in `main`).
pub fn failure_event(out: &Out, e: &CliError) {
    out.event(json!({
        "event": "failure",
        "kind": e.kind(),
        "message": e.to_string(),
        "exit_code": e.code(),
        "after_point_of_no_return": matches!(e, CliError::InstallFailed { after_ponr: true, .. }),
    }));
}
/// A logger without a file that prints to stderr as the verbosity allows (for the short read-only commands).
pub fn console_logger(out: &Out) -> Logger {
    use super::out::{level_name, log_visible};
    let (tx, rx) = channel::<Event>();
    let (verbose, quiet, color) = (out.verbose, out.quiet, out.color_err);
    std::thread::spawn(move || {
        for e in rx {
            if let Event::Log(l) = e
                && !quiet
                && verbose >= 1
                && log_visible(&l, verbose)
            {
                let lvl = super::out::paint(color, super::out::Style::Dim, &level_name(l.level).to_uppercase());
                super::out::err_raw(&format!("{} {lvl:5} [{}] {}\n", l.ts, l.source.tag(), l.msg));
            }
        }
    });
    Logger::memory(Some(tx))
}

impl<T> RunReport<T> {
    /// For jobs that are not installs: any failure is a plain error (exit 1), with the log path appended.
    pub fn plain_failure(&self) -> Option<CliError> {
        let e = self.result.as_ref().err()?;
        let mut m = format!("{e:#}");
        if let Some(p) = &self.log_path {
            m.push_str(&format!(" (log: {})", p.display()));
        }
        Some(CliError::Other(anyhow!(m)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    /// Interrupt counting is process-wide: the tests that raise it must not overlap with other jobs.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn raise(n: usize, after: Duration) {
        std::thread::spawn(move || {
            std::thread::sleep(after);
            for _ in 0..n {
                INTERRUPTS.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(250));
            }
        });
    }

    #[test]
    fn a_job_returns_its_value_and_streams_events() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let out = Out::plain();
        let rep = run_job(&out, &["s3cr3t-value".to_string()], |lg, _c| {
            lg.info("hello s3cr3t-value");
            lg.emit(Event::PointOfNoReturn);
            Ok(42)
        });
        assert_eq!(*rep.result.as_ref().unwrap(), 42);
        assert!(rep.past_ponr, "the point of no return is tracked from the event stream");
        assert!(rep.failure().is_none());
        if let Some(p) = &rep.log_path {
            let text = std::fs::read_to_string(p).unwrap();
            assert!(!text.contains("s3cr3t-value"), "secrets are redacted in the log file");
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn failures_map_to_the_exit_codes() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let out = Out::plain();
        let rep: RunReport<()> = run_job(&out, &[], |_, _| Err(anyhow!("boom")));
        let e = rep.failure().unwrap();
        assert_eq!(e.code(), 7);
        assert!(e.to_string().contains("boom"));
        let rep: RunReport<()> = run_job(&out, &[], |lg, _| {
            lg.emit(Event::PointOfNoReturn);
            Err(anyhow!("disk exploded"))
        });
        let e = rep.failure().unwrap();
        assert_eq!(e.code(), 6);
        assert!(e.to_string().contains("point of no return"));
        for r in [&rep] {
            if let Some(p) = &r.log_path {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    #[test]
    fn a_panicking_job_is_reported_not_hung() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let out = Out::plain();
        let rep: RunReport<()> = run_job(&out, &[], |_, _| panic!("worker bug"));
        assert!(rep.result.is_err());
        assert_eq!(rep.failure().unwrap().code(), 7);
        if let Some(p) = &rep.log_path {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn ctrl_c_before_the_point_of_no_return_cancels_and_exits_seven() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let out = Out::plain();
        raise(1, Duration::from_millis(300));
        let rep: RunReport<()> = run_job(&out, &[], |_, cancel| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(10) {
                if cancel.load(Ordering::Relaxed) {
                    anyhow::bail!("cancelled by the user");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        });
        assert!(rep.cancel_requested && !rep.past_ponr && !rep.detached);
        let e = rep.failure().unwrap();
        assert_eq!(e.code(), 7);
        assert!(e.to_string().contains("No disk was changed"), "{e}");
        if let Some(p) = &rep.log_path {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn ctrl_c_after_the_point_of_no_return_cannot_cancel_only_stop_watching() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let out = Out::plain();
        raise(2, Duration::from_millis(500));
        let started = Instant::now();
        let rep: RunReport<()> = run_job(&out, &[], |lg, cancel| {
            lg.emit(Event::PointOfNoReturn);
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(4) {
                assert!(!cancel.load(Ordering::Relaxed), "the cancel flag must never be set after the point of no return");
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        });
        assert!(rep.past_ponr && rep.detached && !rep.cancel_requested);
        assert!(started.elapsed() < Duration::from_secs(3), "the second Ctrl-C returns control without waiting for the job");
        let e = rep.failure().unwrap();
        assert_eq!(e.code(), 6);
        assert!(e.to_string().contains("point of no return"), "{e}");
        if let Some(p) = &rep.log_path {
            let _ = std::fs::remove_file(p);
        }
    }
}
