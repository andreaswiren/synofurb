//! Step 4: choose a goal. Each card says plainly what it does to the disks.

use crate::ui::app::{App, Goal};
use crate::ui::render::Page;
use crate::ui::state::{BootChoice, Screen};
use crate::ui::theme::{self, Themed, Tone, radius, space};
use crate::ui::widgets::*;
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, px};
use synofurb_core::compat::Status;

impl App {
    pub(crate) fn continue_from_goal(&mut self, window: &mut Window, cx: &mut Context<App>) {
        match self.goal {
            Goal::Install if self.can_install() => self.go(Screen::Plan, window, cx),
            Goal::BootManager if self.can_boot_manager().is_ok() => self.go(Screen::BootMgr, window, cx),
            Goal::Recover => self.go(Screen::Recover, window, cx),
            _ => {}
        }
    }

    pub(crate) fn can_install(&self) -> bool {
        self.facts.as_ref().is_some_and(|f| !self.compat_for_facts(f).blocking)
    }

    pub(crate) fn can_boot_manager(&self) -> Result<(), &'static str> {
        let Some(f) = &self.facts else { return Err("No unit has been inspected.") };
        if self.compat_for_facts(f).blocking {
            return Err("This hardware is not supported.");
        }
        BootChoice::Dom.available(f).map_err(|_| "Needs an editable internal boot flash (DOM), which this unit does not have.")
    }

    pub(crate) fn page_goal(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let name = self.unit_name();
        let status = self.facts.as_ref().map(|f| self.compat_for_facts(f).status).unwrap_or(Status::Untested);
        let install_ok = self.can_install();
        let bm = self.can_boot_manager();
        let promote_bm = bm.is_ok() && matches!(status, Status::Untested | Status::KnownIssues | Status::Community);

        let tag = |tone: Tone, ic: &str, label: &str| badge(&p, tone, ic, label.to_string(), false);

        let install_card = v_flex().w_full().gap(px(space::S6)).child(
            choice_card(
                &p,
                "goal-install",
                self.goal == Goal::Install,
                !install_ok,
                format!("Install Debian on {name}"),
                "Replaces DSM with Debian 13 on LVM, with a mirrored system area and your choice of data RAID. You plan the layout next and confirm before anything is written.",
                cx.listener(|this, _, _, cx| {
                    this.goal = Goal::Install;
                    cx.notify();
                }),
            ),
        );
        let install_tags = h_flex()
            .gap(px(space::S8))
            .pl(px(space::S12))
            .child(tag(Tone::Err, "triangle-alert", "Erases the disks"))
            .when(!install_ok, |d| d.child(tag(Tone::Err, "ban", "Disabled: unsupported hardware")));

        let bm_reason = bm.err();
        let bm_card = choice_card(
            &p,
            "goal-bootmgr",
            self.goal == Goal::BootManager,
            bm_reason.is_some(),
            format!("Boot manager mode{}", if promote_bm { " (try this first)" } else { "" }),
            "Adds a small SSH-controllable boot environment to the unit's internal boot flash so you can netboot or kexec any OS later. DSM stays the fail-safe default, and your disks are not touched.",
            cx.listener(|this, _, _, cx| {
                this.goal = Goal::BootManager;
                cx.notify();
            }),
        );
        let bm_tags = h_flex()
            .gap(px(space::S8))
            .pl(px(space::S12))
            .child(tag(Tone::Ok, "shield-check", "Disks untouched, reversible"))
            .when_some(bm_reason, |d, r| d.child(muted(&p, r).text_size(theme::CAPTION.size())));

        let rec_card = choice_card(
            &p,
            "goal-recover",
            self.goal == Goal::Recover,
            false,
            "Restore and recover",
            "Make the unit boot its original DSM again (from DSM, from the installed Debian or from the boot agent), or find out what is still possible after a failed install.",
            cx.listener(|this, _, _, cx| {
                this.goal = Goal::Recover;
                cx.notify();
            }),
        );
        let rec_tags = h_flex().gap(px(space::S8)).pl(px(space::S12)).child(tag(Tone::Info, "life-buoy", "Always available"));

        let body = v_flex()
            .w_full()
            .gap(px(space::S12))
            .child(install_card)
            .child(install_tags)
            .child(bm_card)
            .child(bm_tags)
            .child(rec_card)
            .child(rec_tags)
            .child(
                h_flex()
                    .p(px(space::S12))
                    .gap(px(space::S8))
                    .rounded(px(radius::MD))
                    .bg(p.surface_2)
                    .child(icon_sized("info", 16.0, p.fg_2).mt(px(2.0)))
                    .child(
                        muted(
                            &p,
                            "Compatibility for this unit: Verified means maintainers reproduced a full install; it is still not a guarantee. For Untested and Known-issues units the boot manager is the safe way to try first.",
                        )
                        .flex_1()
                        .min_w_0(),
                    ),
            );

        let enabled = match self.goal {
            Goal::Install => install_ok,
            Goal::BootManager => self.can_boot_manager().is_ok(),
            Goal::Recover => true,
        };
        Page::new("What do you want to do?", body)
            .subtitle(format!("{name} is inspected and nothing has been changed. Pick a goal; each one says what it does to the disks."))
            .note(muted(
                &p,
                if enabled { "Nothing is written until the confirmation step." } else { "This goal is not available for this unit." },
            ))
            .buttons(vec![
                btn_secondary("back", "Back")
                    .icon(icon("arrow-left"))
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Inspect, window, cx)))
                    .into_any_element(),
                btn_primary("continue", "Continue")
                    .icon(icon("arrow-right"))
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, window, cx| this.continue_from_goal(window, cx)))
                    .into_any_element(),
            ])
    }
}
