//! SSH host-key trust, UI side. The engine asks a [`HostKeyConfirm`] callback on a WORKER thread when a unit shows a key it
//! has never seen; nothing (no password, no key signature, no command) leaves this computer until it answers. The bridge
//! below turns that blocking call into a message for the UI thread (a dialog) and blocks the worker until the user answers.
//! No answer is a refusal: a closed dialog, a vanished window and a timeout all mean "do not trust".
//!
//! Pure and unit-tested (no GPUI types).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};
use synofurb_core::hostkey::{HostKeyConfirm, HostKeyError, HostKeyPrompt};

/// How long the question may stay open: the engine's own limit (`timing::HOST_KEY_ANSWER_TIMEOUT_SECS`), not a second number.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(synofurb_core::timing::HOST_KEY_ANSWER_TIMEOUT_SECS);

/// What the dialog may show about the question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustState {
    /// The worker is waiting; `remaining_s` seconds are left.
    Waiting { remaining_s: u64 },
    /// The connection attempt has ended (timeout, cancel or failure): a late Trust would do nothing, so it is not offered.
    Over,
}

/// A cheap handle the dialog keeps to see whether the worker still waits for the answer.
#[derive(Clone)]
pub struct TrustWatch {
    ended: Arc<AtomicBool>,
    started: Instant,
    timeout: Duration,
}

impl TrustWatch {
    pub fn state(&self, now: Instant) -> TrustState {
        let elapsed = now.saturating_duration_since(self.started);
        if self.ended.load(Ordering::SeqCst) || elapsed >= self.timeout {
            TrustState::Over
        } else {
            TrustState::Waiting { remaining_s: (self.timeout - elapsed).as_secs() + 1 }
        }
    }
}

/// `2:41`.
pub fn mmss(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// A first-contact question for the UI. Answer with [`TrustAsk::answer`]; dropping it unanswered is a refusal.
pub struct TrustAsk {
    pub prompt: HostKeyPrompt,
    reply: Sender<bool>,
    watch: TrustWatch,
}

impl TrustAsk {
    /// A question nobody will receive an answer to (the demo's dialogs).
    pub fn demo(prompt: HostKeyPrompt) -> TrustAsk {
        let (reply, _rx) = channel::<bool>();
        TrustAsk { prompt, reply, watch: TrustWatch { ended: Arc::new(AtomicBool::new(false)), started: Instant::now(), timeout: ANSWER_TIMEOUT } }
    }

    pub fn watch(&self) -> TrustWatch {
        self.watch.clone()
    }

    /// Hand the answer to the waiting worker. `false` when nobody is waiting any more (the attempt ended first): nothing was
    /// trusted, nothing was pinned.
    pub fn answer(self, trust: bool) -> bool {
        if self.watch.state(Instant::now()) == TrustState::Over {
            return false;
        }
        self.reply.send(trust).is_ok()
    }
}

/// The confirm callback for `HostKeyPolicy::tofu_with`: `deliver` hands the question to the UI (it returns `false` when the UI is gone).
pub fn bridge(deliver: impl Fn(TrustAsk) -> bool + Send + Sync + 'static, timeout: Duration) -> HostKeyConfirm {
    Arc::new(move |prompt: &HostKeyPrompt| {
        let (reply, rx) = channel::<bool>();
        let ended = Arc::new(AtomicBool::new(false));
        let watch = TrustWatch { ended: ended.clone(), started: Instant::now(), timeout };
        if !deliver(TrustAsk { prompt: prompt.clone(), reply, watch }) {
            return false;
        }
        let answer = rx.recv_timeout(timeout).unwrap_or(false);
        ended.store(true, Ordering::SeqCst);
        answer
    })
}

/// `SHA256:` + the 43 characters, split for reading and comparing: `["SHA256:", first 22, rest]`.
pub fn fingerprint_lines(fp: &str) -> [String; 3] {
    let body = fp.strip_prefix("SHA256:").unwrap_or(fp);
    let cut = body.char_indices().nth(22).map(|(i, _)| i).unwrap_or(body.len());
    ["SHA256:".to_string(), body[..cut].to_string(), body[cut..].to_string()]
}

/// A changed key found in a connection error: `(host, port, pinned, presented)`.
pub fn changed_key(e: &anyhow::Error) -> Option<(String, u16, String, String)> {
    match e.downcast_ref::<HostKeyError>() {
        Some(HostKeyError::Changed { host, port, pinned, presented, .. }) => Some((host.clone(), *port, pinned.clone(), presented.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const FP: &str = "SHA256:q3Zb1c8mPvX0dT7nKfL2yHrW9sUeJ5aG4oB6iNxVwYc";

    fn prompt() -> HostKeyPrompt {
        HostKeyPrompt { host: "192.168.1.20".into(), port: 22, fingerprint: FP.into() }
    }

    #[test]
    fn the_worker_blocks_until_the_user_answers() {
        for want in [true, false] {
            let slot: Arc<Mutex<Option<TrustAsk>>> = Arc::new(Mutex::new(None));
            let s2 = slot.clone();
            let confirm = bridge(
                move |a| {
                    *s2.lock().unwrap() = Some(a);
                    true
                },
                Duration::from_secs(5),
            );
            let h = std::thread::spawn(move || confirm(&prompt()));
            // the "UI" picks the question up and answers it
            let ask = loop {
                if let Some(a) = slot.lock().unwrap().take() {
                    break a;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            assert_eq!(ask.prompt.fingerprint, FP);
            ask.answer(want);
            assert_eq!(h.join().unwrap(), want);
        }
    }

    #[test]
    fn a_late_trust_after_the_connection_ended_is_not_delivered() {
        let held: Arc<Mutex<Vec<TrustAsk>>> = Arc::new(Mutex::new(vec![]));
        let h2 = held.clone();
        let confirm = bridge(
            move |a| {
                h2.lock().unwrap().push(a);
                true
            },
            Duration::from_millis(80),
        );
        let t = std::thread::spawn(move || confirm(&prompt()));
        let ask = loop {
            if let Some(a) = held.lock().unwrap().pop() {
                break a;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let watch = ask.watch();
        assert!(matches!(watch.state(Instant::now()), TrustState::Waiting { remaining_s } if remaining_s >= 1));
        assert!(!t.join().unwrap(), "no answer in time: refused");
        assert_eq!(watch.state(Instant::now()), TrustState::Over, "the dialog is told the attempt is over");
        assert!(!ask.answer(true), "a late Trust claims nothing");
    }

    #[test]
    fn the_remaining_time_counts_down_and_ends_at_the_engine_limit() {
        assert_eq!(ANSWER_TIMEOUT.as_secs(), 180);
        assert_eq!((mmss(180), mmss(161), mmss(5)), ("3:00".into(), "2:41".into(), "0:05".into()));
        let a = TrustAsk::demo(prompt());
        let w = a.watch();
        let t0 = Instant::now();
        let TrustState::Waiting { remaining_s: r0 } = w.state(t0) else { panic!() };
        assert!((180..=181).contains(&r0));
        assert_eq!(w.state(t0 + Duration::from_secs(181)), TrustState::Over);
    }

    #[test]
    fn no_answer_is_a_refusal() {
        // timeout
        let keep: Arc<Mutex<Vec<TrustAsk>>> = Arc::new(Mutex::new(vec![]));
        let k2 = keep.clone();
        let slow = bridge(
            move |a| {
                k2.lock().unwrap().push(a); // held, never answered
                true
            },
            Duration::from_millis(60),
        );
        assert!(!slow(&prompt()));
        // dialog closed: the question was dropped
        let dropped = bridge(
            |a| {
                drop(a);
                true
            },
            Duration::from_secs(5),
        );
        assert!(!dropped(&prompt()));
        // the UI is gone
        let gone = bridge(|_| false, Duration::from_secs(5));
        assert!(!gone(&prompt()));
    }

    #[test]
    fn the_engine_policy_pins_only_after_a_yes() {
        use synofurb_core::hostkey::{HostKeyDecision, HostKeyPolicy, KnownHosts};
        let path = std::env::temp_dir().join(format!("fv-trust-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = KnownHosts::at(&path);
        let no = HostKeyPolicy::tofu_with(
            store.clone(),
            bridge(
                |a| {
                    a.answer(false);
                    true
                },
                Duration::from_secs(5),
            ),
        );
        assert!(matches!(no.check("nas", 22, FP).unwrap_err(), HostKeyError::NotTrusted { .. }));
        assert!(store.get("nas", 22).unwrap().is_none(), "a refusal pins nothing");
        let yes = HostKeyPolicy::tofu_with(
            store.clone(),
            bridge(
                |a| {
                    a.answer(true);
                    true
                },
                Duration::from_secs(5),
            ),
        );
        assert_eq!(yes.check("nas", 22, FP).unwrap(), HostKeyDecision::AcceptedAndPinned);
        assert_eq!(store.get("nas", 22).unwrap().unwrap().fingerprint, FP);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fingerprints_split_for_comparing() {
        let l = fingerprint_lines(FP);
        assert_eq!(l[0], "SHA256:");
        assert_eq!(l[1].len(), 22);
        assert_eq!(format!("{}{}{}", l[0], l[1], l[2]), FP);
        assert_eq!(fingerprint_lines("short")[2], "");
    }

    #[test]
    fn a_changed_key_is_found_in_the_error_chain() {
        let e = anyhow::Error::new(HostKeyError::Changed {
            host: "h".into(),
            port: 22,
            pinned: "SHA256:a".into(),
            presented: "SHA256:b".into(),
            port_arg: String::new(),
        })
        .context("connecting");
        assert_eq!(changed_key(&e), Some(("h".into(), 22, "SHA256:a".into(), "SHA256:b".into())));
        assert_eq!(changed_key(&anyhow::anyhow!("other")), None);
    }
}
