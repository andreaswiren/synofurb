//! D8 Install: progress, the point of no return, cancel rules. Phases behind a Popover, three Accordion items
//! (boot flash, steps, log), the log collapsed by default.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure::info_tip;
use crate::ui::format::percent;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::*;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FontWeight, IntoElement, Role, Window, div, px};

const FLASH_STEPS: [&str; 4] = ["Back up flash", "Upload agent", "Reboot into agent", "Send installer"];

impl App {
    /// The 12 engine steps as rows with a status icon; step 3 carries the "No return" tag.
    pub(crate) fn step_rows(&self, cx: &mut Context<App>, failed: bool) -> AnyElement {
        let p = cx.pal();
        let v = &self.install;
        let app = cx.entity();
        v_flex()
            .gap(px(2.0))
            .children((1..=12u32).map(|n| {
                let st = v.step_progress(n, failed);
                let name = INSTALL_STEPS[n as usize - 1];
                let icon: AnyElement = match st {
                    StepProgress::Done => icons::icon_sized(role::OK, 16.0, p.ok_fg).into_any_element(),
                    StepProgress::Failed => icons::icon_sized(role::ERR, 16.0, p.err_fg).into_any_element(),
                    StepProgress::Running if self.settings.reduce_motion => {
                        icons::icon_sized(role::RUNNING, 16.0, p.accent_fg).into_any_element()
                    }
                    StepProgress::Running => Spinner::new().small().color(p.accent).into_any_element(),
                    StepProgress::Pending => icons::icon_sized(role::PENDING, 16.0, p.placeholder).into_any_element(),
                };
                h_flex()
                    .id(("step", n as usize))
                    .role(Role::ListItem)
                    .aria_label(format!("Step {n} of 12: {name}"))
                    .gap(px(8.0))
                    .items_center()
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(cx.theme().radius)
                    .when(st == StepProgress::Running, |d| d.bg(cx.theme().tokens.accent))
                    .child(icon)
                    .child(
                        div()
                            .text_sm()
                            .flex_1()
                            .min_w_0()
                            .when(st == StepProgress::Pending, |d| d.text_color(cx.theme().muted_foreground))
                            .child(format!("{n}. {name}")),
                    )
                    .when(n == PONR_STEP, |d| {
                        d.child(
                            h_flex()
                                .gap(px(2.0))
                                .child(kit::tag(&p, Tone::Err, Some(role::STOP), copy::INSTALL_NO_RETURN.short))
                                .child(info_tip("ponr-info", &copy::INSTALL_NO_RETURN, &app)),
                        )
                    })
            }))
            .into_any_element()
    }

    pub(crate) fn page_install(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let log = self.render_log_viewer(Some(240.0), cx);
        let v = &self.install;
        let phase_ix = v.phase_index().max(1);
        let app_e = cx.entity();
        let upload = v.progress.clone().filter(|(_, f)| *f < 1.0);
        let finished = v.installer_done;
        let dom = self.facts.as_ref().is_some_and(|f| self.sys.boot.resolve(f) == BootChoice::Dom);

        // One numbering scheme: the engine's 12 steps. Before step 1 starts the line names the phase instead.
        let now_line = if v.step_total > 0 {
            format!("Step {} of {} \u{00b7} {}", v.step_n, v.step_total, v.step_name)
        } else {
            v.phase_label().to_string()
        };
        let seg_state = |n: u32| v.step_progress(n, false);
        let strip = v_flex()
            .gap(px(4.0))
            .child(
                // the point-of-no-return flag sits over segment 3
                h_flex().w_full().gap(px(4.0)).children((1..=12u32).map(|n| {
                    div().flex_1().min_w_0().h(px(26.0)).when(n == PONR_STEP, |d| {
                        d.child(
                            h_flex()
                                .gap(px(2.0))
                                .child(kit::tag(&p, Tone::Err, Some(role::STOP), copy::INSTALL_NO_RETURN.short))
                                .child(info_tip("ponr-info", &copy::INSTALL_NO_RETURN, &app_e)),
                        )
                    })
                })),
            )
            .child(h_flex().w_full().gap(px(4.0)).children((1..=12u32).map(|n| {
                let c = match seg_state(n) {
                    StepProgress::Done => p.ok_solid,
                    StepProgress::Running => p.accent,
                    StepProgress::Failed => p.err_solid,
                    StepProgress::Pending => p.border_strong.opacity(0.35),
                };
                div().id(("seg", n as usize)).flex_1().min_w_0().h(px(8.0)).rounded(px(2.0)).bg(c)
            })));
        let progress = v_flex()
            .gap(px(8.0))
            .child(
                h_flex()
                    .gap(px(12.0))
                    .items_baseline()
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(crate::ui::theme::TITLE.size())
                            .font_weight(FontWeight::BOLD)
                            .child(percent(v.bar_fraction())),
                    )
                    .child(kit::muted(cx, now_line)),
            )
            .child(strip)
            .when_some(upload, |d, (what, f)| {
                d.child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::caption(cx, what))
                        .child(div().flex_1().child(Progress::new("upload-bar").value(f * 100.0)))
                        .child(kit::mono_sm(cx, percent(f))),
                )
            });

        let flash_rows = v_flex().gap(px(2.0)).children(FLASH_STEPS.iter().enumerate().map(|(i, n)| {
            let done = phase_ix > i + 1 || finished;
            h_flex()
                .gap(px(8.0))
                .items_center()
                .px(px(8.0))
                .py(px(4.0))
                .child(icons::icon_sized(if done { role::OK } else { role::PENDING }, 16.0, if done { p.ok_fg } else { p.placeholder }))
                .child(div().text_sm().child(*n))
        }));
        let steps = self.step_rows(cx, false);
        let flash_open = dom && !finished && phase_ix <= 4;
        let acc = Accordion::new("install-acc")
            .multiple(true)
            .bordered(true)
            .when(dom, |a| a.item(|i| i.title(copy::INSTALL_FLASH.short).open(flash_open).child(flash_rows)))
            .item(|i| i.title(copy::INSTALL_STEPS.short).child(steps))
            .item(|i| i.title(copy::INSTALL_LOG.short).child(log));

        let mut body = v_flex().w_full().gap(px(12.0));
        if v.past_no_return {
            body = body.child(
                Alert::error("ponr", format!("{} \u{00b7} {}", copy::INSTALL_NO_RETURN.short, copy::INSTALL_SUB.short))
                    .icon(icons::icon(role::DANGER)),
            );
        }
        body = body.child(progress).child(acc);

        let can_cancel = self.install.can_cancel() && self.result.is_none();
        let log_dir = self.cfg_dir.join("logs");
        let log_path = self.logger_path.clone();
        Page::new(copy::INSTALL_TITLE.short, body)
            .subtitle(copy::INSTALL_SUB.short)
            .left(vec![
                Button::new("open-log")
                    .ghost()
                    .icon(icons::icon(role::FILE))
                    .tooltip(copy::INSTALL_OPEN_LOG.tip)
                    .accessibility_label(copy::INSTALL_OPEN_LOG.short)
                    .on_click(move |_, _, cx| {
                        let target = log_path.clone().unwrap_or_else(|| log_dir.clone());
                        cx.reveal_path(&target)
                    })
                    .into_any_element(),
            ])
            .right(vec![kit::with_reason(
                "cancel-why",
                Button::new("cancel-install")
                    .outline()
                    .label(copy::INSTALL_CANCEL.short)
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_cancel(window, cx))),
                (!can_cancel).then(|| copy::INSTALL_CANCEL.tip.to_string()),
            )])
    }

    /// "Cancel install?" (AlertDialog): Esc and the scrim never stop an install; only "Stop install" does.
    pub(crate) fn confirm_cancel(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let app = app.clone();
            alert
                .icon(icons::icon_sized(role::WARN, 20.0, gpui_kit::component::ActiveTheme::theme(cx).warning))
                .title(copy::INSTALL_CANCEL_DIALOG.short)
                .description(copy::INSTALL_CANCEL_DIALOG.body)
                .show_cancel(true)
                .ok_text(copy::INSTALL_STOP.short)
                .cancel_text(copy::INSTALL_KEEP_GOING.short)
                .on_ok(move |_, _, cx| {
                    app.update(cx, |a, cx| a.cancel_install(cx));
                    true
                })
        });
    }
}
