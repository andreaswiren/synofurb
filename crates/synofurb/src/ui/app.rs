//! The root view: all UI state, the message pump from the worker threads, and the actions screens trigger.
//! Rendering lives in `render.rs` (chrome) and `screens/` (one module per screen).

use super::copy;
use super::demo::{self, DemoUnit, Script};
use super::screens::inventory::InventoryTable;
use super::settings::{UiSettings, now_unix};
use super::state::*;
use super::theme::{self, ThemeChoice};
use super::worker::{self, Msg, Tx};
use crate::cli::config_dir;
use futures::StreamExt;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::table::{TableEvent, TableState};
use gpui_kit::component::{IndexPath, WindowExt};
use gpui_kit::{AppContext, ClipboardItem, Context, Entity, FocusHandle, PathPromptOptions, SharedString, Subscription, Window};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use synofurb_core::compat::{CompatDb, Status};
use synofurb_core::config::{InstallConfig, KernelChannel, SshKeypair, generate_password, generate_ssh_keypair, hash_password};
use synofurb_core::discovery::{Found, Inventory, ScanOptions};
use synofurb_core::logs::Event;
use synofurb_core::model::DeviceFacts;
use synofurb_core::plan::{DataRaid, Plan, PlanRequest, StorageMode, build_plan, default_system_copies};
use synofurb_core::platform::{self, Support};
use synofurb_core::remote::Auth;
use synofurb_core::session::{BootMethod, InstallJob, Outcome, SshTarget, confirm_phrase};
use synofurb_core::updater::{Channel, Progress, UpdateInfo, VerifiedDownload};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    Install,
    BootManager,
    Recover,
}

/// Waiting for the unit's DSM to answer after a failed install (probed every few seconds, `recovery::wait_for_dsm`).
#[derive(Clone, Debug, Default)]
pub enum DsmWaitState {
    #[default]
    Idle,
    Waiting {
        secs: u64,
        last: Option<synofurb_core::recovery::ProbeResult>,
    },
    Done {
        last: synofurb_core::recovery::ProbeResult,
        timed_out: bool,
        secs: u64,
    },
}

/// A background job with a result.
pub enum Job<T> {
    Idle,
    Running,
    Done(Result<T, String>),
}

impl<T> Job<T> {
    pub fn running(&self) -> bool {
        matches!(self, Job::Running)
    }
}

#[derive(Default)]
pub struct ScanState {
    pub running: bool,
    pub methods: Vec<(&'static str, Result<usize, String>)>,
    /// A scan has finished at least once in this session.
    pub finished_once: bool,
}

/// Compatibility information for display, independent of where it came from (live verdict, list, or demo data).
#[derive(Clone, Debug)]
pub struct CompatInfo {
    pub status: Status,
    pub headline: String,
    pub details: Vec<String>,
    pub blocking: bool,
}

pub enum UpdateState {
    /// Nothing known (never checked, or checking disabled).
    Unknown,
    Checking,
    UpToDate,
    CheckFailed(String),
    Available(Box<UpdateInfo>),
    Downloading(Box<UpdateInfo>, Progress),
    Verified(Box<UpdateInfo>, Box<VerifiedDownload>),
    Failed(Box<UpdateInfo>, String),
}

pub struct Inputs {
    pub manual_ip: Entity<InputState>,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub user: Entity<InputState>,
    pub password: Entity<InputState>,
    pub sudo: Entity<InputState>,
    pub key_path: Entity<InputState>,
    pub hostname: Entity<InputState>,
    pub username: Entity<InputState>,
    pub new_password: Entity<InputState>,
    pub pubkey: Entity<InputState>,
    pub timezone: Entity<InputState>,
    pub mirror: Entity<InputState>,
    pub serial_text: Entity<InputState>,
    pub serial_speed: Entity<InputState>,
    pub dtb: Entity<InputState>,
    pub kexec_path: Entity<InputState>,
    pub kit_dir: Entity<InputState>,
    pub typed: Entity<InputState>,
    pub log_search: Entity<InputState>,
    pub filter: Entity<InputState>,
    pub add_ip: Entity<InputState>,
    pub bundle: Entity<InputState>,
    pub bm_timeout: Entity<InputState>,
    /// The Manage page's own sign-in (never the Connect form's).
    pub mg_user: Entity<InputState>,
    pub mg_password: Entity<InputState>,
    pub mg_key_path: Entity<InputState>,
}

pub struct PlanForm {
    pub req: PlanRequest,
    pub keep: BTreeSet<String>,
    pub copies_auto: bool,
    pub copies: usize,
    pub system_gib: f32,
    pub kernel: KernelChannel,
    pub plan: Option<Plan>,
    pub system_slider: Entity<SliderState>,
    pub system_range: (f32, f32),
    pub copies_slider: Entity<SliderState>,
    pub free_slider: Entity<SliderState>,
    pub raid_select: Entity<SelectState<Vec<SharedString>>>,
}

pub struct SysForm {
    pub key_mode: KeyMode,
    pub show_password: bool,
    pub enable_mgmt: bool,
    pub with_firmware: bool,
    pub verbose: bool,
    pub boot: BootChoice,
    pub generated_key: Option<(SshKeypair, PathBuf)>,
}

pub struct App {
    pub screen: Screen,
    pub return_to: Screen,
    pub settings: UiSettings,
    pub cfg_dir: PathBuf,
    pub demo: Option<DemoUnit>,
    pub tx: Tx,
    pub focus: FocusHandle,
    pub log_focus: FocusHandle,
    pub subs: Vec<Subscription>,
    pub error: Option<String>,

    pub inv: Inventory,
    pub selected_ip: Option<String>,
    pub scan: ScanState,
    pub compat: CompatDb,
    pub compat_refreshing: bool,
    pub compat_note: Option<(String, bool)>,

    pub inp: Inputs,
    pub use_key: bool,
    pub same_sudo: bool,
    pub inspecting: bool,

    pub facts: Option<DeviceFacts>,
    pub inspect_raw: String,
    pub goal: Goal,
    pub form: PlanForm,
    pub sys: SysForm,
    pub ack_backup: bool,
    pub ack_loss: bool,
    pub ack_unit: bool,
    /// Source: Offline in the System form.
    pub source_offline: bool,
    /// The app itself could not reach the Internet (a check failed); the status bar says Offline.
    pub app_offline: bool,
    /// "Password copied" on the Done screen (Finish stays disabled until it is ticked).
    pub password_copied: bool,
    pub inv_table: Entity<TableState<InventoryTable>>,
    /// The converted unit shown on the Manage page.
    pub manage_unit: Option<Found>,
    /// A dialog opened by a demo target, once the window exists.
    pub pending_overlay: Overlay,

    pub install: InstallView,
    pub install_started: bool,
    pub cancel: Arc<AtomicBool>,
    pub logger_path: Option<PathBuf>,
    pub result: Option<Result<Outcome, String>>,
    pub reveal_password: bool,
    pub log_follow: bool,
    pub log_selected_all: bool,
    pub log_scroll: gpui_kit::UniformListScrollHandle,
    /// The recovery action running now (one at a time).
    pub rec_running: Option<super::facts::RecKind>,
    /// The unit's boot state as read over SSH when Recovery opened (never this session's memory), or why it was not read.
    pub unit_state: Option<synofurb_core::recovery::UnitBootState>,
    pub unit_state_err: Option<String>,
    pub unit_state_loading: bool,
    /// A failure before the first write to the boot flash (typed `FlashBackupFailed`): the flash is untouched.
    pub flash_untouched: bool,
    pub dsm_wait: DsmWaitState,
    /// `(host, port, pinned fingerprint)` as last read from the known-hosts file (read again after every connection).
    /// Open host-key questions: ended when a connection attempt fails, so a late Trust is not offered.
    /// Bumped whenever another unit is opened or selected (or the session starts over): worker results carry the number they were
    /// started with and are dropped when it no longer matches.
    pub unit_gen: u64,
    /// Demo: Find shows no units (`Some(false)`: the scan finished empty, `Some(true)`: it never finishes).
    pub demo_no_units: Option<bool>,
    /// Set by "Cancel" while Open agent waits for the agent; a fresh flag per action.
    pub rec_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub open_trust: Vec<super::trust::TrustWatch>,
    pub pin_cache: Option<(String, u16, Option<String>)>,
    pub dsm_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub rec_result: Option<crate::ui::screens::recover::RecResult>,
    pub bootmgr: Job<()>,
    pub update: UpdateState,
    pub page_scroll: gpui_kit::ScrollHandle,
    pub help_view: Option<gpui_kit::WeakEntity<crate::ui::screens::help::HelpView>>,
    pub manage_tab: usize,
    /// The last `health --json` of the managed unit; `None` = it did not answer.
    pub manage_report: Option<synofurb_core::manage::Health>,
    pub manage_loading: bool,
    /// Which unit Manage is bound to; another unit starts with nothing.
    pub manage_bind: super::mgmt::ManageBinding,
    /// The user pressed "Read health" for the bound unit (the credential is the one typed on the Manage page).
    pub manage_signed_in: bool,
    /// Why the last health read failed (the unit's own error), shown instead of a guess.
    pub manage_err: Option<String>,
    pub mg_use_key: bool,
    /// Recovery was opened from Manage: its actions target the managed unit, not the Connect form.
    pub recover_for_manage: bool,
    pub mgmt: super::screens::manage::MgmtState,
    /// The engine's one-sentence result of the last management update or rollback.
    pub mgmt_note: Option<String>,
    /// Demo: the Storage tab shows the Hybrid layout.
    pub manage_hybrid: bool,
    /// The agent was told to return to DSM.
    /// The boot agent was armed but never answered (the default boot entry is not confirmed to be DSM).
    pub agent_unknown: bool,
    pub svc_fixed: bool,
    pub settings_auto: Option<Entity<SelectState<Vec<SharedString>>>>,
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

impl App {
    pub fn new(demo: Option<DemoTarget>, window: &mut Window, cx: &mut Context<App>) -> App {
        let cfg_dir = config_dir();
        let settings = if demo.is_some() { UiSettings::for_demo() } else { UiSettings::load(&UiSettings::path(&cfg_dir)) };
        let (tx, rx) = futures::channel::mpsc::unbounded::<Msg>();
        let is_demo = demo.is_some();
        if !is_demo {
            // Trust on first use with a question: the engine asks on a worker thread before any password is sent, the UI
            // shows "Trust this NAS?" and the worker waits for the answer (no answer = refuse).
            let txh = tx.clone();
            synofurb_core::hostkey::set_default_policy(synofurb_core::hostkey::HostKeyPolicy::tofu_with(
                synofurb_core::hostkey::KnownHosts::app_default(),
                super::trust::bridge(move |a| txh.unbounded_send(Msg::HostKeyAsk(Box::new(a))).is_ok(), super::trust::ANSWER_TIMEOUT),
            ));
        }
        let unit = DemoUnit::from_env();

        let mut subs: Vec<Subscription> = vec![];
        let mut input = |placeholder: &str, value: &str, masked: bool, subs: &mut Vec<Subscription>, cx: &mut Context<App>| {
            let st = cx.new(|cx| {
                let s = InputState::new(window, cx).placeholder(placeholder.to_string()).default_value(value.to_string());
                if masked { s.masked(true) } else { s }
            });
            subs.push(cx.observe(&st, |_, _, cx| cx.notify()));
            st
        };
        let inp = Inputs {
            manual_ip: input("192.168.1.20", "", false, &mut subs, cx),
            host: input("192.168.1.20 or nas.local", "", false, &mut subs, cx),
            port: input("22", "22", false, &mut subs, cx),
            user: input("admin", "admin", false, &mut subs, cx),
            password: input("SSH password", "", true, &mut subs, cx),
            sudo: input("empty = same as the SSH password", "", true, &mut subs, cx),
            key_path: input("path to an OpenSSH private key", "", false, &mut subs, cx),
            hostname: input("nas", "nas", false, &mut subs, cx),
            username: input("nasadmin", "nasadmin", false, &mut subs, cx),
            new_password: input("password", &generate_password(20), true, &mut subs, cx),
            pubkey: input("ssh-ed25519 AAAA... me@laptop", "", false, &mut subs, cx),
            timezone: input("UTC", "UTC", false, &mut subs, cx),
            mirror: input("deb.debian.org", "deb.debian.org", false, &mut subs, cx),
            serial_text: input("ttyS0", "ttyS0", false, &mut subs, cx),
            serial_speed: input("115200", "115200", false, &mut subs, cx),
            dtb: input("armada-385-synology-ds116.dtb", "", false, &mut subs, cx),
            kexec_path: input("assets/kexec/<arch>/kexec (auto-detected)", "", false, &mut subs, cx),
            kit_dir: input("folder for the installer kit", &cfg_dir.join("kit").display().to_string(), false, &mut subs, cx),
            typed: input("", "", false, &mut subs, cx),
            log_search: input("Search the log", "", false, &mut subs, cx),
            filter: input(copy::INV_FILTER.short, "", false, &mut subs, cx),
            add_ip: input("192.168.1.20", "", false, &mut subs, cx),
            bundle: input("/path/to/bundle", "", false, &mut subs, cx),
            bm_timeout: input("", "30 min", false, &mut subs, cx),
            mg_user: input("nasadmin", "nasadmin", false, &mut subs, cx),
            mg_password: input("password of this NAS", "", true, &mut subs, cx),
            mg_key_path: input("path to an OpenSSH private key", "", false, &mut subs, cx),
        };
        // Enter in the connection fields starts the inspection (the primary action of that screen).
        for st in [&inp.host, &inp.port, &inp.user, &inp.password, &inp.sudo, &inp.key_path] {
            subs.push(cx.subscribe_in(st, window, |this: &mut App, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) && this.screen == Screen::Connect {
                    this.start_inspect(window, cx);
                }
            }));
        }
        subs.push(cx.subscribe_in(&inp.manual_ip, window, |this: &mut App, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.use_manual_address(window, cx);
            }
        }));
        subs.push(cx.subscribe(&inp.log_search, |this: &mut App, st, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                let text = st.read(cx).value().to_string();
                this.install.log.set_text(&text);
                this.log_selected_all = false;
                cx.notify();
            }
        }));

        // Follow the operating system's light/dark setting while the choice is "follow system".
        subs.push(cx.observe_window_appearance(window, |this: &mut App, _window, cx| {
            if this.settings.theme == ThemeChoice::System {
                let scale = this.settings.text_scale;
                theme::apply(theme::os_is_dark(cx), scale, this.settings.accent, cx);
            }
        }));

        // Plan form widgets.
        let raid_items: Vec<SharedString> = DataRaid::ALL.iter().map(|r| SharedString::from(super::raid::raid_label(*r))).collect();
        let raid_select = cx.new(|cx| SelectState::new(raid_items, Some(IndexPath::default().row(0)), window, cx));
        subs.push(cx.subscribe(&raid_select, |this: &mut App, _, ev: &SelectEvent<Vec<SharedString>>, cx| {
            let SelectEvent::Confirm(Some(v)) = ev else { return };
            if let Some(r) = DataRaid::ALL.iter().find(|r| super::raid::raid_label(**r) == v.as_ref()) {
                this.form.req.data_raid = *r;
                this.rebuild_plan();
                cx.notify();
            }
        }));
        let free_slider = cx.new(|_| SliderState::new().min(0.0).max(90.0).step(5.0).default_value(20.0));
        subs.push(cx.subscribe(&free_slider, |this: &mut App, _, ev: &SliderEvent, cx| {
            if let SliderEvent::Change(v) = ev {
                this.form.req.data_free_percent = v.start().round().clamp(0.0, 90.0) as u8;
                this.rebuild_plan();
                cx.notify();
            }
        }));
        let copies_slider = cx.new(|_| SliderState::new().min(1.0).max(2.0).step(1.0).default_value(2.0));
        subs.push(cx.subscribe(&copies_slider, |this: &mut App, _, ev: &SliderEvent, cx| {
            if let SliderEvent::Change(v) = ev {
                this.form.copies = v.start().round().max(1.0) as usize;
                this.rebuild_plan();
                cx.notify();
            }
        }));
        let system_slider = cx.new(|_| SliderState::new().min(8.0).max(64.0).step(1.0).default_value(16.0));

        let form = PlanForm {
            req: PlanRequest::default(),
            keep: BTreeSet::new(),
            copies_auto: true,
            copies: 2,
            system_gib: 0.0,
            kernel: KernelChannel::Newest,
            plan: None,
            system_slider,
            system_range: (0.0, 0.0),
            copies_slider,
            free_slider,
            raid_select,
        };

        let (inv, compat) = if is_demo {
            (demo::inventory(), CompatDb::embedded())
        } else {
            (Inventory::load(&cfg_dir.join("inventory.json")), CompatDb::load_best(&cfg_dir))
        };

        let inv_table = cx.new(|cx| TableState::new(InventoryTable::default(), window, cx));
        subs.push(cx.subscribe(&inv_table, |this: &mut App, st, ev: &TableEvent, cx| {
            if let TableEvent::SelectRow(ix) = ev {
                let ip = st.read(cx).delegate().ip_at(*ix);
                if ip.is_some() {
                    this.selected_ip = ip;
                    cx.notify();
                }
            }
        }));
        let mut app = App {
            screen: Screen::Inventory,
            return_to: Screen::Inventory,
            settings,
            cfg_dir,
            demo: is_demo.then_some(unit),
            tx: tx.clone(),
            focus: cx.focus_handle(),
            log_focus: cx.focus_handle(),
            subs,
            error: None,
            inv,
            selected_ip: None,
            scan: ScanState::default(),
            compat,
            compat_refreshing: false,
            compat_note: None,
            inp,
            use_key: false,
            same_sudo: true,
            inspecting: false,
            facts: None,
            inspect_raw: String::new(),
            goal: Goal::Install,
            form,
            sys: SysForm {
                key_mode: KeyMode::Generate,
                show_password: false,
                enable_mgmt: true,
                with_firmware: true,
                verbose: true,
                boot: BootChoice::Auto,
                generated_key: None,
            },
            ack_backup: false,
            ack_loss: false,
            ack_unit: false,
            source_offline: false,
            app_offline: false,
            password_copied: false,
            inv_table,
            manage_unit: None,
            pending_overlay: Overlay::None,
            install: InstallView::default(),
            install_started: false,
            cancel: Arc::new(AtomicBool::new(false)),
            logger_path: None,
            result: None,
            reveal_password: false,
            log_follow: true,
            log_selected_all: false,
            log_scroll: gpui_kit::UniformListScrollHandle::new(),
            rec_running: None,
            unit_state: None,
            unit_state_err: None,
            unit_state_loading: false,
            flash_untouched: false,
            dsm_wait: DsmWaitState::Idle,
            unit_gen: 0,
            demo_no_units: None,
            rec_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            open_trust: vec![],
            pin_cache: None,
            dsm_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            rec_result: None,
            bootmgr: Job::Idle,
            update: UpdateState::Unknown,
            page_scroll: gpui_kit::ScrollHandle::new(),
            help_view: None,
            manage_tab: 0,
            manage_report: None,
            manage_loading: false,
            manage_bind: Default::default(),
            manage_signed_in: false,
            manage_err: None,
            mg_use_key: false,
            recover_for_manage: false,
            mgmt: super::screens::manage::MgmtState::Idle,
            mgmt_note: None,
            manage_hybrid: false,
            agent_unknown: false,
            svc_fixed: false,
            settings_auto: Some(App::make_auto_select(window, cx)),
        };

        // Message pump: workers send on `tx`; this task applies the messages on the UI thread.
        let mut rx = rx;
        cx.spawn_in(window, async move |this, cx| {
            while let Some(m) = rx.next().await {
                if this.update_in(cx, |this, window, cx| this.on_msg(m, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        match demo {
            Some(t) => app.start_demo(t, unit, window, cx),
            None => {
                app.start_scan(cx);
                app.maybe_check_updates();
                if !app.settings.welcome_done {
                    app.pending_overlay = Overlay::Welcome(0);
                    cx.defer_in(window, |this, window, cx| this.open_pending_overlay(window, cx));
                }
            }
        }
        app
    }

    // ------------------------------------------------------------------------------------------ navigation

    /// Another unit of the same model on the network (a twin): the confirmation phrase then carries the serial tail.
    pub fn twin_of_selected(&self, cx: &gpui_kit::App) -> Option<Found> {
        let me = self.facts.as_ref()?;
        let host = super::format::display_host(&self.input_value(&self.inp.host, cx));
        self.inv
            .devices
            .iter()
            .find(|d| {
                !d.synofurb_debian
                    && !me.model.is_empty()
                    && d.model == me.model
                    && !d.serial.is_empty()
                    && d.serial != me.serial
                    && d.primary_ip().map(|i| i.to_string()).is_some_and(|i| !host.contains(&i))
            })
            .cloned()
    }

    pub fn go(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<App>) {
        if screen == Screen::Settings && self.screen != Screen::Settings {
            self.return_to = self.screen;
        }
        if screen != Screen::Manage && screen != Screen::Recover {
            self.recover_for_manage = false;
        }
        self.screen = screen;
        self.error = None;
        if screen == Screen::Recover {
            self.read_unit_state(cx);
        }
        window.focus(&self.focus, cx);
        self.page_scroll.set_offset(gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(0.0)));
        cx.notify();
    }

    pub fn current_step(&self) -> Step {
        self.screen.step().or(self.return_to.step()).unwrap_or(Step::Find)
    }

    pub fn set_theme(&mut self, choice: ThemeChoice, cx: &mut Context<App>) {
        self.settings.theme = choice;
        self.save_settings();
        let dark = choice.is_dark(theme::os_is_dark(cx));
        theme::apply(dark, self.settings.text_scale, self.settings.accent, cx);
        cx.notify();
    }

    pub fn save_settings(&self) {
        if self.demo.is_none() {
            let _ = self.settings.save(&UiSettings::path(&self.cfg_dir));
        }
    }

    pub fn toast(&self, window: &mut Window, cx: &mut gpui_kit::App, msg: &str, ok: bool) {
        use gpui_kit::component::notification::Notification;
        let n = if ok { Notification::success(msg.to_string()) } else { Notification::error(msg.to_string()) };
        window.push_notification(n, cx);
    }

    pub fn copy_text(&self, text: String, what: &str, window: &mut Window, cx: &mut gpui_kit::App) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.toast(window, cx, &format!("{what} copied"), true);
    }

    // ------------------------------------------------------------------------------------------ demo

    fn start_demo(&mut self, t: DemoTarget, unit: DemoUnit, window: &mut Window, cx: &mut Context<App>) {
        self.selected_ip = Some(unit.address().to_string());
        self.set_input(&self.inp.host.clone(), &format!("{} (demo)", unit.address()), window, cx);
        self.set_input(&self.inp.password.clone(), "demo-not-a-real-password", window, cx);
        self.app_offline = t.offline;
        self.source_offline = t.offline && t.screen == Screen::System;
        let past_inspect = !matches!(t.screen, Screen::Inventory | Screen::Connect | Screen::Settings | Screen::About);
        if past_inspect {
            let raw = unit.raw();
            let facts = unit.facts();
            self.on_inspected(facts, raw, false, window, cx);
        }
        if let Some(scanning) = t.no_units {
            self.inv = Inventory::default();
            self.demo_no_units = Some(scanning);
            self.selected_ip = None;
        }
        if t.twin {
            self.inv.merge(demo::twin_of_ds918());
        }
        if t.keep {
            self.form.req.mode = StorageMode::KeepVolumes;
            self.rebuild_plan();
        }
        if t.screen == Screen::Failed {
            // the state the dialogs read exists before the first frame (the replayed events only add to it)
            self.install.past_no_return = t.past_no_return;
        }
        match t.screen {
            Screen::Confirm if t.ready || std::env::var("SYNOFURB_DEMO_GATE").as_deref() == Ok("ready") => {
                self.ack_backup = true;
                self.ack_loss = true;
                self.ack_unit = true;
                let phrase = self.phrase(cx);
                self.set_input(&self.inp.typed.clone(), &phrase, window, cx);
            }
            Screen::Confirm => {
                self.ack_backup = true;
                let partial: String = self.phrase(cx).chars().take(9).collect();
                self.set_input(&self.inp.typed.clone(), &partial, window, cx);
            }
            Screen::Failed if t.fixture == Fixture::Agent => {
                self.ack_backup = true;
                self.ack_loss = true;
                self.ack_unit = true;
                self.install_started = true;
                self.agent_unknown = true;
                self.result = Some(Err("the boot agent did not appear within 10 minutes. The NAS defaults to DSM: power-cycle it if it is stuck and it will boot DSM normally.".into()));
            }
            Screen::Install | Screen::Failed | Screen::Done => {
                self.ack_backup = true;
                self.ack_loss = true;
                self.ack_unit = true;
                let model = self.facts.as_ref().map(|f| f.model.clone()).unwrap_or_default();
                let script = match (t.screen, t.past_no_return) {
                    (Screen::Install, true) => Script::Running,
                    (Screen::Install, false) => Script::RunningEarly,
                    (Screen::Failed, true) => Script::Fails,
                    (Screen::Failed, false) => Script::FailsEarly,
                    _ => Script::Succeeds,
                };
                self.install_started = true;
                self.screen = Screen::Install;
                if script == Script::Succeeds {
                    // Done needs a generated key as the real flow would have produced.
                    if let Ok(kp) = generate_ssh_keypair("synofurb-demo") {
                        self.sys.generated_key = Some((kp, self.cfg_dir.join("keys").join("nas_ed25519")));
                    }
                }
                let out = demo::outcome(unit, "nas", "nasadmin");
                demo::spawn_script(self.tx.clone(), script, unit, model, Some(out));
            }
            Screen::BootMgr => self.goal = Goal::BootManager,
            Screen::Recover => {
                self.goal = Goal::Recover;
                match t.fixture {
                    Fixture::None => {}
                    Fixture::Failed | Fixture::Debian | Fixture::Agent => {
                        self.bootmgr = Job::Done(Ok(()));
                        self.install.past_no_return = t.fixture != Fixture::Agent;
                        if t.fixture == Fixture::Debian {
                            self.result = Some(Ok(demo::outcome(unit, "nas", "nasadmin")));
                        }
                        self.agent_unknown = t.fixture == Fixture::Agent;
                    }
                }
            }
            Screen::Manage => {
                self.manage_unit = demo::inventory().devices.into_iter().find(|d| d.synofurb_debian);
                if let Some(u) = &self.manage_unit {
                    self.manage_bind.bind(&super::mgmt::unit_key(&u.serial, u.primary_ip()));
                }
                use super::mgmt::DemoHealth as DH;
                self.manage_signed_in = t.health != DH::NeedsSignIn;
                self.manage_report = (!matches!(t.health, DH::Unreachable | DH::NeedsSignIn)).then(|| super::mgmt::demo_health(t.health));
                self.manage_err = (t.health == DH::Unreachable).then(|| "connection to 192.168.1.60 timed out".to_string());
                self.manage_tab = t.tab;
                self.manage_hybrid = t.hybrid_view;
                if t.updating {
                    self.mgmt = super::screens::manage::MgmtState::Updating(62);
                }
            }
            _ => {}
        }
        if t.screen != Screen::Install {
            self.screen = t.screen;
        }
        if matches!(t.screen, Screen::Inventory) {
            self.start_scan(cx);
        }
        if t.screen.is_side_page() {
            self.return_to = Screen::Inventory;
        }
        // Dialogs and the Help sheet open once the first frame exists.
        self.pending_overlay = t.overlay;
        match t.overlay {
            Overlay::Update => {
                self.update = UpdateState::Available(Box::new(demo::update_info()));
            }
            Overlay::None => {}
            _ => {}
        }
        if t.overlay != Overlay::None {
            cx.defer_in(window, |this, window, cx| this.open_pending_overlay(window, cx));
        }
        if t.screen == Screen::Confirm && std::env::var("SYNOFURB_DEMO_SCROLL").as_deref() == Ok("bottom") {
            // Screenshots of the gate: scroll to the confirmation fields once the page has been laid out.
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                let _ = this.update(cx, |this, cx| {
                    this.page_scroll.scroll_to_bottom();
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Open the dialog or sheet a demo target asked for (and, in a real run, the Welcome guide until it is dismissed).
    pub fn open_pending_overlay(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let o = std::mem::take(&mut self.pending_overlay);
        let app = cx.entity();
        match o {
            Overlay::None => {}
            Overlay::Update => self.open_update_dialog(window, cx),
            Overlay::Help(topic) => self.open_help_topic(topic, window, cx),
            Overlay::RecoveryDialog => super::disclosure::dialog(window, cx, &copy::DLG_RECOVERY, &app),
            Overlay::KeepDialog => self.open_keep_dialog(window, cx),
            Overlay::HostKey => {
                let fp = "SHA256:q3Zb1c8mPvX0dT7nKfL2yHrW9sUeJ5aG4oB6iNxVwYc".to_string();
                let ask = super::trust::TrustAsk::demo(synofurb_core::hostkey::HostKeyPrompt {
                    host: "192.168.1.20".into(),
                    port: 22,
                    fingerprint: fp,
                });
                self.open_hostkey_dialog(ask, window, cx)
            }
            Overlay::HostKeyEnded => {
                let fp = "SHA256:q3Zb1c8mPvX0dT7nKfL2yHrW9sUeJ5aG4oB6iNxVwYc".to_string();
                let ask = super::trust::TrustAsk::demo(synofurb_core::hostkey::HostKeyPrompt {
                    host: "192.168.1.20".into(),
                    port: 22,
                    fingerprint: fp,
                });
                ask.watch().end();
                self.open_hostkey_dialog(ask, window, cx)
            }
            Overlay::HostKeyChanged => self.open_hostkey_changed(
                "192.168.1.20".into(),
                22,
                "SHA256:q3Zb1c8mPvX0dT7nKfL2yHrW9sUeJ5aG4oB6iNxVwYc".into(),
                "SHA256:Hd8vN0cYk2tqLxP4wZr1eJm7aB9sUf3oGiX5nKyVd6E".into(),
                window,
                cx,
            ),
            Overlay::Quick => self.open_quick_dialog(window, cx),
            Overlay::Manual => self.open_manual_recovery(window, cx),
            Overlay::NextDialog => self.open_next_dialog(window, cx),
            Overlay::Welcome(step) => self.open_welcome(step, window, cx),
        }
    }

    pub fn set_input(&self, st: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut Context<App>) {
        let v = value.to_string();
        st.update(cx, |s, cx| s.set_value(v, window, cx));
    }

    pub fn input_value(&self, st: &Entity<InputState>, cx: &gpui_kit::App) -> String {
        st.read(cx).value().to_string()
    }

    // ------------------------------------------------------------------------------------------ messages

    pub fn on_msg(&mut self, m: Msg, window: &mut Window, cx: &mut Context<App>) {
        // Results of work started for a unit only count while that unit is the one on screen.
        let m = match m {
            Msg::Scoped { epoch, msg } => {
                let manage_result = matches!(*msg, Msg::ManageHealth(_) | Msg::MgmtProgress(_) | Msg::MgmtDone(_));
                if !super::mgmt::accepts_result(epoch, self.unit_gen, manage_result, self.manage_bound()) {
                    if manage_result && epoch == self.unit_gen {
                        // same unit, but the user left Manage: forget what was in flight; opening Manage again reads afresh
                        self.manage_loading = false;
                        if matches!(self.mgmt, super::screens::manage::MgmtState::Updating(_)) {
                            self.mgmt = super::screens::manage::MgmtState::Idle;
                        }
                    }
                    return;
                }
                *msg
            }
            other => other,
        };
        // A connection that failed ends any open "Trust this NAS?" question: Trust would no longer do anything.
        let failed = match &m {
            Msg::Inspected(Err(_)) | Msg::ManageHealth(Err(_)) | Msg::UnitState(Err(_)) | Msg::Finished(Err(_)) => true,
            Msg::BootManagerDone(Err(_)) | Msg::MgmtDone(Err(_)) => true,
            Msg::RecoveryDone(o) => o.level != worker::RecLevel::Done && o.level != worker::RecLevel::Info,
            Msg::DsmWaitDone(w) => w.timed_out,
            _ => false,
        };
        if failed {
            for w in self.open_trust.drain(..) {
                w.end();
            }
        }
        match m {
            Msg::ScanMethod { name, result } => {
                self.scan.methods.push((name, result));
            }
            Msg::ScanPartial(inv) => self.inv = inv,
            Msg::ScanDone(inv) => {
                if self.demo.is_none() {
                    let _ = inv.save(&self.cfg_dir.join("inventory.json"));
                }
                self.inv = inv;
                self.scan.running = false;
                self.scan.finished_once = true;
            }
            Msg::Inspected(r) => {
                self.pin_cache = None;
                self.inspecting = false;
                match r {
                    Ok((f, raw)) => self.on_inspected(*f, raw, true, window, cx),
                    Err(e) => self.error = Some(e),
                }
            }
            Msg::Ev(e) => {
                if matches!(e, Event::PointOfNoReturn) && !self.install.past_no_return {
                    // The first time only: announce it for assistive technology and the user.
                    self.toast(window, cx, "Point of no return passed. The disks are being changed.", false);
                }
                self.install.apply(e);
                if self.log_follow {
                    self.scroll_log_to_end();
                }
            }
            Msg::Finished(r) => {
                if let Err(e) = &r {
                    // the typed flag from `FailureKind` is authoritative; the text is only a fallback
                    self.agent_unknown |= super::facts::is_agent_timeout(e);
                }
                let ok = r.is_ok();
                self.result = Some(r);
                self.screen = if ok { Screen::Done } else { Screen::Failed };
                window.focus(&self.focus, cx);
            }
            Msg::Scoped { .. } => {}
            Msg::HostKeyAsk(ask) => {
                self.open_trust.retain(|w| matches!(w.state(std::time::Instant::now()), super::trust::TrustState::Waiting { .. }));
                self.open_trust.push(ask.watch());
                self.open_hostkey_dialog(*ask, window, cx)
            }
            Msg::HostKeyChanged { host, port, pinned, presented } => self.open_hostkey_changed(host, port, pinned, presented, window, cx),
            Msg::FailureKind { agent_timeout, flash_untouched } => {
                self.agent_unknown = agent_timeout;
                self.flash_untouched = flash_untouched;
            }
            Msg::RecoveryDone(o) => {
                let kind = self.rec_running.take();
                let o = *o;
                if let Some(st) = o.state {
                    self.unit_state = Some(st);
                    self.unit_state_err = None;
                }
                self.rec_result = Some(super::screens::recover::RecResult {
                    kind,
                    level: o.level,
                    title: o.title,
                    detail: o.detail,
                    commands: o.commands,
                });
            }
            Msg::UnitState(r) => {
                self.unit_state_loading = false;
                match r {
                    Ok(s) => {
                        self.unit_state = Some(*s);
                        self.unit_state_err = None;
                    }
                    Err(e) => {
                        self.unit_state = None;
                        self.unit_state_err = Some(e);
                    }
                }
            }
            Msg::DsmProbe { last, secs } => self.dsm_wait = DsmWaitState::Waiting { secs, last: Some(last) },
            Msg::DsmWaitDone(w) => {
                self.dsm_wait = DsmWaitState::Done { last: w.last, timed_out: w.timed_out, secs: w.waited_s };
            }
            Msg::ManageHealth(r) => {
                self.manage_loading = false;
                match r {
                    Ok(h) => {
                        self.manage_report = Some(*h);
                        self.manage_err = None;
                    }
                    Err(e) => {
                        self.manage_report = None;
                        self.manage_err = Some(e);
                    }
                }
                self.svc_fixed = false;
            }
            Msg::BootManagerDone(r) => self.bootmgr = Job::Done(r),
            Msg::MgmtProgress(p) => {
                if matches!(self.mgmt, super::screens::manage::MgmtState::Updating(_)) {
                    self.mgmt = super::screens::manage::MgmtState::Updating(p.max(4));
                }
            }
            Msg::MgmtDone(r) => {
                use super::screens::manage::MgmtState;
                use synofurb_core::manage::UpdateOutcome as O;
                let mut reread: Option<SshTarget> = None;
                match r {
                    Ok(d) => {
                        reread = Some(d.target.clone());
                        self.mgmt = match d.outcome {
                            O::Updated | O::UpToDate | O::RolledBack => MgmtState::Updated,
                            O::Refused | O::Failed => MgmtState::Failed,
                        };
                        self.mgmt_note = Some(d.headline);
                    }
                    Err(e) => {
                        self.mgmt = MgmtState::Failed;
                        self.mgmt_note = Some(e);
                    }
                }
                // the health after the update is read with the target the update ran against, never with `target()`
                if let Some(t) = reread {
                    self.manage_loading = true;
                    self.manage_err = None;
                    worker::spawn_manage_health(self.tx.clone(), self.unit_gen, t);
                }
            }
            Msg::CompatRefreshed(r) => {
                self.compat_refreshing = false;
                self.compat = r.db;
                self.compat_note = Some((r.summary, r.ok));
            }
            Msg::UpdateChecked(r) => match r {
                Ok(info) if info.update_available => {
                    if self.settings.should_announce(&info.latest, now_unix()) {
                        self.update = UpdateState::Available(info);
                    } else {
                        self.update = UpdateState::UpToDate;
                    }
                }
                Ok(_) => self.update = UpdateState::UpToDate,
                Err(e) => self.update = UpdateState::CheckFailed(e),
            },
            Msg::UpdateProgress(p) => {
                if let UpdateState::Downloading(info, _) = &self.update {
                    self.update = UpdateState::Downloading(info.clone(), p);
                }
            }
            Msg::UpdateDownloaded(r) => {
                let info = match std::mem::replace(&mut self.update, UpdateState::Unknown) {
                    UpdateState::Downloading(i, _)
                    | UpdateState::Available(i)
                    | UpdateState::Failed(i, _)
                    | UpdateState::Verified(i, _) => i,
                    other => {
                        self.update = other;
                        return;
                    }
                };
                self.update = match r {
                    Ok(vd) => UpdateState::Verified(info, vd),
                    Err(e) => UpdateState::Failed(info, e),
                };
            }
        }
        cx.notify();
    }

    pub fn scroll_log_to_end(&self) {
        let n = self.install.log.visible_len();
        if n > 0 {
            self.log_scroll.scroll_to_item(n - 1, gpui_kit::ScrollStrategy::Bottom);
        }
    }

    // ------------------------------------------------------------------------------------------ discovery

    pub fn start_scan(&mut self, cx: &mut Context<App>) {
        if self.scan.running {
            return;
        }
        self.scan = ScanState { running: true, methods: vec![], finished_once: self.scan.finished_once };
        if let (Some(_), Some(scanning)) = (self.demo, self.demo_no_units) {
            // Demo "no units": the scan never ends (skeleton rows) or ends with nothing found (the empty state).
            if !scanning {
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    let _ = tx.unbounded_send(Msg::ScanDone(Inventory::default()));
                });
            }
            cx.notify();
            return;
        }
        if self.demo.is_some() {
            // Replay the demo inventory as if it were streaming in.
            let tx = self.tx.clone();
            let full = demo::inventory();
            std::thread::spawn(move || {
                let mut inv = Inventory::default();
                for (i, name) in
                    ["Synology Assistant broadcast", "Bonjour/mDNS", "UPnP/SSDP", "Subnet sweep (DSM ports)"].iter().enumerate()
                {
                    std::thread::sleep(std::time::Duration::from_millis(350));
                    for d in full.devices.iter().skip(i * 2).take(2) {
                        inv.merge(d.clone());
                    }
                    let _ = tx.unbounded_send(Msg::ScanPartial(inv.clone()));
                    let _ = tx.unbounded_send(Msg::ScanMethod { name, result: Ok(2) });
                }
                let _ = tx.unbounded_send(Msg::ScanDone(full));
            });
            self.inv = Inventory::default();
        } else {
            worker::spawn_scan(self.tx.clone(), ScanOptions::default(), self.inv.clone());
        }
        cx.notify();
    }

    pub fn use_manual_address(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let ip = self.input_value(&self.inp.manual_ip, cx).trim().to_string();
        if ip.is_empty() {
            return;
        }
        self.select_host(&ip, window, cx);
    }

    /// Facts that belong to one unit's session (the app cannot read them from the unit yet): cleared when another unit
    /// is selected and on "start over", so one unit's failed agent never shows on the next unit.
    pub fn reset_unit_session(&mut self) {
        self.unit_gen += 1;
        self.agent_unknown = false;
        self.install_started = false;
        self.bootmgr = Job::Idle;
        self.rec_running = None;
        self.rec_result = None;
        self.unit_state = None;
        self.unit_state_err = None;
        self.unit_state_loading = false;
        self.flash_untouched = false;
        self.dsm_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.dsm_wait = DsmWaitState::Idle;
    }

    /// The host key pinned for the unit in the host and port fields (`None` = never trusted yet). In the demo a sample pin.
    pub fn host_pin(&mut self, cx: &mut Context<App>) -> Option<String> {
        let host = crate::ui::format::display_host(&self.input_value(&self.inp.host, cx));
        let port: u16 = self.input_value(&self.inp.port, cx).trim().parse().unwrap_or(22);
        if host.is_empty() {
            return None;
        }
        if self.demo.is_some() {
            return Some("SHA256:q3Zb1c8mPvX0dT7nKfL2yHrW9sUeJ5aG4oB6iNxVwYc".to_string());
        }
        if let Some((h, p, fp)) = &self.pin_cache
            && *h == host
            && *p == port
        {
            return fp.clone();
        }
        let fp = synofurb_core::hostkey::KnownHosts::app_default().get(&host, port).ok().flatten().map(|k| k.fingerprint);
        self.pin_cache = Some((host, port, fp.clone()));
        fp
    }

    pub fn select_host(&mut self, host: &str, window: &mut Window, cx: &mut Context<App>) {
        if self.selected_ip.as_deref() != Some(host) {
            self.reset_unit_session();
        }
        self.selected_ip = Some(host.to_string());
        self.set_input(&self.inp.host.clone(), host, window, cx);
        self.go(Screen::Connect, window, cx);
    }

    /// Compatibility status for a discovered unit (before inspection): the list entry, or Untested.
    pub fn compat_for_found(&self, d: &Found) -> (Status, Option<String>) {
        if self.demo.is_some()
            && let Some(s) = demo::status_override(&d.model)
        {
            return (s, demo::status_reason(&d.model).map(String::from));
        }
        match self.compat.lookup(&d.model) {
            Some(e) => {
                let reason = match e.status {
                    Status::Verified => None,
                    Status::Unsupported => e.notes.clone(),
                    _ => e.notes.clone(),
                };
                (e.status, reason.map(|r| r.chars().take(110).collect::<String>()))
            }
            None => (Status::Untested, Some("model not in the compatibility list".into())),
        }
    }

    /// Compatibility information for the inspected unit.
    pub fn compat_for_facts(&self, f: &DeviceFacts) -> CompatInfo {
        let v = self.compat.status_for(f);
        let mut info = CompatInfo { status: v.status, headline: v.headline, details: v.details, blocking: v.blocking };
        if self.demo.is_some()
            && !info.blocking
            && let Some(s) = demo::status_override(&f.model)
        {
            info.status = s;
            info.headline = format!("{}: {} (demo data)", f.model, s.label().to_lowercase());
        }
        info
    }

    // ------------------------------------------------------------------------------------------ connect / inspect

    /// The SSH target of what is on screen: Manage (and Recovery opened from it) uses the managed unit and the sign-in typed on
    /// that page; everything else uses the Connect form. The two never mix.
    pub fn target(&self, cx: &gpui_kit::App) -> Result<SshTarget, String> {
        if self.manage_bound() {
            return self.manage_target(cx);
        }
        self.form_target(cx)
    }

    /// Is what the user sees about the managed unit (Manage, or Recovery opened from it)?
    pub fn manage_bound(&self) -> bool {
        self.screen == Screen::Manage || (self.screen == Screen::Recover && self.recover_for_manage)
    }

    pub fn manage_target(&self, cx: &gpui_kit::App) -> Result<SshTarget, String> {
        use super::mgmt::{ManageSecret, manage_target_for};
        let addr = self.manage_unit.as_ref().and_then(|u| u.primary_ip());
        let secret = if self.mg_use_key {
            let p = self.input_value(&self.inp.mg_key_path, cx);
            ManageSecret::Key(std::fs::read_to_string(p.trim()).map_err(|e| format!("cannot read the key file: {e}"))?)
        } else {
            ManageSecret::Password(self.input_value(&self.inp.mg_password, cx))
        };
        manage_target_for(addr, &self.input_value(&self.inp.mg_user, cx), &secret)
    }

    /// The address actions work on: the managed unit's on Manage (and Recovery from it), else the Connect form's.
    pub fn active_host(&self, cx: &gpui_kit::App) -> String {
        if self.manage_bound() {
            return self.manage_unit.as_ref().and_then(|u| u.primary_ip()).map(|i| i.to_string()).unwrap_or_default();
        }
        super::format::display_host(&self.input_value(&self.inp.host, cx))
    }

    pub fn active_user(&self, cx: &gpui_kit::App) -> String {
        if self.manage_bound() {
            return self.input_value(&self.inp.mg_user, cx).trim().to_string();
        }
        self.input_value(&self.inp.user, cx).trim().to_string()
    }

    /// Open Manage for `unit`. Another unit than the bound one starts clean: no report, no sign-in, no recovery facts.
    pub fn open_manage(&mut self, unit: Found, window: &mut Window, cx: &mut Context<App>) {
        let key = super::mgmt::unit_key(&unit.serial, unit.primary_ip());
        if self.manage_bind.bind(&key) {
            self.manage_report = None;
            self.manage_err = None;
            self.manage_loading = false;
            self.manage_signed_in = false;
            self.mgmt = super::screens::manage::MgmtState::Idle;
            self.mgmt_note = None;
            self.reset_unit_session();
            self.set_input(&self.inp.mg_password.clone(), "", window, cx);
        }
        self.manage_unit = Some(unit);
        self.recover_for_manage = false;
        self.go(Screen::Manage, window, cx);
        if self.manage_signed_in && self.manage_report.is_none() {
            self.read_manage_health(cx);
        }
    }

    pub fn form_target(&self, cx: &gpui_kit::App) -> Result<SshTarget, String> {
        let password = self.input_value(&self.inp.password, cx);
        let auth = if self.use_key {
            let p = self.input_value(&self.inp.key_path, cx);
            Auth::Key(std::fs::read_to_string(p.trim()).map_err(|e| format!("cannot read the key file: {e}"))?)
        } else {
            Auth::Password(password.clone())
        };
        let sudo_typed = self.input_value(&self.inp.sudo, cx);
        let sudo = if self.same_sudo || sudo_typed.is_empty() { password } else { sudo_typed };
        let host = super::format::display_host(&self.input_value(&self.inp.host, cx));
        if host.is_empty() {
            return Err("Enter the address of the NAS.".into());
        }
        Ok(SshTarget {
            host,
            port: self.input_value(&self.inp.port, cx).trim().parse().map_err(|_| "The SSH port must be a number.".to_string())?,
            user: self.input_value(&self.inp.user, cx).trim().to_string(),
            auth,
            sudo_password: Some(sudo).filter(|s| !s.is_empty()),
        })
    }

    pub fn start_inspect(&mut self, window: &mut Window, cx: &mut Context<App>) {
        if self.inspecting {
            return;
        }
        if self.demo.is_some() {
            let (raw, facts) = (self.demo.unwrap().raw(), self.demo.unwrap().facts());
            self.on_inspected(facts, raw, true, window, cx);
            return;
        }
        match self.target(cx) {
            Ok(t) => {
                self.error = None;
                self.inspecting = true;
                worker::spawn_inspect(self.tx.clone(), t);
            }
            Err(e) => self.error = Some(e),
        }
        cx.notify();
    }

    pub fn on_inspected(&mut self, f: DeviceFacts, raw: String, navigate: bool, window: &mut Window, cx: &mut Context<App>) {
        self.error = None;
        self.form.req = PlanRequest::default();
        self.form.keep = f.data_arrays().iter().map(|a| a.name.clone()).collect();
        let n = f.candidate_disks().len();
        self.form.copies = default_system_copies(n, n).max(1);
        self.form.copies_auto = true;
        self.form.system_gib = 0.0;
        self.form.kernel = KernelChannel::Newest;
        let serial = f.serial_console();
        self.set_input(&self.inp.serial_text.clone(), &serial_field_text(&serial), window, cx);
        self.set_input(&self.inp.serial_speed.clone(), &serial.speed.to_string(), window, cx);
        let dtb = platform::mainline_dtb(&f.model).or_else(|| platform::mainline_dtb(&f.unique)).unwrap_or("");
        self.set_input(&self.inp.dtb.clone(), dtb, window, cx);
        self.sys.boot = BootChoice::Auto;
        self.goal = Goal::Install;
        let phrase = confirm_phrase(&f);
        self.inp.typed.update(cx, |s, cx| s.set_placeholder(phrase, window, cx));
        self.facts = Some(f);
        self.inspect_raw = raw;
        self.rebuild_plan();
        if let Some(plan) = &self.form.plan {
            let max_copies = self.facts.as_ref().map(|f| f.candidate_disks().len()).unwrap_or(1).max(1);
            let _ = (plan, max_copies);
        }
        self.reset_copies_slider(window, cx);
        if navigate {
            self.go(Screen::Inspect, window, cx);
        }
    }

    fn reset_copies_slider(&mut self, _window: &mut Window, cx: &mut Context<App>) {
        let n = self.facts.as_ref().map(|f| f.candidate_disks().len()).unwrap_or(1).max(2) as f32;
        let v = self.form.copies as f32;
        let sl = cx.new(|_| SliderState::new().min(1.0).max(n).step(1.0).default_value(v));
        self.subs.push(cx.subscribe(&sl, |this: &mut App, _, ev: &SliderEvent, cx| {
            if let SliderEvent::Change(v) = ev {
                this.form.copies = v.start().round().max(1.0) as usize;
                this.rebuild_plan();
                cx.notify();
            }
        }));
        self.form.copies_slider = sl;
    }

    // ------------------------------------------------------------------------------------------ plan

    /// Recompute the plan from the current form (cheap; runs on every change).
    pub fn rebuild_plan(&mut self) {
        let Some(f) = &self.facts else { return };
        let form = &mut self.form;
        form.req.system_size_mib = (form.system_gib.max(0.0) as u64) * 1024;
        form.req.system_copies = if form.copies_auto { None } else { Some(form.copies) };
        form.req.keep_arrays = if form.req.mode == StorageMode::KeepVolumes { Some(form.keep.iter().cloned().collect()) } else { None };
        form.plan = Some(build_plan(f, &form.req));
    }

    /// Bring the system-size slider's range in line with the plan (the range depends on mode and copies).
    pub fn sync_system_slider(&mut self, cx: &mut Context<App>) {
        let Some(plan) = &self.form.plan else { return };
        let (min, max, rec) = (
            (plan.sizes.min_system_mib as f32 / 1024.0).ceil(),
            (plan.sizes.max_system_mib as f32 / 1024.0).floor(),
            (plan.sizes.recommended_system_mib as f32 / 1024.0).round(),
        );
        if max < min || (min, max) == self.form.system_range {
            return;
        }
        if self.form.system_gib == 0.0 {
            self.form.system_gib = rec;
        }
        self.form.system_gib = self.form.system_gib.clamp(min, max);
        let v = self.form.system_gib;
        let sl = cx.new(|_| SliderState::new().min(min).max(max).step(1.0).default_value(v));
        self.subs.push(cx.subscribe(&sl, |this: &mut App, _, ev: &SliderEvent, cx| {
            if let SliderEvent::Change(v) = ev {
                this.form.system_gib = v.start().round();
                this.rebuild_plan();
                cx.notify();
            }
        }));
        self.form.system_slider = sl;
        self.form.system_range = (min, max);
        self.rebuild_plan();
    }

    pub fn set_system_gib(&mut self, gib: f32, window: &mut Window, cx: &mut Context<App>) {
        self.form.system_gib = gib;
        let sl = self.form.system_slider.clone();
        sl.update(cx, |s, cx| s.set_value(gib, window, cx));
        self.rebuild_plan();
        cx.notify();
    }

    pub fn plan_ok(&self) -> bool {
        let Some(f) = &self.facts else { return false };
        let supported = platform::classify(&f.unique, &f.uname_m).support != Support::Unsupported;
        self.form.plan.as_ref().is_some_and(|p| p.is_ok()) && supported
    }

    // ------------------------------------------------------------------------------------------ system / install

    /// `ERASE <MODEL>`, with the serial tail added when a twin is on the network.
    pub fn phrase(&self, cx: &gpui_kit::App) -> String {
        self.facts
            .as_ref()
            .map(|f| super::facts::confirm_phrase_for(&confirm_phrase(f), &f.serial, self.twin_of_selected(cx).is_some()))
            .unwrap_or_default()
    }

    pub fn gate<'a>(&'a self, typed: &'a str, phrase: &'a str) -> ConfirmGate<'a> {
        ConfirmGate {
            phrase,
            typed,
            ack_backup: self.ack_backup,
            ack_loss: self.ack_loss,
            ack_unit: self.ack_unit,
            plan_ok: self.plan_ok(),
        }
    }

    pub fn build_job(&mut self, cx: &gpui_kit::App) -> Result<InstallJob, String> {
        let facts = self.facts.clone().ok_or("No NAS has been inspected.")?;
        let plan = self.form.plan.clone().ok_or("There is no plan.")?;
        let password = self.input_value(&self.inp.new_password, cx);
        let hash = hash_password(&password).map_err(|e| e.to_string())?;
        let mut cfg =
            InstallConfig::new(self.input_value(&self.inp.hostname, cx).trim(), self.input_value(&self.inp.username, cx).trim(), hash);
        cfg.timezone = self.input_value(&self.inp.timezone, cx).trim().to_string();
        cfg.mirror_host = self.input_value(&self.inp.mirror, cx).trim().to_string();
        cfg.enable_mgmt = self.sys.enable_mgmt;
        cfg.kernel = self.form.kernel;
        cfg.serial = serial_from_input(&self.input_value(&self.inp.serial_text, cx), &self.input_value(&self.inp.serial_speed, cx))
            .ok_or("The serial console setting is not valid (for example ttyS0 and 115200).")?;
        let dtb = self.input_value(&self.inp.dtb, cx);
        cfg.dtb = Some(dtb.trim().to_string()).filter(|d| !d.is_empty());
        match self.sys.key_mode {
            KeyMode::Generate => {
                let kp = generate_ssh_keypair(&format!("synofurb-{}", cfg.hostname)).map_err(|e| e.to_string())?;
                let dir = self.cfg_dir.join("keys");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(format!("{}_ed25519", cfg.hostname));
                std::fs::write(&path, &kp.private_openssh).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                cfg.ssh_authorized_keys.push(kp.public_openssh.clone());
                self.sys.generated_key = Some((kp, path));
            }
            KeyMode::Paste => cfg.ssh_authorized_keys.push(self.input_value(&self.inp.pubkey, cx).trim().to_string()),
            KeyMode::None => {}
        }
        cfg.validate().map_err(|e| e.to_string())?;
        let backup_dir = self.cfg_dir.join("dom-backups");
        let method = match self.sys.boot.resolve(&facts) {
            BootChoice::Dom => BootMethod::Dom { agent_dir: None, backup_dir },
            BootChoice::Kexec => {
                let k = self.input_value(&self.inp.kexec_path, cx);
                BootMethod::Kexec { kexec_bin: Some(k.trim()).filter(|s| !s.is_empty()).map(PathBuf::from) }
            }
            _ => BootMethod::Kit { dir: PathBuf::from(self.input_value(&self.inp.kit_dir, cx).trim()) },
        };
        Ok(InstallJob {
            target: self.target(cx)?,
            facts,
            plan,
            cfg,
            method,
            cache_dir: self.cfg_dir.join("cache"),
            with_firmware: self.sys.with_firmware,
            verbose_installer: self.sys.verbose,
            display_password: password,
            offline_bundle: if self.source_offline { Some(PathBuf::from(self.input_value(&self.inp.bundle, cx).trim())) } else { None },
        })
    }

    /// Start the installation. Only reachable through the confirm gate; re-checks the gate itself.
    pub fn start_install(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let typed = self.input_value(&self.inp.typed, cx);
        let phrase = self.phrase(cx);
        if !self.gate(&typed, &phrase).is_open() {
            self.error = Some("The confirmation is not complete.".into());
            cx.notify();
            return;
        }
        if self.demo.is_some() {
            self.install.reset();
            self.install_started = true;
            self.result = None;
            self.go(Screen::Install, window, cx);
            let unit = self.demo.unwrap();
            let model = self.facts.as_ref().map(|f| f.model.clone()).unwrap_or_default();
            demo::spawn_script(self.tx.clone(), Script::Succeeds, unit, model, Some(demo::outcome(unit, "nas", "nasadmin")));
            return;
        }
        let job = match self.build_job(cx) {
            Ok(j) => j,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        self.error = None;
        self.install.reset();
        self.result = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        match worker::spawn_install(self.tx.clone(), job, &self.cfg_dir.join("logs"), self.cancel.clone()) {
            Ok(logger) => {
                self.logger_path = logger.path().map(|p| p.to_path_buf());
                self.install_started = true;
                self.go(Screen::Install, window, cx);
            }
            Err(e) => self.error = Some(e),
        }
        cx.notify();
    }

    pub fn cancel_install(&mut self, cx: &mut Context<App>) {
        if self.install.can_cancel() {
            self.cancel.store(true, Ordering::Relaxed);
            self.install.log.push(synofurb_core::logs::LogLine {
                ts: String::new(),
                level: synofurb_core::logs::Level::Warn,
                source: synofurb_core::logs::Source::App,
                msg: "Cancel requested: the installer stops at the next safe point.".into(),
            });
            cx.notify();
        }
    }

    pub fn start_over(&mut self, window: &mut Window, cx: &mut Context<App>) {
        self.result = None;
        self.install.reset();
        self.install_started = false;
        self.ack_backup = false;
        self.ack_loss = false;
        self.ack_unit = false;
        self.password_copied = false;
        self.sys.generated_key = None;
        self.reveal_password = false;
        self.reset_unit_session();
        self.set_input(&self.inp.typed.clone(), "", window, cx);
        self.set_input(&self.inp.new_password.clone(), &generate_password(20), window, cx);
        self.go(Screen::Inventory, window, cx);
        self.start_scan(cx);
    }

    /// Who the unit on screen is, for matching its saved flash images (the serial number is what separates two units of one model).
    pub fn unit_id(&self, cx: &gpui_kit::App) -> synofurb_core::domboot::UnitIdentity {
        let (model, serial) = self.unit_identity(cx);
        synofurb_core::domboot::UnitIdentity { serial, model, host: self.active_host(cx) }
    }

    /// `(model, serial)` of the unit the current page is about (the managed unit on Manage, else the inspected one).
    pub fn unit_identity(&self, _cx: &gpui_kit::App) -> (String, String) {
        if self.manage_bound() {
            return self.manage_unit.as_ref().map(|u| (u.model.clone(), u.serial.clone())).unwrap_or_default();
        }
        self.facts.as_ref().map(|f| (f.model.clone(), f.serial.clone())).unwrap_or_default()
    }

    /// What the installer of the planned install does after a failure (boot-flash units reboot, the rest halt).
    pub fn fail_clock(&self) -> super::facts::FailClock {
        super::facts::fail_clock(self.form.plan.as_ref().map(synofurb_core::script::failure_handling), self.install.past_no_return)
    }

    /// The name the unit's agent key is kept under: its serial number, else the address.
    pub fn unit_key_name(&self, cx: &gpui_kit::App) -> String {
        match self.facts.as_ref().filter(|_| !self.manage_bound()) {
            Some(f) if !f.serial.is_empty() => f.serial.clone(),
            _ => match &self.manage_unit {
                Some(u) if !u.serial.is_empty() => u.serial.clone(),
                _ => self.active_host(cx),
            },
        }
    }

    /// Run a recovery action on the unit (the engine checks and reads the unit back). Demo: a scripted result.
    pub fn start_recovery(&mut self, kind: super::facts::RecKind, action: worker::RecAction, cx: &mut Context<App>) {
        if self.rec_running.is_some() {
            return;
        }
        self.rec_result = None;
        self.rec_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if self.demo.is_some() {
            self.rec_running = Some(kind);
            let (title, detail) = match &action {
                worker::RecAction::BootDsm { reboot } => {
                    (if *reboot { "Reboot requested" } else { "Default set to DSM" }, "Demo: nothing was sent to a unit.")
                }
                worker::RecAction::Restore { .. } => ("Boot flash restored", "Demo: nothing was written."),
                worker::RecAction::Remove => ("Our entries are removed", "Demo: nothing was removed."),
                worker::RecAction::Agent => ("The agent answers", "Demo: no unit was rebooted."),
            };
            let out = worker::RecOutcome {
                level: worker::RecLevel::Done,
                title: title.into(),
                detail: detail.into(),
                commands: vec![],
                state: None,
            };
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1200));
                let _ = tx.unbounded_send(Msg::RecoveryDone(Box::new(out)));
            });
            cx.notify();
            return;
        }
        match self.target(cx) {
            Ok(t) => {
                self.rec_running = Some(kind);
                let unit = self.unit_key_name(cx);
                worker::spawn_recovery(self.tx.clone(), self.unit_gen, t, unit, action, self.rec_cancel.clone());
            }
            Err(e) => {
                self.rec_result = Some(super::screens::recover::RecResult::error("Not done", e));
            }
        }
        cx.notify();
    }

    /// Read the unit's boot state (Recovery opens, and after a recovery action). Demo and unreachable units read nothing.
    pub fn read_unit_state(&mut self, cx: &mut Context<App>) {
        if self.demo.is_some() || self.unit_state_loading {
            return;
        }
        match self.target(cx) {
            Ok(t) => {
                self.unit_state_loading = true;
                let unit = self.unit_key_name(cx);
                worker::spawn_unit_state_for(self.tx.clone(), self.unit_gen, t, unit);
            }
            Err(e) => self.unit_state_err = Some(e),
        }
        cx.notify();
    }

    /// "Wait for DSM": probe the unit's address until DSM answers or `DSM_WAIT_MIN` minutes pass. Demo: a scripted wait.
    pub fn start_wait_dsm(&mut self, window: &mut Window, cx: &mut Context<App>) {
        if matches!(self.dsm_wait, DsmWaitState::Waiting { .. }) {
            return;
        }
        self.dsm_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.dsm_wait = DsmWaitState::Waiting { secs: 0, last: None };
        if self.demo.is_some() {
            cx.spawn_in(window, async move |this, cx| {
                for secs in [2u64, 4, 6] {
                    cx.background_executor().timer(std::time::Duration::from_millis(900)).await;
                    let _ = this.update(cx, |t, cx| {
                        t.dsm_wait = DsmWaitState::Waiting { secs, last: Some(synofurb_core::recovery::ProbeResult::Unreachable) };
                        cx.notify();
                    });
                }
                let _ = this.update(cx, |t, cx| {
                    t.dsm_wait = DsmWaitState::Done {
                        last: synofurb_core::recovery::ProbeResult::DsmBack { ssh_banner: "SSH-2.0-OpenSSH_8.2".into(), web_port: 5000 },
                        timed_out: false,
                        secs: 8,
                    };
                    cx.notify();
                });
            })
            .detach();
        } else {
            let host = self.active_host(cx);
            let host = if host.is_empty() { self.selected_ip.clone().unwrap_or_default() } else { host };
            worker::spawn_wait_dsm(self.tx.clone(), self.unit_gen, host, self.dsm_cancel.clone());
        }
        cx.notify();
    }

    pub fn start_boot_manager(&mut self, cx: &mut Context<App>) {
        if self.bootmgr.running() {
            return;
        }
        if self.demo.is_some() {
            self.bootmgr = Job::Done(Ok(()));
            return;
        }
        let hostname = self.input_value(&self.inp.hostname, cx).trim().to_string();
        let arm = self.facts.as_ref().is_some_and(|f| f.arch.is_arm());
        match self.target(cx) {
            Ok(t) => {
                self.bootmgr = Job::Running;
                worker::spawn_boot_manager(
                    self.tx.clone(),
                    self.unit_gen,
                    t,
                    hostname,
                    arm,
                    self.cfg_dir.join("dom-backups"),
                    self.cfg_dir.join("logs"),
                );
            }
            Err(e) => self.bootmgr = Job::Done(Err(e)),
        }
        cx.notify();
    }

    // ------------------------------------------------------------------------------------------ settings / updates

    pub fn refresh_compat(&mut self, cx: &mut Context<App>) {
        if self.compat_refreshing {
            return;
        }
        if self.demo.is_some() {
            self.compat_note = Some(("Demo mode: the list is not downloaded.".into(), true));
            cx.notify();
            return;
        }
        self.compat_refreshing = true;
        worker::spawn_compat_refresh(self.tx.clone(), self.cfg_dir.clone());
        cx.notify();
    }

    pub fn channel(&self) -> Channel {
        self.settings.update_channel.parse().unwrap_or_default()
    }

    pub fn maybe_check_updates(&mut self) {
        if self.settings.check_updates && !matches!(self.update, UpdateState::Checking) {
            self.update = UpdateState::Checking;
            worker::spawn_update_check(self.tx.clone(), version(), self.channel(), self.cfg_dir.join("updates"));
        }
    }

    pub fn check_updates_now(&mut self, cx: &mut Context<App>) {
        if self.demo.is_some() {
            self.update = UpdateState::Available(Box::new(demo::update_info()));
        } else {
            self.update = UpdateState::Checking;
            worker::spawn_update_check(self.tx.clone(), version(), self.channel(), self.cfg_dir.join("updates"));
        }
        cx.notify();
    }

    pub fn update_info(&self) -> Option<&UpdateInfo> {
        match &self.update {
            UpdateState::Available(i) | UpdateState::Downloading(i, _) | UpdateState::Verified(i, _) | UpdateState::Failed(i, _) => Some(i),
            _ => None,
        }
    }

    pub fn start_update_download(&mut self, cx: &mut Context<App>) {
        let Some(info) = self.update_info().cloned() else { return };
        if self.demo.is_some() {
            self.update = UpdateState::Failed(
                Box::new(info),
                "Demo mode: nothing is downloaded. In a real run the file is checked against the project's signature and SHA-256 before it can be installed.".into(),
            );
            cx.notify();
            return;
        }
        self.update =
            UpdateState::Downloading(Box::new(info.clone()), Progress { downloaded: 0, total: info.asset.as_ref().map(|a| a.size) });
        worker::spawn_update_download(self.tx.clone(), info.clone(), self.cfg_dir.join("updates").join("downloads").join(&info.latest));
        cx.notify();
    }

    pub fn remind_tomorrow(&mut self, cx: &mut Context<App>) {
        self.settings.remind_after = Some(now_unix() + 24 * 3600);
        self.save_settings();
        self.update = UpdateState::UpToDate;
        cx.notify();
    }

    pub fn skip_version(&mut self, cx: &mut Context<App>) {
        if let Some(i) = self.update_info() {
            self.settings.skip_version = Some(i.latest.clone());
        }
        self.save_settings();
        self.update = UpdateState::UpToDate;
        cx.notify();
    }

    /// Apply a verified update. Never while an install is running.
    pub fn apply_update(&mut self, cx: &mut Context<App>) {
        if self.install_running() {
            return;
        }
        let UpdateState::Verified(info, vd) = &self.update else { return };
        match synofurb_core::updater::apply(&vd.path) {
            Ok(synofurb_core::updater::ApplyOutcome::InstallerLaunched) => cx.quit(),
            Ok(o) => {
                let msg = match o {
                    synofurb_core::updater::ApplyOutcome::Opened => {
                        "The installer was opened. Finish the installation there, then start synofurb again.".to_string()
                    }
                    synofurb_core::updater::ApplyOutcome::Replaced { target, backup } => {
                        format!(
                            "Updated {} to {}. The previous version is kept at {}. Start synofurb again.",
                            target.display(),
                            info.latest,
                            backup.display()
                        )
                    }
                    synofurb_core::updater::ApplyOutcome::PackageInstalled { command } => format!("Installed with: {command}"),
                    synofurb_core::updater::ApplyOutcome::PackageCommand { command } => {
                        format!("This is a system package. Run it yourself: {command}")
                    }
                    synofurb_core::updater::ApplyOutcome::Manual { reason } => reason,
                    synofurb_core::updater::ApplyOutcome::InstallerLaunched => String::new(),
                };
                self.compat_note = Some((msg, true));
            }
            Err(e) => self.update = UpdateState::Failed(info.clone(), e.to_string()),
        }
        cx.notify();
    }

    /// True while an installation is in progress (started and not finished): blocks updates and navigation away.
    pub fn install_running(&self) -> bool {
        self.install_started && self.result.is_none() && matches!(self.screen, Screen::Install)
    }

    // ------------------------------------------------------------------------------------------ files

    /// Ask the OS for a file, then put its path into `target`.
    pub fn browse_file(&mut self, target: Entity<InputState>, window: &mut Window, cx: &mut Context<App>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.first()
            {
                let s = p.display().to_string();
                let _ = this.update_in(cx, |this, window, cx| this.set_input(&target, &s, window, cx));
            }
        })
        .detach();
    }

    pub fn browse_dir(&mut self, target: Entity<InputState>, window: &mut Window, cx: &mut Context<App>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.first()
            {
                let s = p.display().to_string();
                let _ = this.update_in(cx, |this, window, cx| this.set_input(&target, &s, window, cx));
            }
        })
        .detach();
    }

    /// Text of the logins summary written by "Save logins to a file".
    pub fn logins_text(&self, o: &Outcome, password: &str) -> String {
        let ip = o.ip.map(|i| i.to_string()).unwrap_or_default();
        format!(
            "host: {}\naddress: {}\nuser: {}\npassword: {}\nssh: ssh {}@{}\nssh host key: {}\nmanagement: {}\n",
            o.hostname,
            ip,
            o.username,
            password,
            o.username,
            ip,
            o.ssh_fingerprint.clone().unwrap_or_default(),
            o.mgmt_url.clone().unwrap_or_default()
        )
    }

    pub fn save_logins(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let Some(Ok(o)) = self.result.clone() else { return };
        let password = self.input_value(&self.inp.new_password, cx);
        let text = self.logins_text(&o, &password);
        let rx = cx.prompt_for_new_path(&self.cfg_dir, Some("nas-logins.txt"));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let ok = std::fs::write(&path, text).is_ok();
                #[cfg(unix)]
                if ok {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                let _ = this.update_in(cx, |this, window, cx| {
                    this.toast(window, cx, if ok { "Logins saved (readable only by you)" } else { "Could not save the file" }, ok)
                });
            }
        })
        .detach();
    }

    pub fn password_input(&self) -> Input {
        Input::new(&self.inp.password).mask_toggle()
    }
}
