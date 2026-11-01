//! D9 Install stopped: what happened, whether disks and flash changed, the clock, the next action.
//! Text follows docs/RECOVERY.md; no promise of data recovery, ever.

use crate::ui::app::{App, Job};
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::{BootChoice, Screen};
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::Sizable;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, px};

const RESCUE_TIP: &str = "Not available in this version; use the NAS console (VGA or serial).";

impl App {
    pub(crate) fn page_failed(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let touched = self.install.past_no_return;
        let err = match &self.result {
            Some(Err(e)) => e.clone(),
            _ => self.install.installer_failed.clone().unwrap_or_else(|| "The installer stopped.".into()),
        };
        let uf_flash = self.unit_facts().flash;
        let dom = self.facts.as_ref().is_some_and(|f| self.sys.boot.resolve(f) == BootChoice::Dom);
        let subtitle = if self.install.step_total > 0 {
            format!("Step {} of {} \u{00b7} {}", self.install.step_n, self.install.step_total, self.install.step_name)
        } else {
            if self.agent_unknown { "Waiting for the boot agent".to_string() } else { "before the installer started".to_string() }
        };
        let (disks_item, disks_tone, disks_icon) = if touched {
            (&copy::FAIL_DISKS_CHANGED, Tone::Err, role::DANGER)
        } else {
            (&copy::FAIL_DISKS_UNTOUCHED, Tone::Ok, role::SAFE)
        };
        let (flash_item, flash_tone) =
            if uf_flash { (&copy::FAIL_FLASH_CHANGED, Tone::Warn) } else { (&copy::FAIL_FLASH_UNTOUCHED, Tone::Ok) };
        let tags = h_flex()
            .gap(px(8.0))
            .items_center()
            .flex_wrap()
            .child(
                h_flex()
                    .gap(px(2.0))
                    .child(kit::tag(&p, disks_tone, Some(disks_icon), disks_item.short))
                    .child(crate::ui::disclosure::info_tip("t-disks", disks_item, &app)),
            )
            .child(
                h_flex()
                    .gap(px(2.0))
                    .child(kit::tag(&p, flash_tone, Some(role::PACKAGE), flash_item.short))
                    .child(crate::ui::disclosure::info_tip("t-flash", flash_item, &app)),
            )
            .children((!self.agent_unknown).then(|| {
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(kit::muted(cx, if touched { copy::FAIL_RESCUE_IN.short } else { copy::FAIL_DSM_IN.short }))
                    .child(kit::mono(cx, if touched { "15 min" } else { "60 s" }))
                    .child(crate::ui::disclosure::info_tip(
                        "t-clock",
                        if touched { &copy::FAIL_RESCUE_IN } else { &copy::FAIL_DSM_IN },
                        &app,
                    ))
            }));

        let status: gpui_kit::AnyElement = match &self.recover {
            Job::Done(Ok(s)) => Alert::success("rec-ok", s.trim().to_string()).small().icon(icons::icon(role::OK)).into_any_element(),
            Job::Done(Err(e)) => Alert::error("rec-err", e.clone()).small().icon(icons::icon(role::ERR)).into_any_element(),
            _ => gpui_kit::div().into_any_element(),
        };
        let tail: Vec<String> = self.install.log.tail_info(14).into_iter().map(crate::ui::logview::format_line).collect();
        let log = v_flex()
            .gap(px(2.0))
            .p(px(8.0))
            .rounded(cx.theme_radius())
            .bg(p.log_bg)
            .children(tail.into_iter().map(|l| kit::mono_sm(cx, l).text_color(p.log_fg)));
        let acc = Accordion::new("fail-acc").bordered(true).item(|i| i.title(copy::FAIL_LOG.short).child(log));

        let body = v_flex()
            .w_full()
            .gap(px(14.0))
            .child(tags)
            .child(Alert::error("fail-banner", err).icon(icons::icon(role::ERR)))
            .child(
                h_flex().gap(px(8.0)).child(
                    Button::new("next")
                        .link()
                        .label(copy::FAIL_NEXT.short)
                        .tooltip(copy::FAIL_NEXT.tip)
                        .on_click(cx.listener(|this, _, window, cx| this.open_next_dialog(window, cx))),
                ),
            )
            .child(status)
            .child(acc);

        let primary: gpui_kit::AnyElement = if self.agent_unknown {
            Button::new("manual")
                .primary()
                .icon(icons::icon(role::HELP))
                .label(copy::REC_MANUAL.short)
                .on_click(cx.listener(|this, _, window, cx| this.open_manual_recovery(window, cx)))
                .into_any_element()
        } else if touched {
            kit::with_reason(
                "rescue-why",
                Button::new("rescue").primary().icon(icons::icon(role::TERMINAL)).label(copy::FAIL_OPEN_RESCUE.short),
                Some(RESCUE_TIP.to_string()),
            )
        } else {
            kit::with_reason(
                "wait-why",
                Button::new("wait-dsm")
                    .primary()
                    .label(copy::FAIL_WAIT_DSM.short)
                    .loading(self.recover.running())
                    .on_click(cx.listener(|this, _, _, cx| this.start_recover(cx))),
                (!dom).then(|| "Nothing was written; DSM comes back when the NAS restarts.".to_string()),
            )
        };
        let diag = Button::new("export")
            .outline()
            .label(copy::FAIL_EXPORT.short)
            .tooltip(copy::FAIL_EXPORT.tip)
            .on_click(cx.listener(|this, _, window, cx| this.copy_diagnostics(window, cx)));
        let recovery = Button::new("rec-opts")
            .outline()
            .label(copy::FAIL_RECOVERY.short)
            .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Recover, window, cx)));
        Page::new(copy::FAIL_TITLE.short, body)
            .subtitle(subtitle)
            .left(vec![
                Button::new("start-over")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label("Start over")
                    .on_click(cx.listener(|this, _, window, cx| this.start_over(window, cx)))
                    .into_any_element(),
            ])
            .right(vec![diag.into_any_element(), recovery.into_any_element(), primary])
    }
}

trait ThemeRadius {
    fn theme_radius(&self) -> gpui_kit::Pixels;
}
impl<T> ThemeRadius for Context<'_, T> {
    fn theme_radius(&self) -> gpui_kit::Pixels {
        use gpui_kit::component::ActiveTheme;
        self.theme().radius
    }
}
