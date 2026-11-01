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
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, IntoElement, Role, Window, div, px};

const PHASES: [&str; PHASE_COUNT] = ["Connect", "Re-check", "Stage", "Boot installer", "Install", "Reboot", "Verify"];
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
                    .py(px(3.0))
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
        let bar = v.bar_fraction() * 100.0;
        let upload = v.progress.clone().filter(|(_, f)| *f < 1.0);
        let finished = v.installer_done;
        let dom = self.facts.as_ref().is_some_and(|f| self.sys.boot.resolve(f) == BootChoice::Dom);

        let phases = Popover::new("phases")
            .trigger(
                Button::new("phase-tag")
                    .ghost()
                    .small()
                    .icon(icons::icon(role::INFO))
                    .label(PHASES[phase_ix - 1])
                    .accessibility_label(copy::INSTALL_PHASES.short),
            )
            .content({
                let cur = phase_ix;
                move |_, _, cx| {
                    v_flex().w(px(240.0)).gap(px(4.0)).text_sm().child(kit::strong(copy::INSTALL_PHASES.tip)).children(
                        PHASES.iter().enumerate().map(move |(i, n)| {
                            h_flex()
                                .gap(px(6.0))
                                .items_center()
                                .child(icons::icon_sized(
                                    if i + 1 < cur {
                                        role::OK
                                    } else if i + 1 == cur {
                                        role::CURRENT
                                    } else {
                                        role::PENDING
                                    },
                                    14.0,
                                    cx.theme().foreground,
                                ))
                                .child(format!("{} {}", i + 1, n))
                        }),
                    )
                }
            });
        let progress = v_flex()
            .gap(px(6.0))
            .child(
                h_flex()
                    .gap(px(12.0))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Progress::new("install-bar").value(bar).accessibility_label("Installation progress")),
                    )
                    .child(kit::mono_sm(cx, percent(v.bar_fraction())))
                    .child(kit::mono_sm(cx, format!("Phase {phase_ix} of {PHASE_COUNT}")))
                    .child(phases),
            )
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
                .py(px(3.0))
                .child(icons::icon_sized(if done { role::OK } else { role::PENDING }, 16.0, if done { p.ok_fg } else { p.placeholder }))
                .child(div().text_sm().child(*n))
        }));
        let steps = self.step_rows(cx, false);
        let flash_open = dom && !finished && phase_ix <= 4;
        let acc = Accordion::new("install-acc")
            .multiple(true)
            .bordered(true)
            .when(dom, |a| a.item(|i| i.title(copy::INSTALL_FLASH.short).open(flash_open).child(flash_rows)))
            .item(|i| i.title(copy::INSTALL_STEPS.short).open(!flash_open).child(steps))
            .item(|i| i.title(copy::INSTALL_LOG.short).child(log));

        let mut body = v_flex().w_full().gap(px(14.0));
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
            .right(vec![
                Button::new("cancel-install")
                    .outline()
                    .label(copy::INSTALL_CANCEL.short)
                    .disabled(!can_cancel)
                    .tooltip(if can_cancel { "" } else { copy::INSTALL_CANCEL.tip })
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_cancel(window, cx)))
                    .into_any_element(),
            ])
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

#[allow(dead_code)]
fn _unused(_: DialogButtonProps) {}
