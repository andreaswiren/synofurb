//! "Update available" dialog. A small view that observes the app, so download progress and verification update live
//! inside the dialog. Uses `synofurb_core::updater` for everything: this file only shows state and forwards clicks.

use crate::ui::app::{App, UpdateState};
use crate::ui::format::{human, percent};
use crate::ui::theme::{self, Themed, Tone, layout, radius, space};
use crate::ui::widgets::*;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, Context, Entity, FontWeight, IntoElement, Render, Role, Window, div, px};
use synofurb_core::updater;

pub struct UpdateView {
    app: Entity<App>,
    _sub: gpui_kit::Subscription,
}

impl UpdateView {
    pub fn new(app: Entity<App>, cx: &mut Context<UpdateView>) -> UpdateView {
        let sub = cx.observe(&app, |_, _, cx| cx.notify());
        UpdateView { app, _sub: sub }
    }
}

impl Render for UpdateView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.pal();
        let app = self.app.clone();
        let a = app.read(cx);
        let Some(info) = a.update_info().cloned() else {
            return v_flex().child(muted(&p, "You are up to date.")).into_any_element();
        };
        let key_missing = updater::signing_key_is_placeholder(updater::UPDATE_PUBLIC_KEY);
        let install_running = a.install_running();
        let size = info.asset.as_ref().map(|x| human(x.size)).unwrap_or_else(|| "no file for this system".into());
        let published = info.published_at.clone().unwrap_or_default();
        let published = published.split('T').next().unwrap_or("").to_string();

        let meta = h_flex()
            .gap(px(space::S8))
            .flex_wrap()
            .child(chip(&p, format!("{} -> {}", info.current, info.latest)))
            .child(chip(&p, size))
            .when(!published.is_empty(), |d| d.child(chip(&p, format!("published {published}"))))
            .when(info.prerelease, |d| d.child(badge(&p, Tone::Warn, "triangle-alert", "Pre-release", false)));
        let notes = div()
            .id("update-notes")
            .max_h(px(200.0))
            .w_full()
            .p(px(space::S12))
            .rounded(px(radius::MD))
            .bg(p.surface_2)
            .border_1()
            .border_color(p.border)
            .overflow_y_scroll()
            .child(v_flex().gap(px(space::S4)).children(info.notes.lines().map(|l| {
                let l = l.trim_end();
                if let Some(h) = l.strip_prefix("## ") {
                    text(&p, theme::BODY, h.to_string()).font_weight(FontWeight(700.0)).mt(px(space::S4)).into_any_element()
                } else if l.is_empty() {
                    div().h(px(4.0)).into_any_element()
                } else {
                    muted(&p, l.to_string()).into_any_element()
                }
            })));

        let mut col = v_flex().w_full().gap(px(space::S12)).child(meta).child(notes);
        if key_missing {
            col = col.child(banner(&p, Tone::Warn, "Installing updates is switched off in this build", "The release signing key has not been configured yet (placeholder key), so a download cannot be verified. synofurb never installs an unverified file. Use the release page to download manually.", vec![]));
        }
        for pr in &info.problems {
            col = col.child(banner(&p, Tone::Warn, "Cannot be installed automatically", pr.clone(), vec![]));
        }

        let (state_ui, buttons): (gpui_kit::AnyElement, Vec<gpui_kit::AnyElement>) = match &a.update {
            UpdateState::Downloading(_, prog) => {
                let frac = prog.total.filter(|t| *t > 0).map(|t| prog.downloaded as f32 / t as f32).unwrap_or(0.0);
                (
                    v_flex()
                        .gap(px(space::S4))
                        .child(muted(
                            &p,
                            format!(
                                "Downloading and verifying... {} of {}",
                                human(prog.downloaded),
                                prog.total.map(human).unwrap_or_else(|| "?".into())
                            ),
                        ))
                        .child(
                            h_flex()
                                .gap(px(space::S8))
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .child(Progress::new("upd-progress").value(frac * 100.0).accessibility_label("Download progress")),
                                )
                                .child(text(&p, theme::CAPTION, percent(frac))),
                        )
                        .into_any_element(),
                    vec![],
                )
            }
            UpdateState::Verified(_, vd) => {
                let blocked = install_running;
                let app2 = app.clone();
                (
                    banner(
                        &p,
                        Tone::Ok,
                        "Verified",
                        format!(
                            "The signature of SHA256SUMS is valid and the SHA-256 matches ({}...).",
                            vd.sha256.chars().take(16).collect::<String>()
                        ),
                        vec![],
                    )
                    .into_any_element(),
                    vec![
                        btn_primary("upd-apply", "Restart and update")
                            .disabled(blocked)
                            .tooltip(if blocked { "Not while an installation is running" } else { "" })
                            .on_click(move |_, _, cx| app2.update(cx, |a, cx| a.apply_update(cx)))
                            .into_any_element(),
                    ],
                )
            }
            UpdateState::Failed(_, e) => {
                let app2 = app.clone();
                (
                    banner(&p, Tone::Err, "The update was not installed", e.clone(), vec![])
                        .id("upd-fail")
                        .role(Role::Alert)
                        .into_any_element(),
                    vec![
                        btn_secondary("upd-retry", "Try again")
                            .on_click(move |_, _, cx| app2.update(cx, |a, cx| a.start_update_download(cx)))
                            .into_any_element(),
                    ],
                )
            }
            _ => {
                let (app_a, app_b, app_c) = (app.clone(), app.clone(), app.clone());
                let dis = key_missing || !info.installable();
                (
                    div().into_any_element(),
                    vec![
                        btn_ghost("upd-skip", "Skip this version")
                            .on_click(move |_, window, cx| {
                                app_a.update(cx, |a, cx| a.skip_version(cx));
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        btn_secondary("upd-later", "Remind me tomorrow")
                            .on_click(move |_, window, cx| {
                                app_b.update(cx, |a, cx| a.remind_tomorrow(cx));
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        btn_primary("upd-download", "Download and verify")
                            .icon(icon("download"))
                            .disabled(dis)
                            .on_click(move |_, _, cx| app_c.update(cx, |a, cx| a.start_update_download(cx)))
                            .into_any_element(),
                    ],
                )
            }
        };
        let url = info.html_url.clone();
        col.child(state_ui)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(space::S8))
                    .pt(px(space::S4))
                    .child(div().flex_1().when_some(url, |d, u| {
                        d.child(
                            btn_ghost("upd-page", "Release page")
                                .small()
                                .icon(icon("external-link"))
                                .on_click(move |_, _, cx| cx.open_url(&u)),
                        )
                    }))
                    .children(buttons),
            )
            .into_any_element()
    }
}

impl App {
    pub fn open_update_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        if self.update_info().is_none() {
            return;
        }
        let app = cx.entity();
        let view = cx.new(|cx| UpdateView::new(app, cx));
        let title = self.update_info().map(|i| format!("synofurb {} is available", i.latest)).unwrap_or_default();
        window.open_dialog(cx, move |dialog, _, _| dialog.title(title.clone()).w(px(layout::DIALOG_W + 120.0)).child(view.clone()));
    }
}
