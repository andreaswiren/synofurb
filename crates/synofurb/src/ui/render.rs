//! The window chrome around every screen: step rail (left), safety bar (top), update banner, scrolling page,
//! pinned action bar (bottom). Screens return a [`Page`]; this file lays it out.

use super::app::{App, UpdateState, version};
use super::state::*;
use super::theme::{self, Palette, Themed, Tone, layout, radius, space};
use super::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FontWeight, IntoElement, KeyBinding, Render, Role, SharedString, Window, actions, div, px};

actions!(synofurb, [GoBack, GoForward, PrimaryAction, OpenSettings]);

/// `cmd-<k>` on macOS, `ctrl-<k>` elsewhere (a `&'static str`, as `KeyBinding::new` needs).
macro_rules! modkey {
    ($k:literal) => {
        if cfg!(target_os = "macos") { concat!("cmd-", $k) } else { concat!("ctrl-", $k) }
    };
}
pub(crate) use modkey;

pub const KEY_CONTEXT: &str = "SynoApp";

pub fn bind_keys(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("alt-left", GoBack, Some(KEY_CONTEXT)),
        KeyBinding::new("alt-right", GoForward, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", PrimaryAction, Some(KEY_CONTEXT)),
        KeyBinding::new(modkey!(","), OpenSettings, Some(KEY_CONTEXT)),
    ]);
}

/// What a screen hands to the chrome.
pub struct Page {
    pub title: SharedString,
    pub subtitle: Option<SharedString>,
    pub header_right: Option<AnyElement>,
    pub body: AnyElement,
    /// Left side of the action bar: the reason a button is disabled, a status, ...
    pub note: Option<AnyElement>,
    /// Right side of the action bar, in order (safe action rightmost unless the screen says otherwise).
    pub buttons: Vec<AnyElement>,
    /// The body scrolls inside the page. Screens with their own scrolling region (the install log) turn this off.
    pub scroll: bool,
}

impl Page {
    pub fn new(title: impl Into<SharedString>, body: impl IntoElement) -> Page {
        Page {
            title: title.into(),
            subtitle: None,
            header_right: None,
            body: body.into_any_element(),
            note: None,
            buttons: vec![],
            scroll: true,
        }
    }
    pub fn subtitle(mut self, s: impl Into<SharedString>) -> Page {
        self.subtitle = Some(s.into());
        self
    }
    pub fn note(mut self, e: impl IntoElement) -> Page {
        self.note = Some(e.into_any_element());
        self
    }
    pub fn buttons(mut self, b: Vec<AnyElement>) -> Page {
        self.buttons = b;
        self
    }
    pub fn no_scroll(mut self) -> Page {
        self.scroll = false;
        self
    }
}

impl App {
    fn page_for(&mut self, window: &mut Window, cx: &mut Context<App>) -> Page {
        match self.screen {
            Screen::Inventory => self.page_inventory(window, cx),
            Screen::Connect => self.page_connect(window, cx),
            Screen::Inspect => self.page_inspect(window, cx),
            Screen::Goal => self.page_goal(window, cx),
            Screen::Plan => self.page_plan(window, cx),
            Screen::System => self.page_system(window, cx),
            Screen::Confirm => self.page_confirm(window, cx),
            Screen::Install => self.page_install(window, cx),
            Screen::Failed => self.page_failed(window, cx),
            Screen::Done => self.page_done(window, cx),
            Screen::Settings => self.page_settings(window, cx),
            Screen::BootMgr => self.page_bootmgr(window, cx),
            Screen::Recover => self.page_recover(window, cx),
        }
    }

    /// Back / forward targets for Alt+Left / Alt+Right and the Back button.
    pub fn back_target(&self) -> Option<Screen> {
        Some(match self.screen {
            Screen::Connect => Screen::Inventory,
            Screen::Inspect => Screen::Connect,
            Screen::Goal => Screen::Inspect,
            Screen::Plan | Screen::BootMgr | Screen::Recover => Screen::Goal,
            Screen::System => Screen::Plan,
            Screen::Confirm => Screen::System,
            Screen::Settings => self.return_to,
            _ => return None,
        })
    }

    fn forward_target(&self) -> Option<Screen> {
        // Forward only through steps that were already completed.
        let cur = self.current_step();
        let next = Step::ALL.iter().find(|s| **s == cur).and_then(|s| Step::ALL.get(s.number()).copied())?;
        if self.install_started || next >= Step::Install {
            return None;
        }
        match (self.screen, self.facts.is_some()) {
            (Screen::Inventory, true) => Some(Screen::Connect),
            (Screen::Connect, true) => Some(Screen::Inspect),
            (Screen::Inspect, true) => Some(Screen::Goal),
            _ => None,
        }
    }

    fn render_rail(&self, p: &Palette, cx: &mut Context<App>) -> AnyElement {
        let cur = self.current_step();
        let running = self.install_running();
        v_flex()
            .w(px(layout::RAIL_W))
            .flex_none()
            .h_full()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.border)
            .p(px(space::S12))
            .gap(px(space::S4))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(space::S8))
                    .px(px(space::S8))
                    .py(px(space::S12))
                    .child(
                        div().size(px(28.0)).rounded(px(radius::SM)).bg(p.accent).flex().items_center().justify_center().child(icon_sized(
                            "server",
                            16.0,
                            p.on_accent,
                        )),
                    )
                    .child(text(p, theme::H2, "synofurb")),
            )
            .children(Step::ALL.iter().enumerate().map(|(i, step)| {
                let state = step_state(cur, *step);
                let clickable = can_jump(cur, *step, self.install_started) && !running;
                let (label_color, weight) = match state {
                    StepState::Current => (p.accent_fg, 650.0),
                    StepState::Done => (p.fg, 500.0),
                    StepState::Upcoming => (p.fg_2, 400.0),
                };
                let circle = match state {
                    StepState::Done => {
                        div().size(px(22.0)).rounded_full().bg(p.accent).flex().items_center().justify_center().child(icon_sized(
                            "check",
                            14.0,
                            p.on_accent,
                        ))
                    }
                    StepState::Current => div()
                        .size(px(22.0))
                        .rounded_full()
                        .border_2()
                        .border_color(p.accent)
                        .bg(p.surface)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(text(p, theme::CAPTION, step.number().to_string()).text_color(p.accent_fg)),
                    StepState::Upcoming => div()
                        .size(px(22.0))
                        .rounded_full()
                        .border_1()
                        .border_color(p.border_strong)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(text(p, theme::CAPTION, step.number().to_string()).text_color(p.fg_2)),
                };
                let state_word = match state {
                    StepState::Done => "completed",
                    StepState::Current => "current step",
                    StepState::Upcoming => "not reached yet",
                };
                let step = *step;
                h_flex()
                    .id(("rail-step", i))
                    .h(px(36.0))
                    .px(px(space::S8))
                    .gap(px(space::S12))
                    .items_center()
                    .rounded(px(radius::MD))
                    .role(Role::ListItem)
                    .aria_label(format!("Step {}: {}, {}", step.number(), step.label(), state_word))
                    .when(state == StepState::Current, |d| d.bg(p.accent_bg))
                    .when(clickable, |d| {
                        d.cursor_pointer().hover(|s| s.bg(p.surface_2)).on_click(cx.listener(move |this, _, window, cx| {
                            this.go(step.screen(), window, cx);
                        }))
                    })
                    .child(circle)
                    .child(text(p, theme::BODY, step.label()).text_color(label_color).font_weight(FontWeight(weight)))
            }))
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap(px(space::S8))
                    .child(
                        Button::new("open-settings")
                            .ghost()
                            .small()
                            .icon(icon("settings"))
                            .label("Settings and About")
                            .disabled(self.install_running())
                            .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Settings, window, cx))),
                    )
                    .child(
                        muted(p, format!("v{}{}", version(), if self.demo.is_some() { " (demo)" } else { "" }))
                            .px(px(space::S8))
                            .text_size(theme::CAPTION.size()),
                    ),
            )
            .into_any_element()
    }

    fn render_safety_bar(&self, p: &Palette, cx: &mut Context<App>) -> AnyElement {
        let risk = risk_for(self.screen, self.install_started, self.install.past_no_return);
        let touched = risk == Risk::DisksModified;
        let unit = self.facts.as_ref().map(|f| {
            let model = if f.model.is_empty() { f.hostname.clone() } else { f.model.clone() };
            let host = super::format::display_host(&self.input_value(&self.inp.host, cx));
            format!("{model}  {host}")
        });
        let row = h_flex()
            .w_full()
            .gap(px(space::S4))
            .items_center()
            .flex_wrap()
            .child(h_flex().flex_none().gap(px(space::S4)).items_center().children(Risk::ALL.iter().map(|r| {
                let reached = *r <= risk;
                let current = *r == risk;
                let tone = if *r == Risk::DisksModified { Tone::Err } else { Tone::Accent };
                let c = p.tone(tone);
                h_flex()
                    .h(px(24.0))
                    .px(px(space::S8))
                    .gap(px(space::S4))
                    .items_center()
                    .rounded(px(radius::SM))
                    .border_1()
                    .border_color(if current { c.solid } else { p.border })
                    .bg(if current { c.bg } else { p.surface })
                    .child(icon_sized(
                        if current {
                            if *r == Risk::DisksModified { "triangle-alert" } else { "shield-check" }
                        } else if reached {
                            "check"
                        } else {
                            "circle-dashed"
                        },
                        13.0,
                        if current {
                            c.fg
                        } else if reached {
                            p.fg_2
                        } else {
                            p.placeholder
                        },
                    ))
                    .child(
                        text(p, theme::CAPTION, r.label())
                            .text_color(if current {
                                c.fg
                            } else if reached {
                                p.fg_2
                            } else {
                                p.placeholder
                            })
                            .font_weight(FontWeight(if current { 700.0 } else { 400.0 })),
                    )
            })))
            .child(div().flex_1())
            .when_some(unit, |d, u| d.child(chip(p, u)))
            .child(badge(
                p,
                if touched { Tone::Err } else { Tone::Ok },
                if touched { "triangle-alert" } else { "shield-check" },
                if touched { "Disks touched: Yes" } else { "Disks touched: No" },
                false,
            ));
        v_flex()
            .w_full()
            .flex_none()
            .px(px(layout::PAGE_PAD))
            .py(px(space::S8))
            .gap(px(space::S4))
            .bg(p.surface)
            .border_b_1()
            .border_color(p.border)
            .id("safety-bar")
            .role(Role::Status)
            .aria_label(format!("Safety state: {}. {}", risk.label(), risk.sentence()))
            .child(row)
            .child(muted(p, risk.sentence()).text_size(theme::CAPTION.size()))
            .into_any_element()
    }

    fn render_update_banner(&self, p: &Palette, cx: &mut Context<App>) -> Option<AnyElement> {
        if self.install_started && self.result.is_none() {
            return None; // never while a conversion runs
        }
        if matches!(self.screen, Screen::Settings) {
            return None;
        }
        let UpdateState::Available(info) = &self.update else { return None };
        let b = banner(
            p,
            Tone::Accent,
            format!("synofurb {} is available", info.latest),
            format!("You have {}. Release notes, size and verification are in the details.", info.current),
            vec![
                btn_secondary("upd-details", "View details")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.open_update_dialog(window, cx)))
                    .into_any_element(),
                btn_ghost("upd-later", "Remind me tomorrow")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.remind_tomorrow(cx)))
                    .into_any_element(),
            ],
        );
        Some(div().px(px(layout::PAGE_PAD)).pt(px(space::S12)).child(b).into_any_element())
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.pal();
        let page = self.page_for(window, cx);
        let rail = self.render_rail(&p, cx);
        let safety = self.render_safety_bar(&p, cx);
        let update = self.render_update_banner(&p, cx);

        let header = h_flex()
            .w_full()
            .items_start()
            .justify_between()
            .gap(px(space::S16))
            .child(
                v_flex()
                    .gap(px(space::S4))
                    .min_w_0()
                    .child(h1(&p, page.title.clone()).id("page-title").role(Role::Heading).aria_label(page.title.clone()))
                    .when_some(page.subtitle.clone(), |d, s| d.child(body(&p, s).text_color(p.fg_2))),
            )
            .when_some(page.header_right, |d, r| d.child(r));

        let content = v_flex().w_full().gap(px(space::S16)).p(px(layout::PAGE_PAD)).child(header).child(page.body);
        let content_area = if page.scroll {
            div()
                .id("page")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .track_scroll(&self.page_scroll)
                .vertical_scrollbar(&self.page_scroll)
                .child(content)
                .into_any_element()
        } else {
            div().flex_1().min_h_0().w_full().flex().flex_col().child(content.h_full().min_h_0()).into_any_element()
        };

        let action_bar = h_flex()
            .w_full()
            .flex_none()
            .px(px(layout::PAGE_PAD))
            .py(px(space::S12))
            .gap(px(space::S16))
            .items_center()
            .bg(p.surface)
            .border_t_1()
            .border_color(p.border)
            .child(div().flex_1().min_w_0().when_some(page.note, |d, n| d.child(n)))
            .child(h_flex().flex_none().gap(px(space::S8)).items_center().children(page.buttons));

        div()
            .id("synofurb")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &GoBack, window, cx| {
                if !this.install_running()
                    && let Some(t) = this.back_target()
                {
                    this.go(t, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| {
                if let Some(t) = this.forward_target() {
                    this.go(t, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                if !this.install_running() && this.screen != Screen::Settings {
                    this.go(Screen::Settings, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PrimaryAction, window, cx| this.primary_action(window, cx)))
            .size_full()
            .flex()
            .flex_row()
            .bg(p.canvas)
            .text_color(p.fg)
            .text_size(theme::BODY.size())
            .child(rail)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(safety)
                    .when_some(update, |d, u| d.child(u))
                    .child(content_area)
                    .child(action_bar),
            )
    }
}

impl App {
    /// Enter outside a text field: the primary action of the screen. Never the destructive action.
    pub fn primary_action(&mut self, window: &mut Window, cx: &mut Context<App>) {
        match self.screen {
            Screen::Inventory => {
                if self.selected_ip.is_some() {
                    let ip = self.selected_ip.clone().unwrap_or_default();
                    self.select_host(&ip, window, cx);
                }
            }
            Screen::Connect => self.start_inspect(window, cx),
            Screen::Inspect => {
                if !self.facts.as_ref().is_some_and(|f| self.compat_for_facts(f).blocking) {
                    self.go(Screen::Goal, window, cx)
                }
            }
            Screen::Goal => self.continue_from_goal(window, cx),
            Screen::Plan => {
                if self.plan_ok() {
                    self.go(Screen::System, window, cx)
                }
            }
            Screen::System => self.go(Screen::Confirm, window, cx),
            Screen::Done | Screen::Failed => {}
            _ => {}
        }
    }
}
