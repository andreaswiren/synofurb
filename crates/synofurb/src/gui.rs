//! The graphical wizard (egui). Thin on purpose: all decisions live in `synofurb-core`.

use crate::cli::config_dir;
use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use synofurb_core::config::*;
use synofurb_core::discovery::{self, Inventory, ScanOptions};
use synofurb_core::logs::{Event, Level, LogLine, Logger, Phase};
use synofurb_core::model::*;
use synofurb_core::plan::*;
use synofurb_core::platform::{self, Support};
use synofurb_core::remote::{Auth, SshSession};
use synofurb_core::session::*;

#[derive(PartialEq, Clone, Copy)]
enum Screen {
    Inventory,
    Connect,
    Layout,
    Account,
    Confirm,
    Install,
    Done,
}

const STEPS: [(Screen, &str); 7] = [
    (Screen::Inventory, "1 Find NAS"),
    (Screen::Connect, "2 Connect"),
    (Screen::Layout, "3 Storage"),
    (Screen::Account, "4 System"),
    (Screen::Confirm, "5 Confirm"),
    (Screen::Install, "6 Install"),
    (Screen::Done, "7 Done"),
];

enum Msg {
    Scanned(Inventory),
    Inspected(Result<(Box<DeviceFacts>, String), String>),
    Ev(Event),
    Finished(Result<Outcome, String>),
}

pub struct App {
    screen: Screen,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    // inventory
    inv: Inventory,
    scanning: bool,
    manual_ip: String,
    // connect
    host: String,
    port: String,
    user: String,
    password: String,
    key_path: String,
    use_key: bool,
    sudo_password: String,
    inspecting: bool,
    facts: Option<DeviceFacts>,
    inspect_raw: String,
    error: Option<String>,
    // layout
    req: PlanRequest,
    system_gib: f32,
    copies_auto: bool,
    copies: usize,
    keep: BTreeSet<String>,
    plan: Option<Plan>,
    // account
    hostname: String,
    username: String,
    new_password: String,
    show_password: bool,
    key_mode: KeyMode,
    pubkey_text: String,
    timezone: String,
    mirror: String,
    enable_mgmt: bool,
    with_firmware: bool,
    verbose: bool,
    serial_tty: String,
    serial_speed: String,
    dtb: String,
    method_kit: bool,
    kexec_path: String,
    kit_dir: String,
    // confirm
    ack_backup: bool,
    ack_loss: bool,
    typed: String,
    // install
    phase: Option<Phase>,
    step: (u32, u32, String),
    progress: Option<(String, f32)>,
    log: Vec<LogLine>,
    /// U-Boot commands the installer printed for ARM boards (shown prominently: the user must act on the serial console)
    uboot_lines: Vec<String>,
    show_debug: bool,
    cancel: Arc<AtomicBool>,
    past_no_return: bool,
    logger: Option<Logger>,
    result: Option<Result<Outcome, String>>,
    generated_key: Option<(SshKeypair, PathBuf)>,
}

#[derive(PartialEq, Clone, Copy)]
enum KeyMode {
    Generate,
    Paste,
    None,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, demo: Option<&str>) -> App {
        let (tx, rx) = channel();
        let inv = Inventory::load(&config_dir().join("inventory.json"));
        let mut app = App {
            screen: Screen::Inventory,
            tx,
            rx,
            inv,
            scanning: false,
            manual_ip: String::new(),
            host: String::new(),
            port: "22".into(),
            user: "admin".into(),
            password: String::new(),
            key_path: String::new(),
            use_key: false,
            sudo_password: String::new(),
            inspecting: false,
            facts: None,
            inspect_raw: String::new(),
            error: None,
            req: PlanRequest::default(),
            system_gib: 0.0,
            copies_auto: true,
            copies: 2,
            keep: BTreeSet::new(),
            plan: None,
            hostname: "nas".into(),
            username: "nasadmin".into(),
            new_password: generate_password(20),
            show_password: false,
            key_mode: KeyMode::Generate,
            pubkey_text: String::new(),
            timezone: "UTC".into(),
            mirror: "deb.debian.org".into(),
            enable_mgmt: true,
            with_firmware: true,
            verbose: true,
            serial_tty: "ttyS0".into(),
            serial_speed: "115200".into(),
            dtb: String::new(),
            method_kit: false,
            kexec_path: String::new(),
            kit_dir: config_dir().join("kit").display().to_string(),
            ack_backup: false,
            ack_loss: false,
            typed: String::new(),
            phase: None,
            step: (0, 0, String::new()),
            progress: None,
            log: vec![],
            uboot_lines: vec![],
            show_debug: true,
            cancel: Arc::new(AtomicBool::new(false)),
            past_no_return: false,
            logger: None,
            result: None,
            generated_key: None,
        };
        if let Some(screen) = demo {
            let raw = synofurb_core::inspect::fixtures::ds_4bay();
            let facts = synofurb_core::inspect::parse_inspect(&raw).expect("fixture parses");
            app.host = "192.168.1.20 (demo)".into();
            app.on_inspected(facts, raw);
            app.rebuild_plan();
            app.screen = match screen {
                "account" => Screen::Account,
                "confirm" => Screen::Confirm,
                _ => Screen::Layout,
            };
        }
        app
    }

    fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Scanned(inv) => {
                    let _ = inv.save(&config_dir().join("inventory.json"));
                    self.inv = inv;
                    self.scanning = false;
                }
                Msg::Inspected(r) => {
                    self.inspecting = false;
                    match r {
                        Ok((f, raw)) => self.on_inspected(*f, raw),
                        Err(e) => self.error = Some(e),
                    }
                }
                Msg::Ev(e) => match e {
                    Event::Log(l) => {
                        if let Some((_, cmd)) = l.msg.split_once("uboot-env| ") {
                            self.uboot_lines.push(cmd.to_string());
                        }
                        if self.log.len() > 50_000 {
                            self.log.drain(..10_000);
                        }
                        self.log.push(l)
                    }
                    Event::Phase(p) => self.phase = Some(p),
                    Event::Step { n, total, name } => self.step = (n, total, name),
                    Event::Progress { what, fraction } => self.progress = Some((what, fraction)),
                    Event::PointOfNoReturn => self.past_no_return = true,
                    Event::InstallerFailed(_) | Event::InstallerDone => {}
                },
                Msg::Finished(r) => {
                    self.result = Some(r);
                    self.screen = Screen::Done;
                }
            }
        }
    }

    fn on_inspected(&mut self, f: DeviceFacts, raw: String) {
        self.error = None;
        self.req = PlanRequest::default();
        self.keep = f.data_arrays().iter().map(|a| a.name.clone()).collect();
        self.copies = default_system_copies(f.candidate_disks().len(), f.candidate_disks().len()).max(1);
        self.serial_tty = f.serial_console().tty;
        self.serial_speed = f.serial_console().speed.to_string();
        self.dtb = platform::mainline_dtb(&f.model).or_else(|| platform::mainline_dtb(&f.unique)).unwrap_or("").to_string();
        if !f.hostname.is_empty() && self.hostname == "nas" {
            self.hostname = "nas".into();
        }
        self.facts = Some(f);
        self.inspect_raw = raw;
        self.system_gib = 0.0;
        self.screen = Screen::Layout;
    }

    fn rebuild_plan(&mut self) {
        let Some(f) = &self.facts else { return };
        self.req.system_size_mib = (self.system_gib as u64) * 1024;
        self.req.system_copies = if self.copies_auto { None } else { Some(self.copies) };
        self.req.keep_arrays = if self.req.mode == StorageMode::KeepVolumes { Some(self.keep.iter().cloned().collect()) } else { None };
        self.plan = Some(build_plan(f, &self.req));
    }

    fn target(&self) -> Result<SshTarget, String> {
        let auth = if self.use_key {
            Auth::Key(std::fs::read_to_string(&self.key_path).map_err(|e| format!("cannot read key file: {e}"))?)
        } else {
            Auth::Password(self.password.clone())
        };
        let sudo = if self.sudo_password.is_empty() { Some(self.password.clone()) } else { Some(self.sudo_password.clone()) };
        Ok(SshTarget {
            host: self.host.trim().to_string(),
            port: self.port.parse().map_err(|_| "invalid port")?,
            user: self.user.clone(),
            auth,
            sudo_password: sudo.filter(|s| !s.is_empty()),
        })
    }

    fn start_inspect(&mut self, ctx: &egui::Context) {
        let t = match self.target() {
            Ok(t) => t,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.error = None;
        self.inspecting = true;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let log = Logger::memory(None);
            let r = (|| -> anyhow::Result<_> {
                let mut ssh = SshSession::connect(&t.host, t.port, &t.user, &t.auth, &log)?;
                let r = inspect_remote(&mut ssh, false, t.sudo_password.clone(), &log);
                ssh.close();
                r
            })();
            let _ = tx.send(Msg::Inspected(r.map(|(f, raw)| (Box::new(f), raw)).map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
    }

    fn build_job(&mut self) -> Result<InstallJob, String> {
        let facts = self.facts.clone().ok_or("no NAS inspected")?;
        let plan = self.plan.clone().ok_or("no plan")?;
        let hash = hash_password(&self.new_password).map_err(|e| e.to_string())?;
        let mut cfg = InstallConfig::new(self.hostname.trim(), self.username.trim(), hash);
        cfg.timezone = self.timezone.clone();
        cfg.mirror_host = self.mirror.clone();
        cfg.enable_mgmt = self.enable_mgmt;
        cfg.serial = SerialConsole { tty: self.serial_tty.clone(), speed: self.serial_speed.parse().map_err(|_| "invalid serial speed")? };
        cfg.dtb = Some(self.dtb.trim().to_string()).filter(|d| !d.is_empty());
        match self.key_mode {
            KeyMode::Generate => {
                let kp = generate_ssh_keypair(&format!("synofurb-{}", cfg.hostname)).map_err(|e| e.to_string())?;
                let dir = config_dir().join("keys");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(format!("{}_ed25519", cfg.hostname));
                std::fs::write(&path, &kp.private_openssh).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                cfg.ssh_authorized_keys.push(kp.public_openssh.clone());
                self.generated_key = Some((kp, path));
            }
            KeyMode::Paste => cfg.ssh_authorized_keys.push(self.pubkey_text.trim().to_string()),
            KeyMode::None => {}
        }
        cfg.validate().map_err(|e| e.to_string())?;
        Ok(InstallJob {
            target: self.target()?,
            facts,
            plan,
            cfg,
            method: if self.method_kit {
                BootMethod::Kit { dir: self.kit_dir.clone().into() }
            } else {
                BootMethod::Kexec { kexec_bin: Some(self.kexec_path.trim()).filter(|s| !s.is_empty()).map(PathBuf::from) }
            },
            cache_dir: config_dir().join("cache"),
            with_firmware: self.with_firmware,
            verbose_installer: self.verbose,
            display_password: self.new_password.clone(),
        })
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        let job = match self.build_job() {
            Ok(j) => j,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.error = None;
        self.log.clear();
        self.uboot_lines.clear();
        self.past_no_return = false;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.screen = Screen::Install;
        let (ev_tx, ev_rx) = channel::<Event>();
        let logger = match Logger::to_dir(&config_dir().join("logs"), Some(ev_tx)) {
            Ok(l) => l,
            Err(e) => {
                self.error = Some(format!("cannot create log file: {e}"));
                return;
            }
        };
        self.logger = Some(logger.clone());
        let (tx, ctx2) = (self.tx.clone(), ctx.clone());
        std::thread::spawn({
            let (tx, ctx) = (tx.clone(), ctx2.clone());
            move || {
                for e in ev_rx {
                    let _ = tx.send(Msg::Ev(e));
                    ctx.request_repaint();
                }
            }
        });
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let lg = logger.clone();
            let r = run_install(&job, &logger, cancel, &mut |what, f| lg.emit(Event::Progress { what: what.to_string(), fraction: f }));
            let _ = tx.send(Msg::Finished(r.map_err(|e| format!("{e:#}"))));
            ctx2.request_repaint();
        });
    }

    // ---------------------------------------------------------------- screens

    fn screen_inventory(&mut self, ui: &mut egui::Ui) {
        ui.heading("Find your Synology NAS");
        ui.label("Scans the network the way Synology's own tools do: Synology Assistant broadcast, Bonjour/mDNS, UPnP/SSDP, the DSM web API on port 5000, and Synology MAC prefixes.");
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.scanning, egui::Button::new(if self.scanning { "Scanning..." } else { "Scan network" })).clicked() {
                self.scanning = true;
                let (tx, ctx) = (self.tx.clone(), ui.ctx().clone());
                std::thread::spawn(move || {
                    let inv = discovery::scan(&ScanOptions::default(), &Logger::memory(None));
                    let _ = tx.send(Msg::Scanned(inv));
                    ctx.request_repaint();
                });
            }
            ui.label("or enter an address:");
            ui.add(egui::TextEdit::singleline(&mut self.manual_ip).desired_width(160.0).hint_text("192.168.1.20"));
            if ui.button("Use this address").clicked() && !self.manual_ip.trim().is_empty() {
                self.host = self.manual_ip.trim().to_string();
                self.screen = Screen::Connect;
            }
        });
        ui.separator();
        let mut pick = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            let devs: Vec<_> = self.inv.devices.iter().filter(|d| d.synology || d.synofurb_debian).collect();
            if devs.is_empty() {
                ui.label("No Synology devices known yet. Click 'Scan network'.");
            }
            for d in devs {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(d.title()).strong());
                            ui.label(format!("MAC {}   DSM {}   serial {}", d.mac.clone().unwrap_or_default(), d.dsm_version, d.serial));
                            ui.label(RichText::new(format!("seen via {}  at {}", d.sources.join(", "), d.last_seen)).weak());
                            if d.synofurb_debian {
                                ui.label(RichText::new("Already runs Debian installed by synofurb").color(Color32::LIGHT_GREEN));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !d.synofurb_debian && ui.button("Select").clicked() {
                                pick = d.primary_ip().map(|i| i.to_string());
                            }
                        });
                    });
                });
            }
        });
        if let Some(h) = pick {
            self.host = h;
            self.screen = Screen::Connect;
        }
    }

    fn screen_connect(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connect to DSM");
        ui.label("synofurb needs SSH access to read the disk layout (read-only) and, later, to start the installer. In DSM: Control Panel > Terminal & SNMP > Enable SSH service. Use an administrator account; DSM 7 admins get root through sudo.");
        egui::Grid::new("conn").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Address");
            ui.text_edit_singleline(&mut self.host);
            ui.end_row();
            ui.label("SSH port");
            ui.add(egui::TextEdit::singleline(&mut self.port).desired_width(60.0));
            ui.end_row();
            ui.label("User");
            ui.text_edit_singleline(&mut self.user);
            ui.end_row();
            ui.label("Login with");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.use_key, false, "password");
                ui.radio_value(&mut self.use_key, true, "private key file");
            });
            ui.end_row();
            if self.use_key {
                ui.label("Key file");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.key_path);
                    if ui.button("Browse...").clicked()
                        && let Some(p) = rfd::FileDialog::new().pick_file()
                    {
                        self.key_path = p.display().to_string();
                    }
                });
            } else {
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
            }
            ui.end_row();
            ui.label("sudo password");
            ui.add(egui::TextEdit::singleline(&mut self.sudo_password).password(true).hint_text("empty = same as the SSH password"));
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("< Back").clicked() {
                self.screen = Screen::Inventory;
            }
            if ui
                .add_enabled(
                    !self.inspecting && !self.host.is_empty(),
                    egui::Button::new(if self.inspecting { "Inspecting..." } else { "Connect and inspect" }),
                )
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_inspect(&ctx);
            }
        });
    }

    fn screen_layout(&mut self, ui: &mut egui::Ui) {
        let Some(f) = self.facts.clone() else { return };
        let info = platform::classify(&f.unique, &f.uname_m);
        ui.heading(format!("{} - storage layout", if f.model.is_empty() { "NAS" } else { &f.model }));
        let col = match info.support {
            Support::Supported => Color32::LIGHT_GREEN,
            Support::Experimental => Color32::YELLOW,
            Support::Unsupported => Color32::LIGHT_RED,
        };
        ui.label(
            RichText::new(format!(
                "{}  |  {}  |  {}  |  DSM {}  |  {:.1} GiB RAM  |  boots via {}",
                info.soc,
                f.arch.label(),
                f.cpu,
                f.dsm_version,
                f.mem_gib(),
                f.firmware.label()
            ))
            .weak(),
        );
        ui.label(RichText::new(format!("Platform support: {} - {}", info.support.label(), info.note)).color(col));
        if !f.kexec && !self.method_kit {
            ui.label(RichText::new("This DSM kernel has no kexec support: use the installer kit (step 4).").color(Color32::YELLOW));
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.collapsing(format!("Disks ({})", f.disks.len()), |ui| {
                for d in &f.disks {
                    ui.label(format!("{:10} {:>10}  {}  serial {}  {}  {:?}{}", d.name, d.human_size(), d.model, d.serial, if d.rotational { "HDD" } else { "SSD" }, d.table, if d.is_candidate() { "" } else { "  (not an install target)" }));
                    for p in &d.parts {
                        let arr = f.arrays.iter().find(|a| a.members.contains(&p.name)).map(|a| format!("{} ({:?})", a.name, a.role)).unwrap_or_else(|| "-".into());
                        ui.label(RichText::new(format!("      {:12} {:>10}   {}", p.name, human_bytes(p.sectors * SECTOR), arr)).weak());
                    }
                }
            });
            ui.separator();
            ui.label(RichText::new("What should happen to the existing disks?").strong());
            ui.radio_value(&mut self.req.mode, StorageMode::WipeAll, "Wipe everything - new partition tables on all disks, all current data is destroyed");
            ui.radio_value(&mut self.req.mode, StorageMode::KeepVolumes, "Keep existing data volumes - reuse only DSM's system/swap area for Debian; preserve the data arrays below (no guarantee, back up!)");
            if self.req.mode == StorageMode::KeepVolumes {
                ui.indent("keep", |ui| {
                    let arrays: Vec<_> = f.data_arrays().into_iter().cloned().collect();
                    if arrays.is_empty() {
                        ui.label("No data arrays found.");
                    }
                    for a in arrays {
                        let mut on = self.keep.contains(&a.name);
                        let txt = format!("Preserve {} - {} {}{}", a.name, a.level, human_bytes(a.sectors * SECTOR), a.vg.as_ref().map(|v| format!(", LVM {v}")).unwrap_or_default());
                        if ui.checkbox(&mut on, txt).changed() {
                            if on { self.keep.insert(a.name.clone()); } else { self.keep.remove(&a.name); }
                        }
                    }
                    ui.label(RichText::new("Preserved volumes are not mounted by Debian automatically; the management plane lists them under 'Import volumes' and mounts them read-only.").weak());
                });
            }
            ui.separator();
            self.rebuild_plan();
            let plan = self.plan.clone().unwrap();
            ui.label(RichText::new("System (Debian on LVM, xfs)").strong());
            let (min, max, rec) = (plan.sizes.min_system_mib as f32 / 1024.0, plan.sizes.max_system_mib as f32 / 1024.0, plan.sizes.recommended_system_mib as f32 / 1024.0);
            if max >= min {
                if self.system_gib == 0.0 {
                    self.system_gib = rec;
                }
                self.system_gib = self.system_gib.clamp(min, max);
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.system_gib, min..=max).suffix(" GiB").step_by(1.0));
                    if ui.button(format!("Recommended: {rec:.0} GiB")).clicked() {
                        self.system_gib = rec;
                    }
                });
                ui.label(RichText::new(format!("min {min:.0} GiB, max {max:.0} GiB on the system disks. Root gets {:.0} GiB, swap {} MiB, the rest stays free in the volume group.", plan.sizes.root_mib as f32 / 1024.0, plan.sizes.swap_mib)).weak());
            }
            let n = f.candidate_disks().len();
            ui.horizontal(|ui| {
                ui.label("System copies (RAID1 mirror across disks):");
                ui.checkbox(&mut self.copies_auto, "automatic");
                if !self.copies_auto {
                    ui.add(egui::Slider::new(&mut self.copies, 1..=n.max(1)));
                }
            });
            if n > 2 {
                ui.label(RichText::new("More than two disks: the system partition is always mirrored (at least 2 copies).").weak());
            }
            ui.horizontal(|ui| {
                ui.label("Data RAID:");
                egui::ComboBox::from_id_salt("raid").selected_text(self.req.data_raid.label()).show_ui(ui, |ui| {
                    for r in DataRaid::ALL {
                        ui.selectable_value(&mut self.req.data_raid, r, r.label());
                    }
                });
                ui.label("leave free in data VG:");
                ui.add(egui::Slider::new(&mut self.req.data_free_percent, 0..=90).suffix(" %"));
            });
            ui.separator();
            ui.label(RichText::new("Resulting layout").strong());
            for l in plan.summary() {
                ui.label(l);
            }
            for w in &plan.warnings {
                ui.label(RichText::new(format!("Warning: {w}")).color(Color32::YELLOW));
            }
            for e in &plan.errors {
                ui.label(RichText::new(format!("Error: {e}")).color(Color32::LIGHT_RED));
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("< Back").clicked() {
                self.screen = Screen::Connect;
            }
            let ok = self.plan.as_ref().is_some_and(|p| p.is_ok()) && info.support != Support::Unsupported;
            if ui.add_enabled(ok, egui::Button::new("Next >")).clicked() {
                self.screen = Screen::Account;
            }
        });
    }

    fn screen_account(&mut self, ui: &mut egui::Ui) {
        let f = self.facts.clone().unwrap();
        ui.heading("The new system");
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("acct").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Host name");
                ui.text_edit_singleline(&mut self.hostname);
                ui.end_row();
                ui.label("Default user (sudo)");
                ui.text_edit_singleline(&mut self.username);
                ui.end_row();
                ui.label("Password");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_password).password(!self.show_password));
                    ui.checkbox(&mut self.show_password, "show");
                    if ui.button("Generate").clicked() {
                        self.new_password = generate_password(20);
                    }
                });
                ui.end_row();
                ui.label("SSH key");
                ui.vertical(|ui| {
                    ui.radio_value(
                        &mut self.key_mode,
                        KeyMode::Generate,
                        "Generate a new ed25519 key pair (private key saved on this computer)",
                    );
                    ui.radio_value(&mut self.key_mode, KeyMode::Paste, "Use my existing public key");
                    ui.radio_value(&mut self.key_mode, KeyMode::None, "No key (password login only)");
                    if self.key_mode == KeyMode::Paste {
                        ui.add(egui::TextEdit::multiline(&mut self.pubkey_text).desired_rows(2).hint_text("ssh-ed25519 AAAA... me@laptop"));
                    }
                    ui.label(
                        RichText::new(
                            "SSH accepts both: key login is tried first, password login stays enabled. Root login over SSH is off.",
                        )
                        .weak(),
                    );
                });
                ui.end_row();
                ui.label("Time zone");
                ui.text_edit_singleline(&mut self.timezone);
                ui.end_row();
                ui.label("Debian mirror");
                ui.text_edit_singleline(&mut self.mirror);
                ui.end_row();
                ui.label("Serial console");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.serial_tty).desired_width(80.0));
                    ui.add(egui::TextEdit::singleline(&mut self.serial_speed).desired_width(70.0));
                    ui.label("baud (taken from DSM's kernel command line). USB serial adapters get a login prompt automatically.");
                });
                ui.end_row();
                if f.arch == Arch::Armhf {
                    ui.label("Board device tree");
                    ui.add(egui::TextEdit::singleline(&mut self.dtb).hint_text("armada-385-synology-ds116.dtb"));
                    ui.end_row();
                }
            });
            ui.checkbox(
                &mut self.enable_mgmt,
                "Install the management plane (status, users, network, volumes, mounts) on https://<ip>:8443",
            );
            ui.checkbox(&mut self.with_firmware, "Include non-free firmware in the installer (needed by some network chips)");
            ui.checkbox(&mut self.verbose, "Verbose installer debug logging (recommended)");
            ui.separator();
            ui.label(RichText::new("How should the installer be started?").strong());
            ui.radio_value(
                &mut self.method_kit,
                false,
                "Automatically, over SSH with kexec (needs a static kexec binary and kexec support in DSM's kernel)",
            );
            if !self.method_kit {
                ui.horizontal(|ui| {
                    ui.label("kexec binary:");
                    ui.add(egui::TextEdit::singleline(&mut self.kexec_path).hint_text("assets/kexec/<arch>/kexec (auto-detected)"));
                    if ui.button("Browse...").clicked()
                        && let Some(p) = rfd::FileDialog::new().pick_file()
                    {
                        self.kexec_path = p.display().to_string();
                    }
                });
            }
            ui.radio_value(
                &mut self.method_kit,
                true,
                "Prepare an installer kit (USB / TFTP / U-Boot) and boot the NAS from it myself - needed for ARM U-Boot boards",
            );
            if self.method_kit {
                ui.horizontal(|ui| {
                    ui.label("Kit folder:");
                    ui.text_edit_singleline(&mut self.kit_dir);
                });
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("< Back").clicked() {
                self.screen = Screen::Layout;
            }
            if ui.button("Next >").clicked() {
                self.screen = Screen::Confirm;
            }
        });
    }

    fn screen_confirm(&mut self, ui: &mut egui::Ui) {
        let f = self.facts.clone().unwrap();
        let plan = self.plan.clone().unwrap();
        let phrase = confirm_phrase(&f);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(RichText::new("Data loss warning").color(Color32::LIGHT_RED));
            ui.label(RichText::new("Reinstalling a NAS repartitions its disks. There is NO guarantee that any of the data currently on this NAS survives - this applies to volumes marked 'preserved' as well. A power cut, a bad sector, a bug in this tool or a mistaken choice can destroy data. Backups are not optional: before you continue, make a complete copy of everything you care about on a different device and check that you can read it back.").strong());
            ui.add_space(6.0);
            ui.label(RichText::new("Will be destroyed").strong().color(Color32::LIGHT_RED));
            for d in &plan.destroyed {
                ui.label(format!("  -  {d}"));
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Will be preserved (best effort, still not guaranteed)").strong().color(Color32::LIGHT_GREEN));
            if plan.preserved.is_empty() {
                ui.label("  (nothing)");
            }
            for d in &plan.preserved {
                ui.label(format!("  -  {}: {}", d.what, d.detail));
            }
            ui.add_space(4.0);
            ui.label(RichText::new("New layout").strong());
            for l in plan.summary() {
                ui.label(format!("  {l}"));
            }
            for w in &plan.warnings {
                ui.label(RichText::new(format!("  warning: {w}")).color(Color32::YELLOW));
            }
            ui.separator();
            ui.checkbox(&mut self.ack_backup, "I have a complete, verified backup of everything on this NAS that I want to keep.");
            ui.checkbox(&mut self.ack_loss, "I understand that data can be lost, including data marked as preserved, and I accept this risk.");
            ui.label(format!("Type  {phrase}  to confirm:"));
            ui.add(egui::TextEdit::singleline(&mut self.typed).hint_text(&phrase));
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).color(Color32::LIGHT_RED));
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("< Back").clicked() {
                self.screen = Screen::Account;
            }
            let ok = self.ack_backup && self.ack_loss && self.typed.trim() == phrase && plan.is_ok();
            if ui
                .add_enabled(
                    ok,
                    egui::Button::new(RichText::new("Start installation").color(Color32::WHITE)).fill(Color32::from_rgb(170, 40, 40)),
                )
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_install(&ctx);
            }
        });
    }

    fn screen_install(&mut self, ui: &mut egui::Ui) {
        ui.heading("Installing");
        ui.label(format!("Phase: {}", self.phase.as_ref().map(|p| format!("{p:?}")).unwrap_or_default()));
        if self.step.1 > 0 {
            ui.add(
                egui::ProgressBar::new(self.step.0 as f32 / self.step.1 as f32)
                    .text(format!("step {}/{}: {}", self.step.0, self.step.1, self.step.2)),
            );
        }
        if let Some((what, f)) = &self.progress
            && *f < 1.0
        {
            ui.add(egui::ProgressBar::new(*f).text(what.as_str()));
        }
        ui.label("Keep this window open and the computer awake: the NAS fetches its installer script from here and streams its log back.");
        if self.past_no_return {
            ui.label(RichText::new("The installer has started changing the disks - do not power off the NAS.").color(Color32::LIGHT_RED));
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.past_no_return, egui::Button::new("Cancel")).clicked() {
                self.cancel.store(true, Ordering::Relaxed);
            }
            ui.checkbox(&mut self.show_debug, "show debug lines");
            if let Some(p) = self.logger.as_ref().and_then(|l| l.path()) {
                ui.label(RichText::new(format!("log file: {}", p.display())).weak());
                if ui.button("Copy log path").clicked() {
                    ui.ctx().copy_text(p.display().to_string());
                }
            }
        });
        if !self.uboot_lines.is_empty() {
            ui.group(|ui| {
                ui.label(RichText::new("ACTION NEEDED on the NAS serial console (U-Boot)").strong().color(Color32::YELLOW));
                ui.label("Debian is installed but this board's U-Boot cannot find it by itself, and DSM's system partition is gone. When the NAS reboots, interrupt U-Boot with Ctrl+C and paste these commands:");
                for l in &self.uboot_lines {
                    ui.label(RichText::new(l).monospace());
                }
                if ui.button("Copy commands").clicked() {
                    ui.ctx().copy_text(self.uboot_lines.join("\n"));
                }
            });
        }
        ui.separator();
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for l in &self.log {
                if !self.show_debug && l.level == Level::Debug {
                    continue;
                }
                let c = match l.level {
                    Level::Error => Color32::LIGHT_RED,
                    Level::Warn => Color32::YELLOW,
                    Level::Info => Color32::LIGHT_GRAY,
                    Level::Debug => Color32::GRAY,
                };
                ui.label(RichText::new(format!("{} [{}] {}", l.ts, l.source.tag(), l.msg)).monospace().color(c));
            }
        });
    }

    fn screen_done(&mut self, ui: &mut egui::Ui) {
        match self.result.clone() {
            Some(Ok(o)) => {
                ui.heading(RichText::new("Debian is installed").color(Color32::LIGHT_GREEN));
                let ip = o.ip.map(|i| i.to_string()).unwrap_or_default();
                egui::Grid::new("logins").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Host name");
                    ui.label(format!("{}  (also {}.local via mDNS)", o.hostname, o.hostname));
                    ui.end_row();
                    ui.label("Address");
                    ui.label(&ip);
                    ui.end_row();
                    ui.label("Login");
                    ui.label(format!("{} / {}", o.username, self.new_password));
                    ui.end_row();
                    ui.label("SSH");
                    ui.label(format!("ssh {}@{}", o.username, ip));
                    ui.end_row();
                    ui.label("SSH host key");
                    ui.label(o.ssh_fingerprint.clone().unwrap_or_default());
                    ui.end_row();
                    if let Some((kp, path)) = &self.generated_key {
                        ui.label("SSH private key");
                        ui.label(format!("{}   ({})   use: ssh -i {} {}@{}", path.display(), kp.fingerprint, path.display(), o.username, ip));
                        ui.end_row();
                    }
                    if let Some(u) = &o.mgmt_url {
                        ui.label("Management plane");
                        ui.label(format!("{u}   (self-signed certificate; log in with the user above)"));
                        ui.end_row();
                    }
                    ui.label("Serial console");
                    ui.label(format!("{} at {} baud, 8N1 - built-in port and any USB serial adapter. VGA/HDMI and USB keyboard/mouse work on the local console.", self.serial_tty, self.serial_speed));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.label("Root login is disabled. The password and the private key are shown once here; store them somewhere safe. If the NAS does not boot from its disks, check the BIOS/UEFI boot order (the internal Synology flash module may take precedence).");
                if let Some(p) = &o.log_path {
                    ui.label(format!("Full debug log: {}", p.display()));
                }
                ui.horizontal(|ui| {
                    if ui.button("Copy ssh command").clicked() {
                        ui.ctx().copy_text(format!("ssh {}@{}", o.username, ip));
                    }
                    if ui.button("Save logins to a file...").clicked()
                        && let Some(p) = rfd::FileDialog::new().set_file_name("nas-logins.txt").save_file()
                    {
                        let txt = format!(
                            "host: {}\naddress: {}\nuser: {}\npassword: {}\nssh: ssh {}@{}\nssh host key: {}\nmanagement: {}\n",
                            o.hostname,
                            ip,
                            o.username,
                            self.new_password,
                            o.username,
                            ip,
                            o.ssh_fingerprint.clone().unwrap_or_default(),
                            o.mgmt_url.clone().unwrap_or_default()
                        );
                        if std::fs::write(&p, txt).is_ok() {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                            }
                        }
                    }
                });
            }
            Some(Err(e)) => {
                ui.heading(RichText::new("The installation did not finish").color(Color32::LIGHT_RED));
                ui.label(RichText::new(&e).color(Color32::LIGHT_RED));
                ui.label("The installer halts instead of guessing. Check the NAS console (serial/VGA) and the debug log below; the full log file is on this computer.");
                if let Some(p) = self.logger.as_ref().and_then(|l| l.path()) {
                    ui.label(format!("Log file: {}", p.display()));
                }
                egui::ScrollArea::vertical().stick_to_bottom(true).max_height(360.0).show(ui, |ui| {
                    for l in self.log.iter().filter(|l| l.level >= Level::Info).rev().take(300).collect::<Vec<_>>().into_iter().rev() {
                        ui.label(RichText::new(format!("{} {}", l.ts, l.msg)).monospace());
                    }
                });
            }
            None => {}
        }
        if ui.button("Start over").clicked() {
            self.screen = Screen::Inventory;
            self.result = None;
            self.log.clear();
            self.new_password = generate_password(20);
            self.ack_backup = false;
            self.ack_loss = false;
            self.typed.clear();
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("steps").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("synofurb {}", env!("CARGO_PKG_VERSION"))).strong());
                ui.separator();
                for (s, name) in STEPS {
                    let cur = s == self.screen;
                    ui.label(if cur { RichText::new(name).strong().underline() } else { RichText::new(name).weak() });
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = self.error.clone()
                && self.screen != Screen::Confirm
            {
                ui.label(RichText::new(e).color(Color32::LIGHT_RED));
            }
            match self.screen {
                Screen::Inventory => self.screen_inventory(ui),
                Screen::Connect => self.screen_connect(ui),
                Screen::Layout => self.screen_layout(ui),
                Screen::Account => self.screen_account(ui),
                Screen::Confirm => self.screen_confirm(ui),
                Screen::Install => self.screen_install(ui),
                Screen::Done => self.screen_done(ui),
            }
        });
    }
}
