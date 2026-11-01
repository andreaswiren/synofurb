//! Recovery (approved mockup): the rows follow the unit's facts. A row that cannot work is a real disabled button with a
//! reason popover beside it; the destructive rows ask for a typed phrase; when the unit never answered, "Manual
//! recovery" is the primary. Rows that need the unit's own tools show the exact command to run (no desktop worker).

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
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, AppContext, Context, Entity, IntoElement, Render, SharedString, Window, div, px};

/// What a finished recovery action reports (an Alert under the chips).
#[derive(Clone, Debug)]
pub struct RecResult {
    pub ok: bool,
    pub title: String,
    pub detail: String,
    /// A command to run on the unit, shown with a Copy button.
    pub command: Option<String>,
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

        let mut body = v_flex().w_full().max_w(px(720.0)).gap(px(14.0)).child(chips);
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
            let a = if r.ok { Alert::success("rec-res", r.detail.clone()) } else { Alert::error("rec-res", r.detail.clone()) };
            body = body.child(a.title(r.title.clone()).icon(icons::icon(if r.ok { role::OK } else { role::ERR })));
            if let Some(cmd) = r.command.clone() {
                body = body.child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::mono(cx, cmd.clone()))
                        .child(gpui_kit::component::clipboard::Clipboard::new("rec-cmd").value(cmd).tooltip(copy::REC_COPY_CMD.short)),
                );
            }
        }
        match &self.recover {
            Job::Done(Ok(s)) => body = body.child(Alert::success("rec-ok", s.trim().to_string()).small().icon(icons::icon(role::OK))),
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
                    .gap(px(10.0))
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
        Page::new(copy::REC_TITLE.short, body).subtitle(copy::REC_SUB.short).left(vec![
            Button::new("back")
                .ghost()
                .icon(icons::icon(role::BACK))
                .label(copy::BTN_BACK.short)
                .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                .into_any_element(),
        ])
    }

    /// Run one Recovery row: Boot DSM through the engine; the destructive rows ask for their phrase first; rows that
    /// need the unit's own tools produce the exact command instead of pretending to run it.
    pub fn run_recovery_row(&mut self, kind: RecKind, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let backup = self.cfg_dir.join("dom-backups");
        match kind {
            RecKind::BootDsm => {
                self.rec_result = None;
                self.start_recover(cx);
            }
            RecKind::Agent => {
                self.rec_result = Some(RecResult {
                    ok: true,
                    title: "Run on this computer".into(),
                    detail: "The agent boots once and returns to DSM by itself.".into(),
                    command: Some("synofurb-cli boot-manager enter --host <ip> --user <admin>".into()),
                });
                cx.notify();
            }
            RecKind::RestoreFlash => {
                let cmd = format!("synofurb-dom restore {}/<backup>.img", backup.display());
                typed_confirm(window, cx, &copy::DLG_RESTORE, facts::RESTORE_PHRASE, app, move |a, cx| {
                    a.rec_result = Some(RecResult {
                        ok: true,
                        title: "Run on the unit".into(),
                        detail: "The engine restores the flash on the unit itself.".into(),
                        command: Some(cmd.clone()),
                    });
                    cx.notify();
                });
            }
            RecKind::RemoveEntries => {
                typed_confirm(window, cx, &copy::DLG_REMOVE, facts::REMOVE_PHRASE, app, |a, cx| {
                    a.rec_result = Some(RecResult {
                        ok: true,
                        title: "Run on the unit".into(),
                        detail: "After it, the flash holds only Synology's own files.".into(),
                        command: Some("synofurb-dom remove".into()),
                    });
                    cx.notify();
                });
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
                        .child(Button::new("mr-close").primary().label(copy::BTN_CLOSE.short).on_click(|_, window, cx| window.close_dialog(cx))),
                )
        });
    }
}

type OnOk = std::rc::Rc<dyn Fn(&mut App, &mut Context<App>)>;

/// A dialog that needs a typed phrase before its confirm button enables (real disabled button + reason popover).
struct TypedConfirm {
    item: &'static copy::Item,
    phrase: &'static str,
    input: Entity<InputState>,
    on_ok: OnOk,
    app: Entity<App>,
    _sub: gpui_kit::Subscription,
}

fn typed_confirm(
    window: &mut Window,
    cx: &mut gpui_kit::App,
    item: &'static copy::Item,
    phrase: &'static str,
    app: Entity<App>,
    on_ok: impl Fn(&mut App, &mut Context<App>) + 'static,
) {
    let a2 = app.clone();
    let view = cx.new(|cx| {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(phrase));
        let sub = cx.subscribe(&input, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        TypedConfirm { item, phrase, input, on_ok: std::rc::Rc::new(on_ok), app: a2, _sub: sub }
    });
    window.open_dialog(cx, move |d, _, _| d.title(item.short).w(px(crate::ui::theme::layout::DIALOG_W)).child(view.clone()));
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
            .gap(px(10.0))
            .child(div().text_sm().child(self.item.body))
            .child(kit::mono(cx, self.phrase.to_string()))
            .child(Input::new(&self.input).aria_label(copy::REC_TYPE.fmt(&[self.phrase])))
            .child(
                h_flex()
                    .gap(px(8.0))
                    .justify_end()
                    .child(
                        Button::new("tc-cancel").outline().label(copy::BTN_CANCEL.short).on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(kit::with_reason("tc-why", ok_btn, (!ok).then(|| copy::REC_TYPE.fmt(&[self.phrase])))),
            )
    }
}

#[allow(dead_code)]
fn _s(_: SharedString, _: AnyElement) {}
