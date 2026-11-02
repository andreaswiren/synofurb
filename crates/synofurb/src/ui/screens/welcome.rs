//! D1b Welcome guide: a modal with five short steps (one icon, one headline in the dialog title, at most two lines,
//! details behind an info button), a pagination row, Back / Next / Skip and, on the last step, "Don't show again" and
//! "Find NAS". The rules live in `ui::welcome` (pure, tested); this file only draws them.
//!
//! * The step dots are the stock `CarouselPagination` over a `CarouselState` (buttons with a "Go to step N" name).
//! * The text block has one height on every step and at every text size: all five steps are laid out invisibly in the
//!   same grid cell and the live step is drawn on top, so the cell is as tall as the tallest step by construction.
//! * Focus starts on the primary button (Enter works); Left and Right move between steps from anywhere in the dialog.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure;
use crate::ui::icons::{self, role};
use crate::ui::theme::{Themed, Tone};
use crate::ui::welcome::{FLOW, Guide, Moved, STEP_COUNT, STEPS, StepContent};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::carousel::{CarouselPagination, CarouselPaginationItem, CarouselState};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render, Subscription, Window, div, px,
    rems,
};

pub struct WelcomeView {
    app: Entity<App>,
    guide: Guide,
    focus: FocusHandle,
    primary_focus: FocusHandle,
    back_focus: FocusHandle,
    skip_focus: FocusHandle,
    pager: Entity<CarouselState>,
    _sub: Subscription,
}

impl WelcomeView {
    /// `dont_show` is the state of the "Don't show again" box: ticked on the first launch, the stored flag on a replay.
    pub fn new(app: Entity<App>, step: usize, dont_show: bool, cx: &mut Context<WelcomeView>) -> WelcomeView {
        let guide = Guide::new(step, dont_show);
        let pager = cx.new(|_| CarouselState::new(STEP_COUNT).with_selected_index(guide.step()));
        let sub = cx.observe(&pager, |_, _, cx| cx.notify());
        WelcomeView {
            app,
            guide,
            focus: cx.focus_handle(),
            primary_focus: cx.focus_handle(),
            back_focus: cx.focus_handle(),
            skip_focus: cx.focus_handle(),
            pager,
            _sub: sub,
        }
    }

    /// Where focus goes when the dialog opens: the primary button, so Enter does what it says.
    pub fn primary_focus(&self) -> &FocusHandle {
        &self.primary_focus
    }

    /// The headline, shown as the dialog title (so the dialog has an accessible name).
    pub fn title(&self) -> &'static str {
        self.guide.content().title.short
    }

    /// What closing now writes to the settings (see `Guide::stores_on_close`).
    pub fn stores_on_close(&self) -> Option<bool> {
        self.guide.stores_on_close()
    }

    pub fn app(&self) -> &Entity<App> {
        &self.app
    }

    /// Write the "Don't show again" decision to the settings (only when it changes).
    pub fn store(app: &Entity<App>, value: Option<bool>, cx: &mut gpui_kit::App) {
        let Some(v) = value else { return };
        app.update(cx, |a, _| {
            if a.settings.welcome_done != v {
                a.settings.welcome_done = v;
                a.save_settings();
            }
        });
    }

    fn close(&mut self, moved: Moved, window: &mut Window, cx: &mut Context<WelcomeView>) {
        if moved == Moved::Closed {
            Self::store(&self.app, self.guide.stores_on_close(), cx);
            window.close_dialog(cx);
        } else {
            self.show_step(cx);
        }
    }

    /// Enter: the dialog binds it to "confirm" before a focused button sees it, so the dialog's `on_ok` calls this and the
    /// control that has focus decides: Back goes back, Skip skips, anything else is the primary button.
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<WelcomeView>) {
        if self.back_focus.is_focused(window) && self.guide.shows_back() {
            self.guide.back();
            self.show_step(cx);
        } else if self.skip_focus.is_focused(window) && self.guide.shows_skip() {
            let m = self.guide.skip();
            self.close(m, window, cx);
        } else {
            let m = self.guide.next();
            self.close(m, window, cx);
        }
    }

    /// The pagination follows the guide.
    fn show_step(&mut self, cx: &mut Context<WelcomeView>) {
        let step = self.guide.step();
        self.pager.update(cx, |p, cx| p.set_selected_index(step, cx));
        cx.notify();
    }

    /// The six wizard steps as a row: the window's Stepper look (current bold, the rest muted), each with a caption
    /// saying what the step is. Not clickable here.
    fn flow(&self, id: &'static str, cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let items = FLOW.iter().enumerate().map(|(i, (label, caption))| {
            // a caption wraps instead of running into its neighbour at large text
            StepperItem::new().child(
                v_flex()
                    .max_w(rems(5.5))
                    .child(
                        div()
                            .text_sm()
                            .when(i == 0, |d| d.font_weight(FontWeight::BOLD))
                            .when(i != 0, |d| d.text_color(muted))
                            .child(label.short),
                    )
                    .child(div().text_xs().text_color(muted).child(caption.short)),
            )
        });
        div().w_full().child(Stepper::new(id).disabled(true).selected_index(0).items(items)).into_any_element()
    }

    /// The text block of one step. `live` adds the info button; the invisible copies used to reserve the height do not.
    fn body(&self, c: &'static StepContent, live: Option<usize>, cx: &Context<Self>) -> AnyElement {
        let p = cx.pal();
        let tone = if c.warn { p.tone(Tone::Warn) } else { p.tone(Tone::Accent) };
        let tile = div()
            .size(px(40.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.surface_2)
            .child(icons::icon_sized(c.icon, 24.0, tone.fg));
        // the info button sits right behind the last visible line (the note, when the step has one)
        let info = |d: gpui_kit::Div| match (live, c.info) {
            (Some(step), Some(info)) => d.child(disclosure::info_tip(("welcome-info", step), info, &self.app)),
            _ => d,
        };
        let lines = v_flex().gap(px(4.0)).text_sm().text_color(cx.theme().foreground).children(c.lines.iter().enumerate().map(|(i, l)| {
            let row = h_flex().gap(px(4.0)).items_center().child(div().child(l.short));
            if c.note.is_none() && i == c.lines.len() - 1 { info(row) } else { row }
        }));
        let note =
            c.note.map(|n| info(h_flex().gap(px(4.0)).items_center().text_sm().text_color(cx.theme().muted_foreground).child(n.short)));
        v_flex()
            .gap(px(12.0))
            .child(tile)
            .child(lines)
            .children(note)
            .when(c.flow, |d| d.child(self.flow(if live.is_some() { "welcome-steps" } else { "welcome-steps-ghost" }, cx)))
            .into_any_element()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // a click on a pagination button moved the pager: the guide follows
        if let Some(i) = self.pager.read(cx).selected_index()
            && i != self.guide.step()
        {
            self.guide.go_to(i);
        }
        let p = cx.pal();
        let step = self.guide.step();

        // one grid cell holds all five steps, four of them invisible: the cell is as tall as the tallest step
        let body = div()
            .grid()
            .grid_cols(1)
            .w_full()
            .children(
                STEPS
                    .iter()
                    .enumerate()
                    .map(|(i, c)| div().id(("welcome-ghost", i)).col_start(1).row_start(1).opacity(0.0).child(self.body(c, None, cx))),
            )
            .child(div().id("welcome-live").col_start(1).row_start(1).child(self.body(self.guide.content(), Some(step), cx)));

        let pages = CarouselPagination::new().accessibility_label(copy::WELCOME_PAGES.short).children((0..STEP_COUNT).map(|i| {
            let now = i == step;
            CarouselPaginationItem::new(("welcome-page", i), i, &self.pager)
                .accessibility_label(copy::WELCOME_GOTO.fmt(&[&(i + 1).to_string()]))
                .child(div().h(px(8.0)).w(px(if now { 20.0 } else { 8.0 })).rounded_full().bg(if now { p.accent } else { p.border_strong }))
        }));

        // footer: Skip (or the checkbox) on the left, Back + the one primary on the right
        let left: AnyElement = if self.guide.shows_checkbox() {
            Checkbox::new("welcome-hide")
                .label(copy::WELCOME_HIDE.short)
                .checked(self.guide.dont_show())
                .on_click(cx.listener(|v, checked: &bool, _, cx| {
                    v.guide.set_dont_show(*checked);
                    cx.notify();
                }))
                .into_any_element()
        } else {
            Button::new("welcome-skip")
                .ghost()
                .track_focus(&self.skip_focus)
                .label(copy::WELCOME_SKIP.short)
                .tooltip(copy::WELCOME_SKIP.tip)
                .on_click(cx.listener(|v, _, window, cx| {
                    let m = v.guide.skip();
                    v.close(m, window, cx);
                }))
                .into_any_element()
        };
        let back = self.guide.shows_back().then(|| {
            Button::new("welcome-back")
                .outline()
                .track_focus(&self.back_focus)
                .icon(icons::icon(role::BACK))
                .label(copy::BTN_BACK.short)
                .on_click(cx.listener(|v, _, _, cx| {
                    v.guide.back();
                    v.show_step(cx);
                }))
        });
        let primary = Button::new("welcome-next").primary().track_focus(&self.primary_focus).label(self.guide.primary().short).on_click(
            cx.listener(|v, _, window, cx| {
                let m = v.guide.next();
                v.close(m, window, cx);
            }),
        );
        let footer = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(left)
            .child(h_flex().gap(px(8.0)).items_center().children(back).child(primary));

        v_flex()
            .id("welcome")
            .track_focus(&self.focus)
            .w_full()
            .gap(px(16.0))
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.modifiers == gpui_kit::Modifiers::default() && v.guide.key(&ev.keystroke.key) {
                    v.show_step(cx);
                }
            }))
            .child(body)
            .child(pages)
            .child(footer)
    }
}
