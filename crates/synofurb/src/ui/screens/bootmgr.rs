//! D13 Boot manager: install the one-shot SSH agent on the boot flash.

use crate::ui::app::{App, Job};
use crate::ui::copy;
use crate::ui::format::percent;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::{KeyMode, Screen};
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::Sizable;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::form::Form;
use gpui_kit::component::input::Input;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div, px};

impl App {
    pub(crate) fn page_bootmgr(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let avail = self.can_boot_manager();
        let tags = h_flex()
            .gap(px(8.0))
            .child(
                h_flex()
                    .gap(px(2.0))
                    .child(kit::tag(&p, Tone::Warn, Some(role::WARN), copy::BM_FLASH_CHANGES.short))
                    .child(crate::ui::disclosure::info_popover("bm-flash", &copy::BM_FLASH_CHANGES, &app)),
            )
            .child(
                h_flex()
                    .gap(px(2.0))
                    .child(kit::tag(&p, Tone::Ok, Some(role::SAFE), copy::BM_DISKS_UNTOUCHED.short))
                    .child(crate::ui::disclosure::info_tip("bm-disks", &copy::BM_DISKS_UNTOUCHED, &app)),
            );
        let key_ix = usize::from(self.sys.key_mode == KeyMode::Paste);
        let form = Form::vertical()
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
            ));
        let state: gpui_kit::AnyElement = match &self.bootmgr {
            Job::Idle => div().into_any_element(),
            Job::Running => {
                let (what, f) = self.install.progress.clone().unwrap_or(("".into(), 0.0));
                v_flex()
                    .gap(px(4.0))
                    .child(kit::caption(cx, what))
                    .child(
                        h_flex()
                            .gap(px(8.0))
                            .items_center()
                            .child(div().flex_1().child(Progress::new("bm-progress").value(f * 100.0)))
                            .child(kit::mono_sm(cx, percent(f))),
                    )
                    .into_any_element()
            }
            Job::Done(Ok(())) => Alert::success("bm-ok", "DSM is still the default. `synofurb-cli boot-manager enter` boots the agent.")
                .icon(icons::icon(role::OK))
                .into_any_element(),
            Job::Done(Err(e)) => Alert::error("bm-err", e.clone()).icon(icons::icon(role::ERR)).into_any_element(),
        };
        let mut body = v_flex().w_full().max_w(px(560.0)).gap(px(14.0)).child(tags).child(form);
        if let Err(why) = avail {
            body = body.child(Alert::error("bm-na", why).title(copy::BM_NOT_EDITABLE.short).icon(icons::icon(role::ERR)));
        }
        body = body.child(
            h_flex()
                .gap(px(2.0))
                .items_start()
                .child(Alert::warning("bm-other", copy::BM_OTHER_OS.short).small().icon(icons::icon(role::WARN)).flex_1())
                .child(crate::ui::disclosure::info_tip("bm-other-i", &copy::BM_OTHER_OS, &app)),
        );
        body = body.child(state);
        let running = self.bootmgr.running();
        Page::new(copy::BM_TITLE.short, body)
            .title_info(&copy::BM_TITLE)
            .subtitle(copy::BM_SUB.short)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
            ])
            .right(vec![
                Button::new("install-bm")
                    .primary()
                    .icon(icons::icon(role::DOWNLOAD))
                    .label(copy::BM_INSTALL.short)
                    .loading(running)
                    .disabled(avail.is_err() || running)
                    .tooltip(avail.err().unwrap_or(""))
                    .on_click(cx.listener(|this, _, _, cx| this.start_boot_manager(cx)))
                    .into_any_element(),
            ])
    }
}
