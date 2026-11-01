//! Recovery (approved mockup): the rows follow the facts this app knows from the session (it cannot read the boot flash
//! yet). A row that cannot work is a real disabled button with a reason popover beside it; when the unit never
//! answered, "Manual recovery" is the primary. Rows that need the unit's own tools show the exact command to run in a
//! neutral note (never a success message): this app does not run them.

use crate::ui::app::{App, Job};
use crate::ui::copy;
use crate::ui::disclosure::{info_tip, paragraphs};
use crate::ui::facts::{self, RecKind};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div, px};

/// What a recovery row shows after it was pressed: a neutral note and the commands to run. Never a success message.
#[derive(Clone, Debug)]
pub struct RecResult {
    pub title: String,
    pub detail: String,
    /// Commands to run, each with a Copy button.
    pub commands: Vec<String>,
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
        let running = self.recover.running();

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
            .child(kit::tag(
                &p,
                if uf.disks { Tone::Warn } else { Tone::Ok },
                Some(if uf.disks { role::WARN } else { role::SAFE }),
                if uf.disks { copy::REC_DISKS_MODIFIED.short } else { copy::REC_DISKS_UNTOUCHED.short },
            ));

        let mut body = v_flex().w_full().max_w(px(720.0)).gap(px(12.0)).child(chips);
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
            body = body.child(Alert::info("rec-none", copy::REC_NOTHING.tip).title(copy::REC_NOTHING.short).icon(icons::icon(role::INFO)));
        }
        if let Some(r) = &self.rec_result {
            body = body.child(Alert::info("rec-res", r.detail.clone()).title(r.title.clone()).icon(icons::icon(role::INFO)));
            for (i, cmd) in r.commands.iter().enumerate() {
                body = body.child(kit::command_row(cx, ("rec-cmd", i), cmd.clone()));
            }
        }
        match &self.recover {
            Job::Running => {
                body = body.child(Alert::info("rec-run", copy::REC_BOOT_RUNNING.short).small().icon(icons::icon(role::RUNNING)))
            }
            Job::Done(Ok(_)) => {
                body = body.child(
                    Alert::info("rec-ok", copy::REC_BOOT_DONE.tip).title(copy::REC_BOOT_DONE.short).small().icon(icons::icon(role::INFO)),
                )
            }
            Job::Done(Err(e)) => body = body.child(Alert::error("rec-err", e.clone()).small().icon(icons::icon(role::ERR))),
            _ => {}
        }

        let list = v_flex().rounded(cx.theme().radius).border_1().border_color(cx.theme().border).children(rows.iter().enumerate().map(
            |(i, r)| {
                let kind = r.kind;
                let mut b = Button::new(("rec-run", i))
                    .label(r.button)
                    .icon(icons::icon(rec_icon(kind)))
                    .accessibility_label(format!("{}: {}", r.button, r.label));
                b = if primary == Some(kind) { b.primary() } else { b.outline() };
                if kind == RecKind::BootDsm {
                    b = b.loading(running);
                }
                let b = b.on_click(cx.listener(move |this, _, window, cx| this.run_recovery_row(kind, window, cx)));
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
        Page::new(copy::REC_TITLE.short, body).subtitle(copy::REC_SUB.short).subtitle_tip(&copy::REC_SUB).left(vec![
            Button::new("back")
                .ghost()
                .icon(icons::icon(role::BACK))
                .label(copy::BTN_BACK.short)
                .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                .into_any_element(),
        ])
    }

    /// Run one Recovery row. Boot DSM asks first and then really calls the engine (it only sets the default; the unit
    /// still has to be rebooted). Every other row shows the exact command: this app does not run it.
    pub fn run_recovery_row(&mut self, kind: RecKind, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let backups = self.cfg_dir.join("dom-backups");
        match kind {
            RecKind::BootDsm => {
                self.rec_result = None;
                let app2 = app.clone();
                crate::ui::disclosure::dialog_with(window, cx, &copy::DLG_BOOT_DSM, &app, copy::REC_BOOT_DSM_OK.short, move |cx| {
                    app2.update(cx, |a, cx| a.start_recover(cx))
                });
            }
            RecKind::Agent => {
                let host = self.input_value(&self.inp.host, cx);
                let host = if host.trim().is_empty() { self.selected_ip.clone().unwrap_or_default() } else { host.trim().to_string() };
                let user = self.input_value(&self.inp.user, cx);
                let user = if user.trim().is_empty() { "admin".to_string() } else { user.trim().to_string() };
                self.rec_result = Some(RecResult {
                    title: "Run on this computer".into(),
                    detail:
                        "This app does not start the agent. If it starts, its watchdog reboots the unit into DSM after the login timeout."
                            .into(),
                    commands: vec![facts::agent_command(&host, &user)],
                });
                cx.notify();
            }
            RecKind::RestoreFlash => {
                let cmds = facts::manual_recovery_commands(facts::latest_backup(&backups).map(|p| p.display().to_string()).as_deref());
                self.rec_result = Some(RecResult {
                    title: "Run on another Linux computer".into(),
                    detail: "This app does not write the flash. Attach the boot flash to a Linux computer and copy the saved image there. Replace /dev/sdX with the 128 MB flash; check twice."
                        .into(),
                    commands: vec![cmds[1].clone()],
                });
                cx.notify();
            }
            RecKind::RemoveEntries => {
                self.rec_result = Some(RecResult {
                    title: "Run on the unit".into(),
                    detail: "This app does not remove the entries. Run it on DSM, Debian or the agent. DSM stays the default.".into(),
                    commands: vec!["sudo synofurb-dom remove --yes".into()],
                });
                cx.notify();
            }
            RecKind::ReinstallDsm => {
                window.open_dialog(cx, move |d, _, cx| {
                    d.title(copy::DLG_REINSTALL.short)
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

    pub fn open_manual_recovery(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let backup = facts::latest_backup(&self.cfg_dir.join("dom-backups"));
        let cmds = facts::manual_recovery_commands(backup.as_ref().map(|p| p.display().to_string()).as_deref());
        let app = cx.entity();
        window.open_dialog(cx, move |d, _, cx| {
            let app = app.clone();
            d.title(copy::DLG_MANUAL.short)
                .w(px(crate::ui::theme::layout::DIALOG_W))
                .child(
                    v_flex()
                        .gap(px(8.0))
                        .child(paragraphs(copy::DLG_MANUAL.body, cx))
                        .child(kit::command_row(cx, "mr-1", cmds[0].clone()))
                        .child(kit::command_row(cx, "mr-2", cmds[1].clone())),
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
