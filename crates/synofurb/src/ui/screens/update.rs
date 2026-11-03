//! D11 Update dialog: release notes, size, date, download progress and the signature state. A small view that
//! observes the app, so progress and verification update live inside the dialog. All checks live in
//! `synofurb_core::updater`; this file only shows state and forwards clicks.

use crate::ui::app::{App, UpdateState};
use crate::ui::copy;
use crate::ui::format::{human, percent};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::theme::{Themed, Tone, layout};
use crate::ui::updates;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, Context, Entity, IntoElement, Render, Window, div, px};
use synofurb_core::updater::{self, InstallMode};

pub struct UpdateView {
    app: Entity<App>,
    /// "I understand and want to install it": required before an unsigned release can be downloaded.
    consent: bool,
    _sub: gpui_kit::Subscription,
}

impl UpdateView {
    pub fn new(app: Entity<App>, cx: &mut Context<UpdateView>) -> UpdateView {
        let sub = cx.observe(&app, |_, _, cx| cx.notify());
        UpdateView { app, consent: false, _sub: sub }
    }
}

impl Render for UpdateView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.pal();
        let app = self.app.clone();
        let consent = self.consent;
        let a = app.read(cx);
        let Some(info) = a.update_info().cloned() else {
            return v_flex().child(kit::muted(cx, "-")).into_any_element();
        };
        let mode = a.install_mode(&info);
        let unsigned = matches!(mode, InstallMode::UnsignedWithConsent);
        let install_running = a.install_running();
        let size = info.asset.as_ref().map(|x| human(x.size)).unwrap_or_else(|| "-".into());
        let published = info.published_at.clone().unwrap_or_default().split('T').next().unwrap_or("").to_string();

        let meta = h_flex()
            .gap(px(8.0))
            .items_center()
            .flex_wrap()
            .child(kit::chip(&p, format!("{} to {}", info.current, info.latest)))
            .child(kit::chip(&p, size))
            .when(!published.is_empty(), |d| d.child(kit::chip(&p, published)))
            .when(info.prerelease, |d| d.child(kit::tag(&p, Tone::Warn, Some(role::WARN), "Pre-release")));
        // Release notes as plain text (the engine strips the Markdown), in their own scroll area: a long list never
        // pushes the buttons off the dialog.
        let notes = div()
            .id("update-notes")
            .max_h(px(180.0))
            .w_full()
            .p(px(8.0))
            .rounded(cx.theme().radius)
            .bg(cx.theme().muted)
            .child(v_flex().gap(px(2.0)).text_sm().children(info.notes_plain().lines().map(|l| {
                let l = l.trim_end().to_string();
                if l.is_empty() { div().h(px(4.0)) } else { div().child(l) }
            })))
            .overflow_y_scrollbar();
        let mut col = v_flex().w_full().gap(px(12.0)).child(meta).child(notes);
        // ONE warning: the unsigned line, or why the release cannot be installed at all
        match &mode {
            InstallMode::UnsignedWithConsent => {
                col = col.child(Alert::warning("upd-unsigned", copy::UPD_ST_UNSIGNED.short).small().icon(icons::icon(role::WARN)));
            }
            InstallMode::Blocked(why) => {
                col = col.child(Alert::warning("upd-blocked", why.clone()).small().icon(icons::icon(role::WARN)));
            }
            InstallMode::Signed => {}
        }

        let waiting = matches!(a.update, UpdateState::Available(_) | UpdateState::Failed(..));
        if unsigned && waiting {
            col = col.child(
                h_flex()
                    .gap(px(4.0))
                    .items_center()
                    .child(Checkbox::new("upd-consent").label(copy::UPD_CONSENT.short).checked(consent).on_click(cx.listener(
                        |v, checked: &bool, _, cx| {
                            v.consent = *checked;
                            cx.notify();
                        },
                    )))
                    .child(crate::ui::disclosure::info_tip("consent-info", &copy::UPD_CONSENT_INFO, &app)),
            );
        }

        let (state_ui, buttons): (gpui_kit::AnyElement, Vec<gpui_kit::AnyElement>) = match &a.update {
            UpdateState::Downloading(_, prog) => {
                let frac = prog.total.filter(|t| *t > 0).map(|t| prog.downloaded as f32 / t as f32).unwrap_or(0.0);
                (
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .child(Progress::new("upd-progress").value(frac * 100.0).accessibility_label("Download progress")),
                        )
                        .child(kit::mono_sm(
                            cx,
                            format!(
                                "{} / {} \u{00b7} {}",
                                human(prog.downloaded),
                                prog.total.map(human).unwrap_or_else(|| "?".into()),
                                percent(frac)
                            ),
                        ))
                        .into_any_element(),
                    vec![],
                )
            }
            UpdateState::Verified(_, vd) => {
                let app2 = app.clone();
                let sha: String = vd.sha256.chars().take(12).collect();
                let tag = if vd.signed {
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .child(kit::tag(&p, Tone::Ok, Some(role::OK), format!("{} \u{00b7} {sha}", copy::UPD_SIGNATURE.short)))
                        .child(crate::ui::disclosure::info_tip("sig-info", &copy::UPD_SIGNATURE, &app))
                } else {
                    // calm, and honest: the checksum matched, nothing was signed
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .child(kit::tag(
                            &p,
                            Tone::Neutral,
                            Some(role::CHECK),
                            format!("{} \u{00b7} {sha}", copy::UPD_VERIFIED_UNSIGNED.short),
                        ))
                        .child(crate::ui::disclosure::info_tip("sig-info", &copy::UPD_VERIFIED_UNSIGNED, &app))
                };
                (
                    tag.into_any_element(),
                    vec![kit::with_reason(
                        "apply-why",
                        Button::new("upd-apply")
                            .primary()
                            .label(copy::UPD_RESTART.short)
                            .on_click(move |_, _, cx| app2.update(cx, |a, cx| a.apply_update(cx))),
                        install_running.then(|| copy::UPD_RESTART.tip.to_string()),
                    )],
                )
            }
            UpdateState::Failed(_, e) => {
                let app2 = app.clone();
                (
                    Alert::error("upd-fail", e.clone()).title(copy::UPD_DISCARDED.short).icon(icons::icon(role::ERR)).into_any_element(),
                    vec![
                        Button::new("upd-retry")
                            .outline()
                            .label("Try again")
                            .disabled(updates::download_blocker(&mode, consent).is_some())
                            .on_click(move |_, _, cx| app2.update(cx, |a, cx| a.start_update_download(consent, cx)))
                            .into_any_element(),
                    ],
                )
            }
            _ => {
                let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
                let label = if unsigned { &copy::UPD_DOWNLOAD_INSTALL } else { &copy::UPD_DOWNLOAD };
                (
                    div().into_any_element(),
                    vec![
                        Button::new("upd-skip")
                            .ghost()
                            .label(copy::UPD_SKIP.short)
                            .on_click(move |_, window, cx| {
                                a1.update(cx, |a, cx| a.skip_version(cx));
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        Button::new("upd-later")
                            .outline()
                            .label(copy::UPD_LATER.short)
                            .on_click(move |_, window, cx| {
                                a2.update(cx, |a, cx| a.remind_tomorrow(cx));
                                window.close_dialog(cx);
                            })
                            .into_any_element(),
                        kit::with_reason(
                            "download-why",
                            Button::new("upd-download")
                                .primary()
                                .icon(icons::icon(role::DOWNLOAD))
                                .label(label.short)
                                .on_click(move |_, _, cx| a3.update(cx, |a, cx| a.start_update_download(consent, cx))),
                            updates::download_blocker(&mode, consent),
                        ),
                    ],
                )
            }
        };
        let url = Some(updates::release_url(&info, updater::DEFAULT_REPO));
        col.child(state_ui)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(div().flex_1().min_w(px(120.0)).when_some(url, |d, u| {
                        d.child(
                            Button::new("upd-page")
                                .link()
                                .small()
                                .icon(icons::icon(role::EXTERNAL))
                                .label("Release page")
                                .on_click(move |_, _, cx| cx.open_url(&u)),
                        )
                    }))
                    .child(h_flex().gap(px(8.0)).items_center().flex_wrap().children(buttons)),
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
        let title = self.update_info().map(|i| format!("{} {}", copy::UPD_BANNER.short, i.latest)).unwrap_or_default();
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
                .title(title.clone())
                .w(px(layout::DIALOG_W_WIDE))
                .child(view.clone())
        });
    }
}
