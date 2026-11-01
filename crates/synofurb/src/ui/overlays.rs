//! Dialogs and the Help sheet that are opened from several screens, and the "Copy command" action.

use super::app::App;
use super::cli_cmd::CliCommand;
use super::copy::{self, Item};
use super::disclosure::{dialog, paragraphs};
use super::icons::{self, role};
use super::screens::help::HelpView;
use super::state::{BootChoice, KeyMode, Screen};
use super::theme::layout;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::Input;
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, ClipboardItem, Context, IntoElement, Window, px};
use gpui_kit::{Entity, div};
use synofurb_core::plan::StorageMode;

impl App {
    /// The Help sheet, optionally opened on a topic. Pressing F1 while it is open does nothing.
    pub fn open_help_topic(&mut self, topic: Option<&'static str>, window: &mut Window, cx: &mut Context<App>) {
        if window.has_active_sheet(cx) {
            // already open: just show the requested topic
            if let Some(v) = self.help_view.as_ref().and_then(|v| v.upgrade()) {
                v.update(cx, |v, cx| v.show(topic, cx));
                return;
            }
        }
        let topic = topic.or_else(|| self.default_help_topic());
        let app = cx.entity();
        let view = cx.new(|cx| HelpView::new(app, topic, window, cx));
        self.help_view = Some(view.downgrade());
        window.open_sheet(cx, move |sheet, _, _| sheet.title(copy::HELP_TITLE.short).size(px(layout::SHEET_W_WIDE)).child(view.clone()));
    }

    /// The topic F1 opens on the current screen.
    pub fn default_help_topic(&self) -> Option<&'static str> {
        Some(match self.screen {
            Screen::Confirm | Screen::Install | Screen::Failed | Screen::Recover => "recovery",
            Screen::BootMgr => "boot-manager",
            Screen::Inventory | Screen::Connect => "first-install",
            Screen::System => "first-install",
            Screen::Done => "first-install",
            _ => return None,
        })
    }

    pub fn open_keep_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let keep = self.form.req.mode == StorageMode::KeepVolumes;
        window.open_dialog(cx, move |d, _, cx| {
            let app = app.clone();
            d.title(copy::PLAN_KEEP_DIALOG.short).w(px(layout::DIALOG_W)).child(paragraphs(copy::PLAN_KEEP_DIALOG.body, cx)).footer(
                DialogFooter::new()
                    .child(
                        Button::new("keep-cancel")
                            .outline()
                            .label(copy::BTN_CANCEL.short)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("keep-use")
                            .primary()
                            .label(if keep { copy::PLAN_USE_WIPE.short } else { copy::PLAN_USE_KEEP.short })
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                app.update(cx, |a, cx| {
                                    a.set_storage_mode(if keep { StorageMode::WipeAll } else { StorageMode::KeepVolumes }, cx)
                                });
                            }),
                    ),
            )
        });
    }

    /// "What happens next" with the variant that matches what happened.
    pub fn open_next_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let item: &'static Item = if self.agent_unknown {
            &copy::DLG_NEXT_AGENT
        } else if !self.install.past_no_return {
            &copy::DLG_NEXT_PRE
        } else if self.form.req.mode == StorageMode::KeepVolumes {
            &copy::DLG_NEXT_KEEP
        } else {
            &copy::DLG_NEXT_WIPE
        };
        let app = cx.entity();
        dialog(window, cx, item, &app);
    }

    /// "Before you start": once, on the first launch.
    pub fn open_first_launch(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        window.open_dialog(cx, move |d, _, cx| {
            let app = app.clone();
            d.title(copy::FIRST_LAUNCH.short).w(px(layout::DIALOG_W)).child(paragraphs(copy::FIRST_LAUNCH.body, cx)).footer(
                DialogFooter::new()
                    .child(Button::new("fl-help").ghost().icon(icons::icon(role::HELP)).label(copy::BTN_OPEN_HELP.short).on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            app.update(cx, |a, cx| a.open_help_topic(Some("cli"), window, cx));
                        }
                    }))
                    .child(Button::new("fl-ok").primary().label(copy::BTN_GOT_IT.short).on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        app.update(cx, |a, _| {
                            a.settings.first_launch_done = true;
                            a.save_settings();
                        });
                    })),
            )
        });
    }

    /// "Add NAS": probe an address that discovery did not find.
    pub fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let input = self.inp.add_ip.clone();
        window.open_dialog(cx, move |d, _, _| {
            let (app, input2) = (app.clone(), input.clone());
            d.title(copy::INV_ADD_TITLE.short)
                .w(px(layout::DIALOG_W))
                .child(v_flex().gap(px(8.0)).child(copy::INV_ADD_TITLE.body).child(Input::new(&input).cleanable(true)))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("add-cancel")
                                .outline()
                                .label(copy::BTN_CANCEL.short)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("add-ok").primary().label(copy::INV_ADD_BTN.short).on_click(move |_, window, cx| {
                            let ip = input2.read(cx).value().trim().to_string();
                            if !ip.is_empty() {
                                window.close_dialog(cx);
                                app.update(cx, |a, cx| a.select_host(&ip, window, cx));
                            }
                        })),
                )
        });
    }

    /// The command line for the current choices (None until a unit has been inspected).
    pub fn cli_spec(&self, cx: &gpui_kit::App) -> Option<CliCommand> {
        let f = self.facts.as_ref()?;
        let plan = &self.form;
        let key_file = self.use_key.then(|| self.input_value(&self.inp.key_path, cx)).filter(|s| !s.trim().is_empty());
        let authorize_key = match self.sys.key_mode {
            KeyMode::Generate => "generate".to_string(),
            KeyMode::None => "none".to_string(),
            KeyMode::Paste => "<public-key-file>".to_string(),
        };
        let dtb = self.input_value(&self.inp.dtb, cx);
        let method = self.sys.boot.resolve(f);
        let serial = self.input_value(&self.inp.serial_text, cx).trim().to_string();
        let kit = self.input_value(&self.inp.kit_dir, cx).trim().to_string();
        let kexec = self.input_value(&self.inp.kexec_path, cx).trim().to_string();
        Some(CliCommand {
            host: super::format::display_host(&self.input_value(&self.inp.host, cx)),
            port: self.input_value(&self.inp.port, cx).trim().parse().unwrap_or(22),
            user: self.input_value(&self.inp.user, cx).trim().to_string(),
            key_file,
            keep_mode: plan.req.mode == StorageMode::KeepVolumes,
            keep_arrays: plan.keep.iter().cloned().collect(),
            system_gib: plan.system_gib.max(0.0) as u64,
            copies: (!plan.copies_auto).then_some(plan.copies),
            raid: plan.req.data_raid,
            free_percent: plan.req.data_free_percent,
            kernel: plan.kernel,
            hostname: self.input_value(&self.inp.hostname, cx).trim().to_string(),
            username: self.input_value(&self.inp.username, cx).trim().to_string(),
            authorize_key,
            timezone: self.input_value(&self.inp.timezone, cx).trim().to_string(),
            mirror: self.input_value(&self.inp.mirror, cx).trim().to_string(),
            serial,
            serial_speed: self.input_value(&self.inp.serial_speed, cx).trim().parse().unwrap_or(115_200),
            dtb: Some(dtb.trim().to_string()).filter(|d| !d.is_empty()),
            method,
            kit_dir: (method == BootChoice::Kit).then_some(kit).filter(|s| !s.is_empty()),
            kexec: (method == BootChoice::Kexec).then_some(kexec).filter(|s| !s.is_empty()),
            bundle: self.source_offline.then(|| self.input_value(&self.inp.bundle, cx).trim().to_string()).filter(|s| !s.is_empty()),
            no_firmware: !self.sys.with_firmware,
            no_verbose: !self.sys.verbose,
        })
    }

    /// The command to copy on this screen: the read-only `plan` form on Plan and before the confirmation is satisfied,
    /// the `install` form with the three confirmation flags only after it is.
    pub fn cli_text(&self, cx: &gpui_kit::App) -> Option<String> {
        let spec = self.cli_spec(cx)?;
        Some(match self.screen {
            Screen::Plan => spec.plan_command(),
            Screen::System => spec.install_command(None),
            Screen::Confirm => {
                let typed = self.input_value(&self.inp.typed, cx);
                let phrase = self.phrase(cx);
                if self.gate(&typed, &phrase).is_open() { spec.install_command(Some(&phrase)) } else { spec.plan_command() }
            }
            _ => spec.install_command(None),
        })
    }

    /// Ctrl+Shift+C and the "Copy command" button: the dialog with the command, editable, and the computed tag.
    pub fn copy_cli_command(&mut self, window: &mut Window, cx: &mut Context<App>) {
        if !matches!(self.screen, Screen::Plan | Screen::System | Screen::Confirm) {
            return;
        }
        let Some(spec) = self.cli_spec(cx) else { return };
        // the confirmation flags are in the command only after the gate is satisfied
        let typed = self.input_value(&self.inp.typed, cx);
        let phrase = self.phrase(cx);
        let confirm = (self.screen == Screen::Confirm && self.gate(&typed, &phrase).is_open()).then_some(phrase);
        let plan_only = self.screen == Screen::Plan || (self.screen == Screen::Confirm && confirm.is_none());
        let text = if plan_only { spec.plan_command() } else { spec.install_command(confirm.as_deref()) };
        let view = cx.new(|cx| CommandView::new(spec, confirm, plan_only, text, window, cx));
        window.open_dialog(cx, move |d, _, _| {
            d.title(copy::DLG_COMMAND.short).w(px(layout::DIALOG_W_WIDE)).child(view.clone()).footer(
                DialogFooter::new()
                    .child(
                        Button::new("cmd-close").outline().label(copy::BTN_CLOSE.short).on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child({
                        let view = view.clone();
                        Button::new("cmd-copy").primary().icon(icons::icon(role::COPY)).label(copy::BTN_COPY.short).on_click(
                            move |_, window, cx| {
                                let text = view.read(cx).text(cx);
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                                window.push_notification(gpui_kit::component::notification::Notification::success("Command copied"), cx);
                            },
                        )
                    }),
            )
        });
    }

    /// The icon button ("Copy command", terminal icon, tooltip only).
    pub fn copy_command_button(&self, cx: &mut Context<App>) -> gpui_kit::AnyElement {
        let item = if self.screen == Screen::Confirm { &copy::BTN_COPY_CMD_CONFIRM } else { &copy::BTN_COPY_CMD };
        Button::new("copy-cmd")
            .ghost()
            .icon(icons::icon(role::TERMINAL))
            .tooltip(item.tip)
            .accessibility_label(item.short)
            .on_click(cx.listener(|this, _, window, cx| this.copy_cli_command(window, cx)))
            .into_any_element()
    }

    /// The app (not the NAS) could not reach the Internet.
    pub fn offline_app(&self) -> bool {
        self.app_offline || matches!(self.update, super::app::UpdateState::CheckFailed(_))
    }

    pub fn set_storage_mode(&mut self, mode: StorageMode, cx: &mut Context<App>) {
        self.form.req.mode = mode;
        self.rebuild_plan();
        cx.notify();
    }
}

impl App {
    /// "Hybrid slices": the slices table of the current plan (Slice, Disks, Level, Usable).
    pub fn open_slices_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
        let Some(shr) = self.form.plan.as_ref().and_then(|p| p.shr.clone()) else { return };
        let rows: Vec<_> = shr.layout.slices.iter().map(|s| super::raid::slice_row(s, &shr.disks)).collect();
        window.open_dialog(cx, move |d, _, cx| {
            let table = Table::new()
                .child(
                    TableHeader::new().child(
                        TableRow::new()
                            .child(TableHead::new().child(copy::SLICE_COL_SLICE.short))
                            .child(TableHead::new().child(copy::SLICE_COL_DISKS.short))
                            .child(TableHead::new().child(copy::SLICE_COL_LEVEL.short))
                            .child(TableHead::new().text_right().child(copy::SLICE_COL_USABLE.short)),
                    ),
                )
                .child(TableBody::new().children(rows.iter().map(|(a, b, c, e)| {
                    TableRow::new()
                        .child(TableCell::new().child(a.clone()))
                        .child(TableCell::new().child(super::kit::mono_sm(cx, b.clone())))
                        .child(TableCell::new().child(c.clone()))
                        .child(TableCell::new().text_right().child(super::kit::mono(cx, e.clone())))
                })));
            d.title(copy::DLG_SLICES.short)
                .w(px(layout::DIALOG_W_WIDE))
                .child(v_flex().gap(px(12.0)).child(table).child(paragraphs(copy::DLG_SLICES.body, cx)))
                .footer(DialogFooter::new().child(
                    Button::new("sl-close").primary().label(copy::BTN_CLOSE.short).on_click(|_, window, cx| window.close_dialog(cx)),
                ))
        });
    }
}

/// The body of the "Copy command" dialog: the command in an editable `Textarea` and the tag that is computed from what
/// the text says right now ("Matches plan" or "Differs from plan"; the read-only plan form says so).
pub struct CommandView {
    spec: CliCommand,
    confirm: Option<String>,
    plan_only: bool,
    input: Entity<gpui_kit::component::input::TextareaState>,
    _sub: gpui_kit::Subscription,
}

impl CommandView {
    pub fn new(
        spec: CliCommand,
        confirm: Option<String>,
        plan_only: bool,
        text: String,
        window: &mut Window,
        cx: &mut Context<CommandView>,
    ) -> CommandView {
        use gpui_kit::component::input::{InputEvent, TextareaState};
        let input = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx);
            s.set_value(text, window, cx);
            s
        });
        let sub = cx.subscribe(&input, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        CommandView { spec, confirm, plan_only, input, _sub: sub }
    }

    pub fn text(&self, cx: &gpui_kit::App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Does the current text still say what the plan says?
    pub fn matches(&self, cx: &gpui_kit::App) -> bool {
        let t = self.text(cx);
        if self.plan_only {
            super::cli_cmd::parse_flags(&t) == super::cli_cmd::parse_flags(&self.spec.plan_command())
        } else {
            self.spec.matches(&t, self.confirm.as_deref())
        }
    }
}

impl gpui_kit::Render for CommandView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use super::theme::{Themed, Tone};
        use gpui_kit::component::input::Textarea;
        let p = cx.pal();
        let ok = self.matches(cx);
        let tag = if !ok {
            super::kit::tag(&p, Tone::Warn, Some(role::WARN), copy::CMD_DIFFERS.short)
        } else if self.plan_only {
            super::kit::tag(&p, Tone::Neutral, Some(role::LOCK), copy::CMD_READ_ONLY.short)
        } else {
            super::kit::tag(&p, Tone::Ok, Some(role::OK), copy::CMD_MATCHES.short)
        };
        v_flex()
            .gap(px(8.0))
            .child(
                div()
                    .font_family(gpui_kit::component::ActiveTheme::theme(&**cx).mono_font_family.clone())
                    .text_xs()
                    .child(Textarea::new(&self.input).h(px(150.0)).aria_label(copy::DLG_COMMAND.short)),
            )
            .child(
                gpui_kit::component::h_flex().gap(px(8.0)).items_center().child(tag).child(super::kit::caption(cx, copy::DLG_COMMAND.body)),
            )
    }
}

// ------------------------------------------------------------------------------------------------ quick setup

/// The Quick setup dialog: recommended defaults in one line and, when the disks differ, an explicit Hybrid vs RAID
/// choice with nothing preselected (the owner's decision): "Use these" stays off until one is picked.
pub struct QuickView {
    opts: Vec<(synofurb_core::plan::DataRaid, String)>,
    pub sel: Option<usize>,
    summary: String,
}

impl gpui_kit::Render for QuickView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::alert::Alert;
        use gpui_kit::component::radio::{Radio, RadioGroup};
        let mut col = v_flex().gap(px(12.0)).child(super::kit::mono(cx, self.summary.clone()));
        if self.opts.len() > 1 {
            col = col
                .child(
                    RadioGroup::vertical("quick-raid")
                        .selected_index(self.sel)
                        .children(self.opts.iter().enumerate().map(|(i, (_, l))| Radio::new(("qr", i)).label(l.clone())))
                        .on_click(cx.listener(|v, ix: &usize, _, cx| {
                            v.sel = Some(*ix);
                            cx.notify();
                        })),
                )
                .child(Alert::info("quick-hybrid", copy::QUICK_HYBRID_CAUTION.short).icon(super::icons::icon(role::INFO)));
        }
        col
    }
}

impl App {
    pub fn open_quick_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        use synofurb_core::plan::{DataRaid, PlanRequest, build_plan};
        let Some(f) = self.facts.clone() else { return };
        let plan_for = |r: DataRaid| build_plan(&f, &PlanRequest { data_raid: r, ..PlanRequest::default() });
        let tib = |b: u64| b as f64 / (1u64 << 40) as f64;
        let sizes: Vec<f64> = f.candidate_disks().iter().map(|d| tib(d.size_bytes())).collect();
        let (hyb_plan, auto_plan) = (plan_for(DataRaid::Shr1), plan_for(DataRaid::Auto));
        let (hyb, auto) = (hyb_plan.sizes.data_usable_bytes, auto_plan.sizes.data_usable_bytes);
        let q = super::facts::quick_defaults(&sizes, tib(hyb), tib(auto));
        // "RAID 6 · survives 2 disks · 7.15 TiB": the level the plan really builds, not "Auto".
        let label = |plan: &synofurb_core::plan::Plan| {
            let surv =
                super::raid::survives(plan).map(|n| format!(" \u{00b7} survives {}", super::raid::survives_text(n))).unwrap_or_default();
            format!("{}{surv} \u{00b7} {}", super::raid::resolved_label(plan), super::format::size_2dp(plan.sizes.data_usable_bytes))
        };
        let (opts, summary_layout) = if q.mixed {
            (vec![(DataRaid::Auto, label(&auto_plan)), (DataRaid::Shr1, label(&hyb_plan))], "choose a layout".to_string())
        } else {
            (vec![(q.raid, String::new())], label(&auto_plan))
        };
        let summary = format!(
            "Wipe all \u{00b7} system {} GiB \u{00d7} {} \u{00b7} free {} % \u{00b7} Stable kernel \u{00b7} {}",
            q.system_gib, q.copies, q.free_percent, summary_layout
        );
        let raids: Vec<DataRaid> = opts.iter().map(|o| o.0).collect();
        let single = opts.len() == 1;
        let view = cx.new(|_| QuickView { opts, sel: single.then_some(0), summary });
        let app = cx.entity();
        window.open_dialog(cx, move |d, _, cx| {
            let (app, view2, raids) = (app.clone(), view.clone(), raids.clone());
            let unchosen = view.read(cx).sel.is_none();
            d.title(copy::QUICK_TITLE.short).w(px(layout::DIALOG_W)).child(view.clone()).footer(
                DialogFooter::new()
                    .child(
                        Button::new("q-cancel").outline().label(copy::BTN_CANCEL.short).on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(super::kit::with_reason(
                        "q-why",
                        Button::new("q-use").primary().label(copy::QUICK_USE.short).on_click(move |_, window, cx| {
                            let Some(sel) = view2.read(cx).sel else { return };
                            let raid = raids[sel.min(raids.len() - 1)];
                            window.close_dialog(cx);
                            app.update(cx, |a, cx| a.apply_quick(raid, q.system_gib, q.copies, q.free_percent, window, cx));
                        }),
                        unchosen.then(|| copy::QUICK_CHOOSE.tip.to_string()),
                    )),
            )
        });
    }

    /// Apply the Quick setup choices to the plan and go straight to Confirm.
    pub fn apply_quick(
        &mut self,
        raid: synofurb_core::plan::DataRaid,
        system_gib: u64,
        copies: usize,
        free: u8,
        window: &mut Window,
        cx: &mut Context<App>,
    ) {
        use gpui_kit::component::IndexPath;
        self.form.req.mode = StorageMode::WipeAll;
        self.form.req.data_raid = raid;
        self.form.req.data_free_percent = free;
        self.form.copies_auto = false;
        self.form.copies = copies;
        self.form.system_gib = system_gib as f32;
        self.sys.boot = BootChoice::Auto;
        self.form.kernel = synofurb_core::config::KernelChannel::Stable;
        if let Some(ix) = synofurb_core::plan::DataRaid::ALL.iter().position(|r| *r == raid) {
            self.form.raid_select.update(cx, |s, cx| s.set_selected_index(Some(IndexPath::default().row(ix)), window, cx));
        }
        self.rebuild_plan();
        self.goal = super::app::Goal::Install;
        self.go(Screen::Confirm, window, cx);
    }
}
