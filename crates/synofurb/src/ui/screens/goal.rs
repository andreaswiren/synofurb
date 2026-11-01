//! D4 Goal: three GroupBox cards (Boot manager, Install Debian, Restore / Recover), one primary action.

use crate::ui::app::{App, Goal};
use crate::ui::copy::{self, Item};
use crate::ui::disclosure::info_tip;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::{BootChoice, Screen};
use crate::ui::theme::{Themed, Tone};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::{ActiveTheme, Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, IntoElement, Window, px};
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
        BootChoice::Dom.available(f).map_err(|_| copy::BM_NOT_EDITABLE.tip)
    }

    #[allow(clippy::too_many_arguments)]
    fn goal_card(
        &self,
        cx: &mut Context<App>,
        app: &Entity<App>,
        id: &'static str,
        icon: IconName,
        item: &'static Item,
        line: &'static Item,
        tag: Option<gpui_kit::component::tag::Tag>,
        button: Button,
    ) -> impl IntoElement {
        GroupBox::new()
            .outline()
            .id(id)
            .title(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .text_color(cx.theme().foreground)
                    .child(icons::icon_sized(icon, 18.0, cx.theme().foreground))
                    .child(kit::heading(item.short))
                    .child(info_tip(gpui_kit::SharedString::from(format!("goal-info-{id}")), item, app))
                    .children(tag),
            )
            .child(v_flex().gap(px(12.0)).child(kit::muted(cx, line.short)).child(h_flex().child(button)))
    }

    pub(crate) fn page_goal(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let status = self.facts.as_ref().map(|f| self.compat_for_facts(f).status).unwrap_or(Status::Untested);
        let install_ok = self.can_install();
        let bm = self.can_boot_manager();
        let try_first = bm.is_ok() && status == Status::Untested;

        let bm_btn = {
            let b = Button::new("goal-bm").label(copy::GOAL_SET_UP.short).disabled(bm.is_err()).tooltip(bm.err().unwrap_or(""));
            let b = if try_first { b.primary() } else { b.outline() };
            b.on_click(cx.listener(|this, _, window, cx| {
                this.goal = Goal::BootManager;
                this.continue_from_goal(window, cx);
            }))
        };
        let rec_tag = try_first.then(|| {
            // the Recommended tag explains itself in a tooltip through its info icon on the card title
            kit::tag(&p, Tone::Info, Some(role::SPARKLES), copy::GOAL_RECOMMENDED.short)
        });
        let inst_btn = {
            let b = Button::new("goal-install").label(copy::GOAL_PLAN_INSTALL.short).disabled(!install_ok).tooltip(if install_ok {
                ""
            } else {
                copy::GOAL_PLAN_INSTALL.tip
            });
            let b = if try_first { b.outline() } else { b.primary() };
            b.on_click(cx.listener(|this, _, window, cx| {
                this.goal = Goal::Install;
                this.continue_from_goal(window, cx);
            }))
        };
        let rec_btn =
            Button::new("goal-recover").outline().label(copy::GOAL_OPEN_RECOVERY.short).on_click(cx.listener(|this, _, window, cx| {
                this.goal = Goal::Recover;
                this.continue_from_goal(window, cx);
            }));

        let body = h_flex()
            .w_full()
            .items_start()
            .gap(px(16.0))
            .child(div_flex(self.goal_card(cx, &app, "bm", role::TERMINAL, &copy::GOAL_BOOTMGR, &copy::GOAL_BOOTMGR_LINE, rec_tag, bm_btn)))
            .child(div_flex(self.goal_card(cx, &app, "inst", role::DISK, &copy::GOAL_INSTALL, &copy::GOAL_INSTALL_LINE, None, inst_btn)))
            .child(div_flex(self.goal_card(cx, &app, "rec", role::HELP, &copy::GOAL_RECOVER, &copy::GOAL_RECOVER_LINE, None, rec_btn)));
        Page::new(copy::GOAL_TITLE.short, body).subtitle(copy::GOAL_SUB.short).left(vec![
            Button::new("back")
                .ghost()
                .icon(icons::icon(role::BACK))
                .label(copy::BTN_BACK.short)
                .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Inspect, window, cx)))
                .into_any_element(),
        ])
    }
}

fn div_flex(e: impl IntoElement) -> gpui_kit::Div {
    use gpui_kit::div;
    div().flex_1().min_w_0().child(e)
}
