//! Recovery (approved mockup): the rows follow the unit's boot state, read over SSH (the session's memory when the unit
//! cannot be read). A row that cannot work is a real disabled button with a reason popover beside it; when the unit never
//! answered, "Manual recovery" is the primary. Boot DSM, Open agent, Restore flash and Remove entries run the engine's
//! recovery actions (`synofurb_core::recovery`); Restore and Remove ask for a typed phrase; the outcome is the engine's
//! own, verified by reading the unit back. Each row also has a secondary "Show command" for the CLI equivalent.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure::{info_tip, paragraphs};
use crate::ui::facts::{self, RecKind};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone};
use crate::ui::worker::{RecAction, RecLevel};
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, Context, Entity, IntoElement, Render, Window, div, px};

/// What a recovery row shows after it was pressed. `Done` only when the engine verified it by reading the unit back; a
/// command shown for the user to run is `Info`, never success.
#[derive(Clone, Debug)]
pub struct RecResult {
    /// Which recovery action produced it (`None` for a note such as "Show command").
    pub kind: Option<RecKind>,
    pub level: RecLevel,
    pub title: String,
    pub detail: String,
    /// Commands to run, each with a Copy button.
    pub commands: Vec<String>,
}

impl RecResult {
    pub fn error(title: &str, detail: String) -> RecResult {
        RecResult { kind: None, level: RecLevel::Error, title: title.into(), detail, commands: vec![] }
    }
}

fn rec_icon(k: RecKind) -> gpui_kit::assets::IconName {
    match k {
        RecKind::BootDsm => role::ROTATE_BACK,
        RecKind::Agent => role::TERMINAL,
        RecKind::RestoreFlash => role::DISK,
        RecKind::RemoveEntries => role::DELETE,
        RecKind::ReinstallDsm => role::HELP,
    }
}

impl App {
    pub(crate) fn page_recover(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let uf = self.unit_facts();
        let rows = facts::recovery_rows(&uf);
        let primary = facts::recovery_primary(&uf);
        let manual_primary = facts::recovery_manual_primary(&uf);
        let running = self.rec_running;

        let chips = h_flex()
            .gap(px(8.0))
            .flex_wrap()
            .child(kit::tag(
                &p,
                if uf.flash { Tone::Warn } else { Tone::Neutral },
                Some(role::DISK),
                if uf.flash { copy::REC_FLASH_OURS.short } else { copy::REC_FLASH_CLEAN.short },
            ))
            .child(if uf.agent_unknown {
                h_flex()
                    .gap(px(2.0))
                    .child(kit::tag(&p, Tone::Warn, Some(role::TIP), copy::REC_DEFAULT_UNKNOWN.short))
                    .child(info_tip("rec-def", &copy::REC_DEFAULT_UNKNOWN, &app))
                    .into_any_element()
            } else {
                kit::tag(
                    &p,
                    Tone::Neutral,
                    Some(role::SERVER),
                    if uf.default_not_dsm { copy::REC_DEFAULT_DEBIAN.short } else { copy::REC_DEFAULT_DSM.short },
                )
                .into_any_element()
            })
            .child({
                // The boot menu can be read from the unit; the disks cannot. When this session did not change them, say so only
                // as "untouched" if the unit was not read from earlier sessions (otherwise: not checked).
                let read = self.unit_state.as_ref().is_some_and(|s| s.read == synofurb_core::recovery::ReadOutcome::Read);
                if uf.disks {
                    kit::tag(&p, Tone::Warn, Some(role::WARN), copy::REC_DISKS_MODIFIED.short)
                } else if read {
                    kit::tag(&p, Tone::Neutral, Some(role::TIP), copy::REC_DISKS_UNKNOWN.short)
                } else {
                    kit::tag(&p, Tone::Ok, Some(role::SAFE), copy::REC_DISKS_UNTOUCHED.short)
                }
            });

        let mut body = v_flex().w_full().max_w(px(720.0)).gap(px(12.0)).child(chips);
        // What was read from the unit, or why nothing was (the chips fall back to this session's memory then).
        use synofurb_core::recovery::ReadOutcome;
        if self.unit_state_loading {
            body = body.child(Alert::info("rec-reading", copy::REC_READING.short).small().icon(icons::icon(role::RUNNING)));
        } else if let Some(e) = &self.unit_state_err {
            body = body.child(
                Alert::warning("rec-readerr", format!("{} {}", copy::REC_READ_FAILED.tip, e))
                    .title(copy::REC_READ_FAILED.short)
                    .small()
                    .icon(icons::icon(role::WARN)),
            );
        } else if let Some(s) = &self.unit_state {
            let why = match &s.read {
                ReadOutcome::Read => None,
                ReadOutcome::ToolMissing => Some(copy::REC_TOOL_MISSING.tip.to_string()),
                ReadOutcome::Refused(t) | ReadOutcome::Failed(t) => Some(format!("{} {}", copy::REC_READ_FAILED.tip, t)),
            };
            if let Some(w) = why {
                body = body.child(Alert::warning("rec-notread", w).title(copy::REC_NOT_READ.short).small().icon(icons::icon(role::WARN)));
            }
        }
        if uf.agent_unknown {
            body = body.child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(
                        Alert::warning("rec-unknown", copy::REC_UNKNOWN_ALERT.tip)
                            .title(copy::REC_UNKNOWN_ALERT.short)
                            .icon(icons::icon(role::TIP))
                            .flex_1()
                            .min_w_0(),
                    )
                    .child({
                        let b = Button::new("manual")
                            .icon(icons::icon(role::HELP))
                            .label(copy::REC_MANUAL.short)
                            .on_click(cx.listener(|this, _, window, cx| this.open_manual_recovery(window, cx)));
                        if manual_primary { b.primary() } else { b.outline() }
                    }),
            );
        } else if !uf.flash && !uf.disks {
            let read = self.unit_state.as_ref().is_some_and(|s| s.read == synofurb_core::recovery::ReadOutcome::Read);
            body = if read {
                body.child(
                    Alert::info("rec-none", copy::REC_NOTHING_READ.tip).title(copy::REC_NOTHING_READ.short).icon(icons::icon(role::INFO)),
                )
            } else {
                body.child(Alert::info("rec-none", copy::REC_NOTHING.tip).title(copy::REC_NOTHING.short).icon(icons::icon(role::INFO)))
            };
        }
        if let Some(k) = self.rec_running {
            let what = match k {
                RecKind::BootDsm => copy::REC_BOOT_RUNNING.short,
                RecKind::Agent => copy::REC_AGENT_RUNNING.short,
                RecKind::RestoreFlash => copy::REC_RESTORE_RUNNING.short,
                RecKind::RemoveEntries => copy::REC_REMOVE_RUNNING.short,
                RecKind::ReinstallDsm => "",
            };
            body = body.child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(Alert::info("rec-run", what).small().icon(icons::icon(role::RUNNING)).flex_1().min_w_0())
                    // Open agent waits up to the agent's start time: the user can stop waiting (the unit may still boot the agent)
                    .children((k == RecKind::Agent).then(|| {
                        let flag = self.rec_cancel.clone();
                        Button::new("rec-cancel")
                            .outline()
                            .label(copy::REC_STOP_WAITING.short)
                            .tooltip(copy::REC_STOP_WAITING.tip)
                            .on_click(move |_, _, _| flag.store(true, std::sync::atomic::Ordering::Relaxed))
                    })),
            );
        }
        if let Some(r) = &self.rec_result {
            body = body.child(kit::rec_alert("rec-res", r.level, &r.title, &r.detail));
            for (i, cmd) in r.commands.iter().enumerate() {
                body = body.child(kit::command_row(cx, ("rec-cmd", i), cmd.clone()));
            }
        }

        let list = v_flex().rounded(cx.theme().radius).border_1().border_color(cx.theme().border).children(rows.iter().enumerate().map(
            |(i, r)| {
                let kind = r.kind;
                let mut b = Button::new(("rec-run", i))
                    .label(r.button)
                    .icon(icons::icon(rec_icon(kind)))
                    .accessibility_label(format!("{}: {}", r.button, r.label));
                b = if primary == Some(kind) { b.primary() } else { b.outline() };
                if running == Some(kind) {
                    b = b.loading(true);
                }
                let busy = running.is_some();
                let b = b.disabled(busy).on_click(cx.listener(move |this, _, window, cx| this.run_recovery_row(kind, window, cx)));
                let show_cmd = (kind != RecKind::ReinstallDsm).then(|| {
                    Button::new(("rec-cmd-btn", i))
                        .ghost()
                        .small()
                        .icon(icons::icon(role::TERMINAL))
                        .tooltip(copy::REC_SHOW_COMMAND.short)
                        .accessibility_label(format!("{}: {}", copy::REC_SHOW_COMMAND.short, r.label))
                        .on_click(cx.listener(move |this, _, _, cx| this.show_recovery_command(kind, cx)))
                });
                let item: &'static copy::Item = match kind {
                    RecKind::BootDsm => &copy::REC_BOOT_DSM,
                    RecKind::Agent => &copy::REC_OPEN_AGENT,
                    RecKind::RestoreFlash => &copy::REC_RESTORE_FLASH,
                    RecKind::RemoveEntries => &copy::REC_REMOVE,
                    RecKind::ReinstallDsm => &copy::REC_REINSTALL,
                };
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .when(i + 1 < rows.len(), |d| d.border_b_1().border_color(cx.theme().border))
                    .child(icons::icon(rec_icon(kind)))
                    .child(div().flex_1().child(h_flex().gap(px(2.0)).items_center().child(kit::strong(r.label)).child(info_tip(
                        ("rec-i", i),
                        item,
                        &app,
                    ))))
                    .children(show_cmd.filter(|_| r.off.is_none()))
                    .child(kit::with_reason(("rec-why", i), b, r.off.map(String::from)))
            },
        ));
        body = body.child(list);

        let backups = self.cfg_dir.join("dom-backups");
        let saved = v_flex().gap(px(6.0)).child(kit::mono_sm(cx, backups.display().to_string())).child(h_flex().child(
            Button::new("open-folder").outline().small().icon(icons::icon(role::FOLDER)).label(copy::REC_OPEN_FOLDER.short).on_click(
                move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&backups);
                    cx.reveal_path(&backups)
                },
            ),
        ));
        body = body.child(Accordion::new("rec-acc").bordered(true).item(|i| i.title(copy::REC_SAVED.short).child(saved)));
        Page::new(copy::REC_TITLE.short, body)
            .subtitle(if self.unit_state.as_ref().is_some_and(|s| s.read == ReadOutcome::Read) {
                copy::REC_SUB_READ.short
            } else {
                copy::REC_SUB.short
            })
            .subtitle_tip(&copy::REC_SUB)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
            ])
    }

    /// Run one Recovery row: the engine's recovery action, behind a confirmation that names what happens (Restore flash and
    /// Remove entries need their typed phrase). The reboot of "Boot DSM" is its own explicit choice.
    pub fn run_recovery_row(&mut self, kind: RecKind, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let backups = self.cfg_dir.join("dom-backups");
        match kind {
            RecKind::BootDsm => self.open_boot_dsm_dialog(window, cx),
            RecKind::Agent => {
                let app2 = app.clone();
                crate::ui::disclosure::dialog_with(
                    window,
                    cx,
                    if self.unit_facts().debian { &copy::DLG_AGENT_DEBIAN } else { &copy::DLG_AGENT },
                    &app,
                    copy::REC_AGENT_OK.short,
                    move |cx| app2.update(cx, |a, cx| a.start_recovery(RecKind::Agent, RecAction::Agent, cx)),
                );
            }
            RecKind::RestoreFlash => {
                // Only an image made for THIS unit (same serial number, by the engine's `backups_for_unit`); never "the newest"
                // of anything else. Without a known serial nothing can be matched, so nothing is offered.
                let id = self.unit_id(cx);
                let backup = synofurb_core::recovery::backups_for_unit(&backups, &id).into_iter().next();
                let Some(backup) = backup else {
                    let why = if id.serial.is_empty() { copy::REC_NO_SERIAL.tip } else { copy::REC_NO_BACKUP.tip };
                    self.rec_result = Some(RecResult::error(copy::REC_NO_BACKUP.short, format!("{why} {}", backups.display())));
                    cx.notify();
                    return;
                };
                let phrase = facts::restore_phrase(&id.model, &id.serial);
                let name = backup.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let body = format!(
                    "{}\n\nThis unit: {} \u{b7} serial {}\nFile: {}\nSaved: {} \u{b7} {} MiB",
                    copy::DLG_RESTORE.body,
                    id.model,
                    id.serial,
                    name,
                    backup.modified.clone().unwrap_or_else(|| "date unknown".into()),
                    backup.bytes / 1_048_576
                );
                let path = backup.path.clone();
                typed_confirm(window, cx, &copy::DLG_RESTORE, body, phrase, app, move |a, cx| {
                    a.start_recovery(RecKind::RestoreFlash, RecAction::Restore { backup: path.clone(), unit: a.unit_id(cx) }, cx)
                });
            }
            RecKind::RemoveEntries => {
                typed_confirm(
                    window,
                    cx,
                    &copy::DLG_REMOVE,
                    copy::DLG_REMOVE.body.to_string(),
                    facts::REMOVE_PHRASE.to_string(),
                    app,
                    |a, cx| a.start_recovery(RecKind::RemoveEntries, RecAction::Remove, cx),
                );
            }
            RecKind::ReinstallDsm => {
                window.open_dialog(cx, move |d, _, cx| {
                    d.bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
                        .title(copy::DLG_REINSTALL.short)
                        .w(px(crate::ui::theme::layout::DIALOG_W))
                        .child(paragraphs(copy::DLG_REINSTALL.body, cx))
                        .footer(
                            DialogFooter::new()
                                .child(
                                    Button::new("ri-close")
                                        .outline()
                                        .label(copy::BTN_CLOSE.short)
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("ri-open")
                                        .primary()
                                        .icon(icons::icon(role::EXTERNAL))
                                        .label(copy::BTN_OPEN_SYNOLOGY.short)
                                        .on_click(|_, _, cx| cx.open_url("https://find.synology.com")),
                                ),
                        )
                });
            }
        }
    }

    /// "Boot DSM next?": set the default now, with or without asking the unit to reboot (the user chooses; nothing reboots
    /// unless "Set and reboot" is pressed).
    pub fn open_boot_dsm_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        window.open_dialog(cx, move |d, _, cx| {
            let (a1, a2) = (app.clone(), app.clone());
            d.bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
                .title(copy::DLG_BOOT_DSM.short)
                .w(px(crate::ui::theme::layout::DIALOG_W))
                .child(paragraphs(copy::DLG_BOOT_DSM.body, cx))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("bd-cancel")
                                .outline()
                                .label(copy::BTN_CANCEL.short)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("bd-set").outline().label(copy::REC_BOOT_DSM_OK.short).on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            a1.update(cx, |a, cx| a.start_recovery(RecKind::BootDsm, RecAction::BootDsm { reboot: false }, cx));
                        }))
                        .child(Button::new("bd-reboot").primary().label(copy::REC_BOOT_DSM_REBOOT.short).on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            a2.update(cx, |a, cx| a.start_recovery(RecKind::BootDsm, RecAction::BootDsm { reboot: true }, cx));
                        })),
                )
        });
    }

    /// "Show command": the CLI equivalent of a row, as a neutral note with Copy.
    pub fn show_recovery_command(&mut self, kind: RecKind, cx: &mut Context<App>) {
        let backups = self.cfg_dir.join("dom-backups");
        let host = {
            let h = self.input_value(&self.inp.host, cx);
            if h.trim().is_empty() { self.selected_ip.clone().unwrap_or_default() } else { h.trim().to_string() }
        };
        let user = {
            let u = self.input_value(&self.inp.user, cx);
            if u.trim().is_empty() { "admin".to_string() } else { u.trim().to_string() }
        };
        let (title, detail, commands) = match kind {
            RecKind::BootDsm => (
                "Run on this computer",
                "Sets the default boot entry to DSM and reads it back; add --reboot --yes to reboot.",
                vec![format!("synofurb-cli recover boot-dsm --host {}", facts::shell_arg(&host))],
            ),
            RecKind::Agent => (
                "Run on this computer",
                "Arms the agent for one boot, reboots the unit and waits for it.",
                vec![facts::agent_command(&host, &user)],
            ),
            RecKind::RestoreFlash => {
                // only an image made for this unit (same serial); an unknown unit gets the placeholder, never "the newest of anything"
                let id = self.unit_id(cx);
                let image =
                    synofurb_core::recovery::backups_for_unit(&backups, &id).into_iter().next().map(|b| b.path.display().to_string());
                (
                    "Run on this computer, or on another Linux computer",
                    "Writes the saved image back. On another computer use the second line and replace /dev/sdX with the 128 MB flash; check twice.",
                    vec![
                        format!(
                            "synofurb-cli recover restore --host {} --backup {} --yes",
                            facts::shell_arg(&host),
                            image.as_deref().map(facts::shell_arg).unwrap_or_else(|| "<saved boot-flash image>".into())
                        ),
                        facts::manual_recovery_commands(image.as_deref())[1].clone(),
                    ],
                )
            }
            RecKind::RemoveEntries => (
                "Run on this computer, or on the unit",
                "Takes our entries out of the boot menu and sets the default to DSM.",
                vec![
                    format!("synofurb-cli recover remove --host {} --yes", facts::shell_arg(&host)),
                    "sudo synofurb-dom remove --yes".into(),
                ],
            ),
            RecKind::ReinstallDsm => return,
        };
        self.rec_result = Some(RecResult { kind: None, level: RecLevel::Info, title: title.into(), detail: detail.into(), commands });
        cx.notify();
    }

    pub fn open_manual_recovery(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let id = self.unit_id(cx);
        let backup = synofurb_core::recovery::backups_for_unit(&self.cfg_dir.join("dom-backups"), &id).into_iter().next().map(|b| b.path);
        let cmds = facts::manual_recovery_commands(backup.as_ref().map(|p| p.display().to_string()).as_deref());
        let app = cx.entity();
        window.open_dialog(cx, move |d, _, cx| {
            let app = app.clone();
            d.bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
                .title(copy::DLG_MANUAL.short)
                .w(px(crate::ui::theme::layout::DIALOG_W))
                .child(
                    v_flex()
                        .gap(px(8.0))
                        .child(paragraphs(copy::DLG_MANUAL.body, cx))
                        .child(kit::command_row(cx, "mr-1", cmds[0].clone()))
                        .child(kit::command_row(cx, "mr-2", cmds[1].clone()))
                        .child(kit::command_row(cx, "mr-3", cmds[2].clone())),
                )
                .footer(
                    DialogFooter::new()
                        .child(Button::new("mr-help").ghost().icon(icons::icon(role::HELP)).label(copy::BTN_OPEN_HELP.short).on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                app.update(cx, |a, cx| a.open_help_topic(Some("recovery"), window, cx));
                            },
                        ))
                        .child(
                            Button::new("mr-close")
                                .primary()
                                .label(copy::BTN_CLOSE.short)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
    }
}

type OnOk = std::rc::Rc<dyn Fn(&mut App, &mut Context<App>)>;

/// A dialog that needs a typed phrase before its confirm button enables (real disabled button + reason popover).
struct TypedConfirm {
    item: &'static copy::Item,
    body: String,
    phrase: String,
    input: Entity<InputState>,
    on_ok: OnOk,
    app: Entity<App>,
    _sub: gpui_kit::Subscription,
}

fn typed_confirm(
    window: &mut Window,
    cx: &mut gpui_kit::App,
    item: &'static copy::Item,
    body: String,
    phrase: String,
    app: Entity<App>,
    on_ok: impl Fn(&mut App, &mut Context<App>) + 'static,
) {
    let a2 = app.clone();
    let view = cx.new(|cx| {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(phrase.clone()));
        let sub = cx.subscribe(&input, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        TypedConfirm { item, body, phrase, input, on_ok: std::rc::Rc::new(on_ok), app: a2, _sub: sub }
    });
    window.open_dialog(cx, move |d, _, cx| {
        d.bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
            .title(item.short)
            .w(px(crate::ui::theme::layout::DIALOG_W))
            .child(view.clone())
    });
}

impl Render for TypedConfirm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let typed = self.input.read(cx).value().trim().to_string();
        let ok = typed == self.phrase;
        let (on_ok, app) = (self.on_ok.clone(), self.app.clone());
        let ok_btn =
            Button::new("tc-ok").danger().label(self.item.short.trim_end_matches('?').to_string()).on_click(move |_, window, cx| {
                window.close_dialog(cx);
                let on_ok = on_ok.clone();
                app.update(cx, |a, cx| on_ok(a, cx));
            });
        v_flex()
            .gap(px(8.0))
            .child(div().text_sm().child(self.body.clone()))
            .child(kit::mono(cx, self.phrase.clone()))
            .child(Input::new(&self.input).aria_label(copy::REC_TYPE.fmt(&[self.phrase.as_str()])))
            .child(
                h_flex()
                    .gap(px(8.0))
                    .justify_end()
                    .child(
                        Button::new("tc-cancel").outline().label(copy::BTN_CANCEL.short).on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(kit::with_reason("tc-why", ok_btn, (!ok).then(|| copy::REC_TYPE.fmt(&[self.phrase.as_str()])))),
            )
    }
}
