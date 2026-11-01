//! Step 8: the install. Phase strip, 12-step list with the point of no return as a named object, upload progress,
//! U-Boot "action needed" box for ARM boards, and the filterable live log.

use crate::ui::app::App;
use crate::ui::format::percent;
use crate::ui::render::Page;
use crate::ui::state::*;
use crate::ui::theme::{self, Palette, Themed, Tone, radius, space};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FontWeight, IntoElement, Role, Window, div, px};

const PHASES: [&str; PHASE_COUNT] = ["Connect", "Re-check", "Stage", "Boot installer", "Install", "Reboot", "Verify"];

impl App {
    pub(crate) fn phase_strip(&self, p: &Palette) -> impl IntoElement {
        let cur = self.install.phase_index();
        h_flex().w_full().gap(px(space::S4)).children(PHASES.iter().enumerate().map(|(i, name)| {
            let n = i + 1;
            let done = n < cur || self.install.installer_done && n <= cur;
            let now = n == cur;
            let c = if now {
                p.accent
            } else if done {
                p.ok_solid
            } else {
                p.border_strong
            };
            v_flex().flex_1().gap(px(space::S4)).child(div().h(px(4.0)).rounded_full().bg(if done || now { c } else { p.surface_3 })).child(
                h_flex()
                    .gap(px(space::S4))
                    .items_center()
                    .child(icon_sized(
                        if done {
                            "check"
                        } else if now {
                            "circle-dot"
                        } else {
                            "circle-dashed"
                        },
                        12.0,
                        if now {
                            p.accent_fg
                        } else if done {
                            p.ok_fg
                        } else {
                            p.placeholder
                        },
                    ))
                    .child(
                        text(p, theme::CAPTION, *name)
                            .text_color(if now {
                                p.accent_fg
                            } else if done {
                                p.fg
                            } else {
                                p.placeholder
                            })
                            .font_weight(FontWeight(if now { 700.0 } else { 400.0 })),
                    ),
            )
        }))
    }

    /// The 12-step list with phase captions and the point-of-no-return rule.
    pub(crate) fn step_list(&self, p: &Palette, failed: bool) -> AnyElement {
        let v = &self.install;
        let mut col = v_flex().w_full().gap(px(space::S2));
        for (caption_text, range) in STEP_GROUPS.iter() {
            col = col.child(caption(p, caption_text.to_string()).mt(px(space::S8)));
            for n in range.clone() {
                if n == PONR_STEP {
                    let passed = v.past_no_return;
                    col = col.child(
                        h_flex()
                            .w_full()
                            .gap(px(space::S8))
                            .items_center()
                            .px(px(space::S8))
                            .py(px(space::S4))
                            .my(px(space::S4))
                            .rounded(px(radius::SM))
                            .border_1()
                            .border_dashed()
                            .border_color(p.err_solid)
                            .bg(if passed { p.err_bg } else { p.surface })
                            .child(icon_sized("flag", 14.0, p.err_fg))
                            .child(
                                text(p, theme::CAPTION, if passed { "Point of no return: passed" } else { "Point of no return: ahead" })
                                    .text_color(p.err_fg)
                                    .font_weight(FontWeight(700.0)),
                            ),
                    );
                }
                let st = v.step_progress(n, failed);
                let name = INSTALL_STEPS[n as usize - 1];
                let glyph: AnyElement = match st {
                    StepProgress::Done => div()
                        .size(px(20.0))
                        .rounded_full()
                        .bg(p.ok_solid)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon_sized("check", 13.0, p.on_accent))
                        .into_any_element(),
                    StepProgress::Failed => div()
                        .size(px(20.0))
                        .rounded_full()
                        .bg(p.err_solid)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon_sized("x", 13.0, p.on_accent))
                        .into_any_element(),
                    StepProgress::Running => {
                        if self.settings.reduce_motion {
                            div()
                                .size(px(20.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon_sized("loader-circle", 16.0, p.accent_fg))
                                .into_any_element()
                        } else {
                            div()
                                .size(px(20.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Spinner::new().small().color(p.accent))
                                .into_any_element()
                        }
                    }
                    StepProgress::Pending => div()
                        .size(px(20.0))
                        .rounded_full()
                        .border_1()
                        .border_color(p.border_strong)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(text(p, theme::CAPTION, n.to_string()).text_color(p.placeholder))
                        .into_any_element(),
                };
                let word = match st {
                    StepProgress::Running => Some(if self.settings.reduce_motion { "running" } else { "" }),
                    StepProgress::Failed => Some("failed"),
                    _ => None,
                };
                col = col.child(
                    h_flex()
                        .id(("step", n as usize))
                        .role(Role::ListItem)
                        .aria_label(format!(
                            "Step {n} of 12: {name}, {}",
                            match st {
                                StepProgress::Done => "done",
                                StepProgress::Running => "running",
                                StepProgress::Pending => "pending",
                                StepProgress::Failed => "failed",
                            }
                        ))
                        .gap(px(space::S8))
                        .items_center()
                        .px(px(space::S8))
                        .py(px(space::S4))
                        .rounded(px(radius::SM))
                        .when(st == StepProgress::Running, |d| d.bg(p.accent_bg))
                        .when(st == StepProgress::Failed, |d| d.bg(p.err_bg))
                        .child(glyph)
                        .child(
                            text(p, theme::BODY_SM, name)
                                .flex_1()
                                .min_w_0()
                                .text_color(match st {
                                    StepProgress::Pending => p.placeholder,
                                    StepProgress::Failed => p.err_fg,
                                    _ => p.fg,
                                })
                                .font_weight(FontWeight(if matches!(st, StepProgress::Running | StepProgress::Failed) {
                                    650.0
                                } else {
                                    400.0
                                })),
                        )
                        .when_some(word.filter(|w| !w.is_empty()), |d, w| {
                            d.child(text(p, theme::CAPTION, w).text_color(if st == StepProgress::Failed { p.err_fg } else { p.accent_fg }))
                        }),
                );
            }
        }
        col.into_any_element()
    }

    /// "ACTION NEEDED" box with the U-Boot commands the installer printed (ARM boards).
    pub(crate) fn uboot_box(&self, p: &Palette, cx: &mut Context<App>) -> Option<AnyElement> {
        if self.install.uboot_lines.is_empty() {
            return None;
        }
        let lines = self.install.uboot_lines.clone();
        let copy = lines.join("\n");
        Some(
            v_flex()
                .w_full()
                .gap(px(space::S8))
                .p(px(space::S12))
                .rounded(px(radius::MD))
                .border_2()
                .border_color(p.warn_solid)
                .bg(p.warn_bg)
                .id("uboot-box")
                .role(Role::Alert)
                .child(
                    h_flex()
                        .gap(px(space::S8))
                        .items_center()
                        .child(icon_sized("triangle-alert", 18.0, p.warn_fg))
                        .child(text(p, theme::BODY, "ACTION NEEDED on the NAS serial console (U-Boot)").text_color(p.warn_fg).font_weight(FontWeight(700.0))),
                )
                .child(text(p, theme::BODY_SM, "Debian is installed, but this board's U-Boot cannot find it by itself and DSM's system partition is gone. A serial adapter (3.3 V TTL, 115200 baud) is required. When the NAS reboots, interrupt U-Boot with Ctrl+C and paste these commands:").text_color(p.warn_fg))
                .child(code_box(p, &lines, cx))
                .child(h_flex().child(btn_secondary("copy-uboot", "Copy commands").small().icon(icon("copy")).on_click(cx.listener(move |this, _, window, cx| this.copy_text(copy.clone(), "U-Boot commands", window, cx)))))
                .into_any_element(),
        )
    }

    pub(crate) fn page_install(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let v = &self.install;
        let name = self.unit_name();
        let step_text = if v.step_total > 0 {
            format!("Step {} of {}: {}", v.step_n, v.step_total, v.step_name)
        } else {
            "Waiting for the installer to start".to_string()
        };
        let bar = v.bar_fraction() * 100.0;
        let upload = v.progress.clone().filter(|(_, f)| *f < 1.0);

        let mut left = v_flex()
            .w(px(300.0))
            .flex_none()
            .gap(px(space::S8))
            .child(text(&p, theme::H2, v.phase_label()))
            .child(
                div()
                    .id("install-phase")
                    .role(Role::Status)
                    .aria_label(format!("{}. {}", v.phase_label(), step_text))
                    .child(muted(&p, step_text)),
            )
            .child(
                h_flex()
                    .gap(px(space::S8))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Progress::new("install-bar").value(bar).accessibility_label("Installation progress")),
                    )
                    .child(text(&p, theme::BODY_SM, percent(v.bar_fraction())).font_weight(FontWeight(600.0))),
            );
        if let Some((what, f)) = upload {
            left = left.child(
                v_flex().gap(px(space::S4)).child(muted(&p, what).text_size(theme::CAPTION.size())).child(
                    h_flex()
                        .gap(px(space::S8))
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Progress::new("upload-bar").value(f * 100.0).accessibility_label("Upload progress")),
                        )
                        .child(text(&p, theme::CAPTION, percent(f))),
                ),
            );
        }
        left = left.child(self.step_list(&p, false));

        let top_banner = if v.past_no_return {
            banner(
                &p,
                Tone::Err,
                "The installer has started changing the disks",
                format!("Do not power off {name}, and keep this window open and the computer awake. Cancel is no longer possible."),
                vec![],
            )
            .id("ponr-banner")
            .role(Role::Alert)
            .into_any_element()
        } else {
            banner(&p, Tone::Info, "Nothing has been changed yet", format!("Until the point of no return (step {PONR_STEP}) you can cancel for free. Keep this window open and the computer awake: the NAS fetches its installer from here and streams its log back."), vec![]).into_any_element()
        };

        let mut right = v_flex().flex_1().min_w_0().min_h_0().gap(px(space::S12)).child(self.phase_strip(&p));
        if let Some(b) = self.uboot_box(&p, cx) {
            right = right.child(b);
        }
        let log = self.render_log_viewer(None, cx);
        right = right.child(log);

        let body = v_flex().size_full().gap(px(space::S12)).child(top_banner).child(
            h_flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .gap(px(space::S16))
                .child(div().id("steps-scroll").h_full().overflow_y_scroll().child(left))
                .child(right),
        );

        let can_cancel = self.install.can_cancel() && self.result.is_none();
        let path = self.logger_path.clone();
        let note: AnyElement = if !can_cancel {
            h_flex()
                .gap(px(space::S8))
                .items_center()
                .child(icon_sized("lock", 14.0, p.fg_2))
                .child(muted(&p, "Cancel unavailable: the point of no return has passed and the disks are being changed."))
                .into_any_element()
        } else {
            muted(&p, "Cancel stops the installer at the next safe point. The NAS boots DSM again.").into_any_element()
        };
        let mut buttons = vec![];
        if let Some(path) = path.clone() {
            let shown = path.display().to_string();
            buttons.push(
                btn_secondary("copy-log-path", "Copy log path")
                    .small()
                    .icon(icon("copy"))
                    .on_click(cx.listener(move |this, _, window, cx| this.copy_text(shown.clone(), "Log path", window, cx)))
                    .into_any_element(),
            );
        }
        buttons.push(
            Button::new("cancel-install")
                .danger()
                .outline()
                .label("Cancel installation")
                .disabled(!can_cancel)
                .on_click(cx.listener(|this, _, _, cx| this.cancel_install(cx)))
                .into_any_element(),
        );
        Page::new("Installing Debian", body)
            .subtitle(format!("{name}. {}", self.install.phase_label()))
            .note(note)
            .buttons(buttons)
            .no_scroll()
    }
}
