//! Boot manager: confirm, progress, result. The agent is one-shot and DSM stays the default; every state says what was
//! changed (the flash) and what was not (the disks).

use crate::ui::app::{App, Job};
use crate::ui::copy;
use crate::ui::disclosure::{dialog_with, info_popover, info_tip};
use crate::ui::format::percent;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::{KeyMode, Screen};
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::form::Form;
use gpui_kit::component::input::Input;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div, px};

impl App {
    pub(crate) fn page_bootmgr(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let avail = self.can_boot_manager();
        let ip = crate::ui::format::display_host(&self.input_value(&self.inp.host, cx));
        let idle = matches!(self.bootmgr, Job::Idle);
        let running = self.bootmgr.running();
        let done = matches!(self.bootmgr, Job::Done(Ok(())));

        let tags = h_flex()
            .gap(px(8.0))
            .child(
                h_flex().gap(px(2.0)).child(kit::tag(&p, Tone::Warn, Some(role::WARN), copy::BM_FLASH_CHANGES.short)).child(info_popover(
                    "bm-flash",
                    &copy::BM_FLASH_CHANGES,
                    &app,
                )),
            )
            .child(h_flex().gap(px(2.0)).child(kit::tag(&p, Tone::Ok, Some(role::SAFE), copy::BM_DISKS_UNTOUCHED.short)).child(info_tip(
                "bm-disks",
                &copy::BM_DISKS_UNTOUCHED,
                &app,
            )));

        let mut body = v_flex().w_full().max_w(px(640.0)).gap(px(12.0)).child(tags);
        let card: gpui_kit::AnyElement = if idle {
            let key_ix = usize::from(self.sys.key_mode == KeyMode::Paste);
            v_flex()
                .gap(px(12.0))
                .child(
                    Form::vertical()
                        .child(kit::field(
                            "bm-key",
                            &copy::BM_SSH_KEY,
                            &app,
                            v_flex()
                                .gap(px(8.0))
                                .child(kit::segmented("bm-key-mode", &[copy::SYS_KEY_GENERATE.short, copy::SYS_KEY_PASTE.short], key_ix, {
                                    let app = app.clone();
                                    move |ix, _, cx| {
                                        app.update(cx, |a, cx| {
                                            a.sys.key_mode = if ix == 1 { KeyMode::Paste } else { KeyMode::Generate };
                                            cx.notify();
                                        })
                                    }
                                }))
                                .when(self.sys.key_mode == KeyMode::Paste, |d| d.child(Input::new(&self.inp.pubkey))),
                        ))
                        .child(kit::field(
                            "bm-timeout",
                            &copy::BM_LOGIN_TIMEOUT,
                            &app,
                            div().w(px(160.0)).child(Input::new(&self.inp.bm_timeout).readonly(true)),
                        )),
                )
                .child(
                    h_flex()
                        .gap(px(2.0))
                        .items_start()
                        .child(Alert::warning("bm-other", copy::BM_AFTER_OS.short).small().icon(icons::icon(role::WARN)).flex_1().min_w_0())
                        .child(info_tip("bm-other-i", &copy::BM_OTHER_OS, &app)),
                )
                .into_any_element()
        } else if running {
            let (_, f) = self.install.progress.clone().unwrap_or(("".into(), 0.0));
            let steps = [&copy::BM_STEP1, &copy::BM_STEP2, &copy::BM_STEP3, &copy::BM_STEP4];
            let reached = ((f * 4.0) as usize).min(4);
            v_flex()
                .gap(px(8.0))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::strong(copy::INSTALL_FLASH.short))
                        .child(kit::tag(&p, Tone::Info, Some(role::RUNNING), "Working"))
                        .child(div().flex_1())
                        .child(kit::mono_sm(cx, percent(f))),
                )
                .child(Progress::new("bm-progress").value(f * 100.0))
                .children(steps.iter().enumerate().map(|(i, s)| {
                    let d = i < reached;
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(icons::icon_sized(if d { role::OK } else { role::PENDING }, 16.0, if d { p.ok_fg } else { p.placeholder }))
                        .child(div().text_sm().child(s.short))
                }))
                .into_any_element()
        } else if let Job::Done(Err(e)) = &self.bootmgr {
            Alert::error("bm-err", e.clone()).icon(icons::icon(role::ERR)).into_any_element()
        } else {
            let cmd = format!("ssh boot@{ip}");
            v_flex()
                .gap(px(12.0))
                .child(h_flex().gap(px(8.0)).flex_wrap().child(kit::tag(&p, Tone::Ok, Some(role::OK), copy::BM_READY.short)).child(
                    h_flex().gap(px(2.0)).child(kit::tag(&p, Tone::Neutral, Some(role::CLOCK), copy::BM_WATCHDOG.short)).child(info_tip(
                        "bm-wd",
                        &copy::BM_WATCHDOG,
                        &app,
                    )),
                ))
                .child(
                    h_flex()
                        .gap(px(4.0))
                        .items_center()
                        .child(kit::mono(cx, cmd.clone()))
                        .child(Clipboard::new("bm-copy").value(cmd).tooltip("Copy")),
                )
                .children(
                    self.rec_running.map(|_| {
                        Alert::info("bm-back-run", copy::REC_BOOT_RUNNING.short).icon(icons::icon(role::RUNNING)).into_any_element()
                    }),
                )
                .children(
                    self.rec_result
                        .as_ref()
                        .filter(|r| r.kind == Some(crate::ui::facts::RecKind::BootDsm))
                        .map(|r| kit::rec_alert("bm-back", r.level, &r.title, &r.detail)),
                )
                .into_any_element()
        };
        body = body.child(card);
        if let Err(why) = avail {
            body = body.child(Alert::error("bm-na", why).title(copy::BM_NOT_EDITABLE.short).icon(icons::icon(role::ERR)));
        }

        let mut page = Page::new(copy::BM_TITLE.short, body).title_info(&copy::BM_TITLE).subtitle(copy::BM_SUB.short);
        if idle {
            page = page
                .left(vec![
                    Button::new("back")
                        .ghost()
                        .icon(icons::icon(role::BACK))
                        .label(copy::BTN_BACK.short)
                        .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                        .into_any_element(),
                ])
                .right(vec![kit::with_reason(
                    "bm-why",
                    Button::new("install-bm").primary().icon(icons::icon(role::DOWNLOAD)).label(copy::BM_INSTALL.short).on_click(
                        cx.listener(|_this, _, window, cx| {
                            let app = cx.entity();
                            let app2 = app.clone();
                            dialog_with(window, cx, &copy::DLG_BM_CONFIRM, &app, copy::BM_INSTALL.short, move |cx| {
                                app2.update(cx, |a, cx| a.start_boot_manager(cx));
                            });
                        }),
                    ),
                    avail.err().map(String::from),
                )]);
        } else if running {
            let began = self.install.progress.as_ref().is_some_and(|(_, f)| *f >= 0.25);
            page = page.right(vec![kit::with_reason(
                "bm-cancel-why",
                Button::new("bm-cancel").outline().label(copy::BTN_CANCEL.short),
                began.then(|| copy::BM_CANCEL_WHY.short.to_string()),
            )]);
        } else if done {
            let returned = self.rec_result.as_ref().is_some_and(|r| r.kind == Some(crate::ui::facts::RecKind::BootDsm) && r.level == crate::ui::worker::RecLevel::Done);
            page = page.right(vec![
                if returned {
                    Button::new("bm-rec")
                        .outline()
                        .label(copy::FAIL_RECOVERY.short)
                        .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Recover, window, cx)))
                        .into_any_element()
                } else {
                    Button::new("bm-return")
                        .outline()
                        .icon(icons::icon(role::ROTATE_BACK))
                        .label(copy::BM_RETURN.short)
                        .tooltip(copy::BM_RETURN.tip)
                        .loading(self.rec_running.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.open_boot_dsm_dialog(window, cx)))
                        .into_any_element()
                },
                Button::new("bm-done")
                    .primary()
                    .icon(icons::icon(role::CHECK))
                    .label(copy::BM_DONE.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Inventory, window, cx)))
                    .into_any_element(),
            ]);
        }
        page
    }
}
