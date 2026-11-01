//! Settings and About: theme, text size, motion, updates, the compatibility list, logs, keyboard shortcuts.

use crate::ui::app::{App, UpdateState, version};
use crate::ui::render::Page;
use crate::ui::theme::{self, TextScale, ThemeChoice, Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, px};
use synofurb_core::compat::DbOrigin;

impl App {
    pub(crate) fn page_settings(&mut self, window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();

        let theme_sel = ThemeChoice::ALL.iter().position(|t| *t == self.settings.theme);
        let theme_group = RadioGroup::horizontal("theme-choice")
            .selected_index(theme_sel)
            .children(ThemeChoice::ALL.iter().map(|t| Radio::new(t.label()).label(t.label())))
            .on_click(cx.listener(|this, ix: &usize, _, cx| this.set_theme(ThemeChoice::ALL[*ix], cx)));
        let scale_sel = TextScale::ALL.iter().position(|t| *t == self.settings.text_scale);
        let scale_group = RadioGroup::horizontal("text-scale")
            .selected_index(scale_sel)
            .children(TextScale::ALL.iter().map(|t| Radio::new(t.label()).label(t.label())))
            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                let s = TextScale::ALL[*ix];
                this.settings.text_scale = s;
                this.save_settings();
                window.set_rem_size(gpui_kit::px(s.rem_px()));
                cx.notify();
            }));
        let _ = window;
        let appearance = v_flex()
            .gap(px(14.0))
            .child(form_row(&p, "Theme", Some("Follow system uses the operating system's light or dark setting.".into()), theme_group))
            .child(form_row(&p, "Text size", Some("Scales all text. Layouts reflow down to the minimum window size.".into()), scale_group))
            .child(form_row(
                &p,
                "Motion",
                None,
                Switch::new("reduce-motion")
                    .label("Reduce motion: show a static glyph instead of the spinner")
                    .checked(self.settings.reduce_motion)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.settings.reduce_motion = *v;
                        this.save_settings();
                        cx.notify();
                    })),
            ));

        // ---- updates
        let status: gpui_kit::AnyElement = match &self.update {
            UpdateState::Unknown => muted(&p, "Not checked yet.").into_any_element(),
            UpdateState::Checking => muted(&p, "Checking...").into_any_element(),
            UpdateState::UpToDate => {
                badge(&p, Tone::Ok, "circle-check", format!("synofurb {} is up to date", version()), false).into_any_element()
            }
            UpdateState::CheckFailed(e) => {
                badge(&p, Tone::Warn, "triangle-alert", format!("Could not check: {}", e.chars().take(90).collect::<String>()), false)
                    .into_any_element()
            }
            UpdateState::Available(i) | UpdateState::Downloading(i, _) | UpdateState::Verified(i, _) | UpdateState::Failed(i, _) => {
                h_flex()
                    .gap(px(space::S8))
                    .items_center()
                    .child(badge(&p, Tone::Accent, "sparkles", format!("{} is available", i.latest), false))
                    .child(
                        btn_secondary("show-update", "View details")
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| this.open_update_dialog(window, cx))),
                    )
                    .into_any_element()
            }
        };
        let chan_sel = if self.settings.update_channel == "prerelease" { 1 } else { 0 };
        let channel = RadioGroup::horizontal("update-channel")
            .selected_index(Some(chan_sel))
            .child(Radio::new("ch-stable").label("Stable"))
            .child(Radio::new("ch-pre").label("Pre-releases too"))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.settings.update_channel = if *ix == 1 { "prerelease".into() } else { "stable".into() };
                this.save_settings();
                cx.notify();
            }));
        let updates = v_flex()
            .gap(px(14.0))
            .child(form_row(&p, "Check for updates", Some("Looks at the project's releases on GitHub; nothing is installed without your click, and only verified files can be installed.".into()), Switch::new("check-updates").label("Check when the app starts").checked(self.settings.check_updates).on_click(cx.listener(|this, v: &bool, _, cx| {
                this.settings.check_updates = *v;
                this.save_settings();
                cx.notify();
            }))))
            .child(form_row(&p, "Channel", None, channel))
            .child(form_row(&p, "Status", None, h_flex().gap(px(space::S8)).items_center().child(status).child(btn_secondary("check-now", "Check now").small().icon(icon("refresh-cw")).on_click(cx.listener(|this, _, _, cx| this.check_updates_now(cx))))));

        // ---- compatibility list
        let origin = match self.compat.origin() {
            DbOrigin::Embedded => "built into this version",
            DbOrigin::Cache => "downloaded and cached",
        };
        let counts = h_flex().gap(px(space::S8)).flex_wrap().children(
            self.compat.count_by_status().into_iter().filter(|(_, n)| *n > 0).map(|(s, n)| {
                h_flex()
                    .gap(px(space::S4))
                    .items_center()
                    .child(compat_badge(&p, s))
                    .child(text(&p, theme::CAPTION, n.to_string()).text_color(p.fg_2))
            }),
        );
        let compat = v_flex()
            .gap(px(space::S12))
            .child(muted(&p, format!("{} models, list dated {} ({}).", self.compat.len(), self.compat.updated(), origin)))
            .child(counts)
            .child(h_flex().gap(px(space::S8)).items_center().child(btn_secondary("refresh-compat", "Refresh list").icon(icon("refresh-cw")).loading(self.compat_refreshing).disabled(self.compat_refreshing).on_click(cx.listener(|this, _, _, cx| this.refresh_compat(cx)))).when_some(self.compat_note.clone(), |d, (m, ok)| d.child(text(&p, theme::BODY_SM, m).text_color(if ok { p.fg_2 } else { p.warn_fg }).flex_1().min_w_0())))
            .child(muted(&p, "Verified means maintainers reproduced a full install on that model; it is still not a guarantee. Statuses are always shown as words with an icon.").text_size(theme::CAPTION.size()));

        // ---- about
        let logs = self.cfg_dir.join("logs");
        let logs_shown = logs.display().to_string();
        let about = v_flex()
            .gap(px(space::S8))
            .child(h_flex().gap(px(space::S8)).items_center().child(text(&p, theme::H2, "synofurb")).child(chip(&p, format!("v{}", version()))))
            .child(muted(&p, "Reinstalls Synology NAS units with plain Debian 13. One program for Windows, macOS and Linux; every decision lives in the shared engine, so the command line does exactly what this window does."))
            .child(h_flex().gap(px(space::S8)).items_center().child(muted(&p, "Logs and keys:")).child(mono(&p, logs_shown, cx).flex_1().min_w_0().truncate()).child(btn_secondary("show-logs", "Show folder").small().icon(icon("folder-open")).on_click(move |_, _, cx| {
                let _ = std::fs::create_dir_all(&logs);
                cx.reveal_path(&logs)
            })))
            .child(h_flex().gap(px(space::S8)).child(btn_ghost("repo", "Project page").small().icon(icon("external-link")).on_click(|_, _, cx| cx.open_url("https://github.com/andreaswiren/synofurb"))));

        let keys = v_flex().gap(px(space::S4)).children(
            [
                ("Tab / Shift+Tab", "Move between controls"),
                ("Enter", "The screen's main action (never the red erase button)"),
                ("Alt+Left / Alt+Right", "Previous / next step through completed steps"),
                ("Ctrl+, (Cmd+, on macOS)", "Settings and About"),
                ("Ctrl+F / Ctrl+A / Ctrl+C in the log", "Search, select all, copy"),
                ("Esc", "Close a dialog; never cancels an installation"),
            ]
            .map(|(k, d)| h_flex().gap(px(space::S12)).child(mono(&p, k, cx).w(px(250.0)).flex_none()).child(muted(&p, d))),
        );

        let body = v_flex()
            .w_full()
            .gap(px(space::S16))
            .child(card(&p, Some("Appearance".into()), None, None, appearance))
            .child(card(&p, Some("Updates".into()), None, None, updates))
            .child(card(&p, Some("Compatibility list".into()), None, None, compat))
            .child(card(&p, Some("Keyboard".into()), None, None, keys))
            .child(card(&p, Some("About".into()), None, None, about));
        let back = self.return_to;
        Page::new("Settings and About", body).subtitle("Preferences are stored on this computer only.").buttons(vec![
            btn_primary("close-settings", "Done")
                .on_click(cx.listener(move |this, _, window, cx| this.go(back, window, cx)))
                .into_any_element(),
        ])
    }
}
