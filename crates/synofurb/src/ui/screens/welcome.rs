//! D1b Welcome guide: a modal with five short steps (one icon, one headline, at most two lines, details behind an info
//! button), dots, Back / Next / Skip and, on the last step, "Don't show again" + Start. The rules live in
//! `ui::welcome` (pure, tested); this file only draws them.
//!
//! The view owns a focus handle so the arrow keys work while nothing else has focus; Tab walks the buttons and the
//! checkbox inside the dialog like in every other dialog.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::theme::{self, Themed, Tone};
use crate::ui::welcome::{Guide, Moved, STEP_COUNT};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render, Window, div, px};

/// Height of the text block of the tallest step (the six-step row), kept on all steps.
const BODY_MIN_H: f32 = 136.0;

pub struct WelcomeView {
    app: Entity<App>,
    guide: Guide,
    focus: FocusHandle,
}

impl WelcomeView {
    pub fn new(app: Entity<App>, step: usize, cx: &mut Context<WelcomeView>) -> WelcomeView {
        WelcomeView { app, guide: Guide::at(step), focus: cx.focus_handle() }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Does the way the guide ended say "seen"? (Esc, the close button, Skip, Start with the box ticked)
    pub fn should_remember(&self) -> bool {
        self.guide.remember_on_close()
    }

    pub fn app(&self) -> &Entity<App> {
        &self.app
    }

    /// Store "seen" in the settings (once).
    pub fn store_seen(app: &Entity<App>, cx: &mut gpui_kit::App) {
        app.update(cx, |a, _| {
            if !a.settings.welcome_done {
                a.settings.welcome_done = true;
                a.save_settings();
            }
        });
    }

    fn close(&mut self, moved: Moved, window: &mut Window, cx: &mut Context<WelcomeView>) {
        if moved == Moved::Closed {
            if self.should_remember() {
                Self::store_seen(&self.app, cx);
            }
            window.close_dialog(cx);
        } else {
            cx.notify();
        }
    }

    /// The six wizard steps as a row: the window's Stepper, not clickable here.
    fn flow(&self) -> gpui_kit::AnyElement {
        let labels = [&copy::STEP_FIND, &copy::STEP_CONNECT, &copy::STEP_GOAL, &copy::STEP_SETUP, &copy::STEP_CONFIRM, &copy::STEP_INSTALL];
        let items = labels.iter().map(|l| StepperItem::new().child(div().text_sm().child(l.short)));
        div().w_full().py(px(4.0)).child(Stepper::new("welcome-steps").disabled(true).selected_index(0).items(items)).into_any_element()
    }

    /// One dot per step; the current one is a longer pill in the accent colour, so position is not colour alone.
    fn dots(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let p = cx.pal();
        h_flex()
            .gap(px(6.0))
            .items_center()
            .children((0..STEP_COUNT).map(|i| {
                let now = i == self.guide.step();
                div().h(px(8.0)).w(px(if now { 20.0 } else { 8.0 })).rounded_full().bg(if now { p.accent } else { p.border_strong })
            }))
            .into_any_element()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.pal();
        let c = self.guide.content();
        let app = self.app.clone();

        let tone = if c.warn { p.tone(Tone::Warn) } else { p.tone(Tone::Accent) };
        let tile = div()
            .size(px(44.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.surface_2)
            .child(icons::icon_sized(c.icon, 24.0, tone.fg));

        let title = div()
            .text_size(theme::TITLE.size())
            .line_height(theme::TITLE.line())
            .font_weight(FontWeight(theme::TITLE.weight))
            .child(c.title.short);
        // the info button sits right behind the last line, not at the far edge
        let last = c.lines.len() - 1;
        let lines = v_flex().gap(px(4.0)).text_color(cx.theme().foreground).children(c.lines.iter().enumerate().map(|(i, l)| {
            let row = h_flex().gap(px(4.0)).items_center().child(div().child(l.short));
            match c.info {
                Some(info) if i == last => row.child(disclosure::info_tip(("welcome-info", self.guide.step()), info, &app)),
                _ => row,
            }
        }));

        // footer: Skip (or the checkbox) on the left, Back + the one primary on the right
        let left: gpui_kit::AnyElement = if self.guide.shows_checkbox() {
            h_flex()
                .gap(px(4.0))
                .items_center()
                .child(Checkbox::new("welcome-hide").label(copy::WELCOME_HIDE.short).checked(self.guide.dont_show()).on_click(cx.listener(
                    |v, checked: &bool, _, cx| {
                        v.guide.set_dont_show(*checked);
                        cx.notify();
                    },
                )))
                .child(disclosure::info_tip("welcome-hide-info", &copy::WELCOME_HIDE, &app))
                .into_any_element()
        } else {
            Button::new("welcome-skip")
                .ghost()
                .label(copy::WELCOME_SKIP.short)
                .tooltip(copy::WELCOME_SKIP.tip)
                .on_click(cx.listener(|v, _, window, cx| {
                    let m = v.guide.skip();
                    v.close(m, window, cx);
                }))
                .into_any_element()
        };
        let back = kit::with_reason(
            "welcome-back-why",
            Button::new("welcome-back").outline().icon(icons::icon(role::BACK)).label(copy::BTN_BACK.short).on_click(cx.listener(
                |v, _, _, cx| {
                    v.guide.back();
                    cx.notify();
                },
            )),
            self.guide.back_reason().map(str::to_string),
        );
        let primary = Button::new("welcome-next").primary().label(self.guide.primary().short).on_click(cx.listener(|v, _, window, cx| {
            let m = v.guide.next();
            v.close(m, window, cx);
        }));
        let footer = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(left)
            .child(h_flex().gap(px(8.0)).items_center().child(back).child(primary));

        let flow = c.flow.then(|| self.flow());
        let dots = self.dots(cx);
        v_flex()
            .id("welcome")
            .track_focus(&self.focus)
            .w_full()
            .gap(px(16.0))
            .px(px(16.0))
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.modifiers == gpui_kit::Modifiers::default() && v.guide.key(&ev.keystroke.key) {
                    cx.notify();
                }
            }))
            .child(tile)
            // the same height on every step, so the buttons do not jump while you click through
            .child(v_flex().gap(px(16.0)).min_h(px(BODY_MIN_H)).child(title).child(lines).children(flow))
            .child(dots)
            .child(footer)
    }
}
