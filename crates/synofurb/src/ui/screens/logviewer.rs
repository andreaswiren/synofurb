//! The log viewer shared by the install and failure screens (no log widget exists, so: `ToggleGroup` for the level,
//! `Input` for the search, a virtualised `uniform_list` of mono rows with the themed scrollbar). The dark terminal
//! palette is kept in both themes, as the design says.

use crate::ui::app::App;
use crate::ui::icons::{self, role};
use crate::ui::logview::{LEVELS, format_line, level_name, level_tag};
use crate::ui::theme::{self, Palette, Themed};
use gpui_kit::component::button::{Button, ButtonVariants, Toggle, ToggleGroup};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FontWeight, IntoElement, Role, Window, actions, div, px, uniform_list};
use synofurb_core::logs::Level;

actions!(synofurb_log, [LogSelectAll, LogCopy, LogFind]);

pub const LOG_CONTEXT: &str = "SynoLog";

pub fn bind_keys(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    cx.bind_keys([
        KeyBinding::new(crate::ui::render::modkey!("a"), LogSelectAll, Some(LOG_CONTEXT)),
        KeyBinding::new(crate::ui::render::modkey!("c"), LogCopy, Some(LOG_CONTEXT)),
        KeyBinding::new(crate::ui::render::modkey!("f"), LogFind, Some(LOG_CONTEXT)),
    ]);
}

fn level_color(p: &Palette, l: Level) -> gpui_kit::Hsla {
    match l {
        Level::Debug => p.log_dbg,
        Level::Info => p.log_inf,
        Level::Warn => p.log_wrn,
        Level::Error => p.log_err,
    }
}

impl App {
    /// Toolbar plus list. `height` is the list height in px (`None` = fill).
    pub(crate) fn render_log_viewer(&mut self, height: Option<f32>, cx: &mut Context<App>) -> AnyElement {
        let p = cx.pal();
        let app = cx.entity();
        let log = &self.install.log;
        let min = log.filter().min_level;
        let sel = LEVELS.iter().position(|l| *l == min).unwrap_or(0);

        let level_group = ToggleGroup::new("log-levels")
            .segmented()
            .small()
            .children(
                LEVELS
                    .iter()
                    .enumerate()
                    .map(|(i, lv)| Toggle::new(i).label(format!("{} {}", level_name(*lv), log.count_at_least(*lv))).checked(i == sel)),
            )
            .on_click({
                let app = app.clone();
                move |checks: &Vec<bool>, _, cx| {
                    if let Some(ix) = checks.iter().enumerate().find(|(i, on)| **on && *i != sel).map(|(i, _)| i) {
                        app.update(cx, |a, cx| {
                            a.install.log.set_min_level(LEVELS[ix]);
                            a.log_selected_all = false;
                            cx.notify();
                        });
                    }
                }
            });
        let toolbar = h_flex()
            .w_full()
            .gap(px(8.0))
            .items_center()
            .flex_wrap()
            .child(level_group)
            .child(
                div()
                    .w(px(200.0))
                    .child(Input::new(&self.inp.log_search).prefix(icons::icon(role::SEARCH)).cleanable(true).aria_label("Search the log")),
            )
            .child(Checkbox::new("log-follow").label("Follow").checked(self.log_follow).on_click(cx.listener(|this, v: &bool, _, cx| {
                this.log_follow = *v;
                if *v {
                    this.scroll_log_to_end();
                }
                cx.notify();
            })))
            .child(div().flex_1())
            .child(
                Button::new("log-first-problem")
                    .ghost()
                    .small()
                    .icon(icons::icon(role::WARN))
                    .tooltip("First warning")
                    .accessibility_label("First warning")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(i) = this.install.log.first_problem() {
                            this.log_follow = false;
                            this.log_scroll.scroll_to_item(i, gpui_kit::ScrollStrategy::Top);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("log-copy")
                    .ghost()
                    .small()
                    .icon(icons::icon(role::COPY))
                    .tooltip("Copy")
                    .accessibility_label("Copy")
                    .on_click(cx.listener(|this, _, window, cx| this.log_copy(window, cx))),
            );

        let n = log.visible_len();
        let selected = self.log_selected_all;
        let list = uniform_list(
            "install-log",
            n,
            cx.processor(move |this: &mut App, range: std::ops::Range<usize>, _window: &mut Window, cx: &mut Context<App>| {
                let p = cx.pal();
                range
                    .filter_map(|i| this.install.log.visible_line(i).map(|l| (i, l.clone())))
                    .map(|(i, l)| {
                        let (glyph, word) = level_tag(l.level);
                        let c = level_color(&p, l.level);
                        let problem = l.level >= Level::Warn;
                        h_flex()
                            .id(("log-line", i))
                            .h(px(theme::LOG.line_px))
                            .w_full()
                            .flex_none()
                            .items_center()
                            .gap(px(8.0))
                            .pr(px(8.0))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(theme::LOG.size())
                            .when(selected, |d| d.bg(p.accent.opacity(0.28)))
                            .when(problem && !selected, |d| d.bg(if l.level == Level::Error { p.log_err_bg } else { p.log_wrn_bg }))
                            .child(div().w(px(3.0)).h_full().flex_none().when(problem, |d| d.bg(c)))
                            .child(div().w(px(70.0)).flex_none().text_color(p.log_dim).child(l.ts.clone()))
                            .child(
                                h_flex()
                                    .w(px(50.0))
                                    .flex_none()
                                    .gap(px(4.0))
                                    .text_color(c)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(glyph)
                                    .child(word),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(if l.level == Level::Debug { p.log_dim } else { p.log_fg })
                                    .child(l.msg.clone()),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.log_scroll)
        .size_full();

        let area = div()
            .id("log-area")
            .key_context(LOG_CONTEXT)
            .track_focus(&self.log_focus)
            .on_action(cx.listener(|this, _: &LogSelectAll, window, cx| this.log_select_all(window, cx)))
            .on_action(cx.listener(|this, _: &LogCopy, window, cx| this.log_copy(window, cx)))
            .on_action(cx.listener(|this, _: &LogFind, window, cx| {
                let f = gpui_kit::Focusable::focus_handle(&this.inp.log_search, cx);
                window.focus(&f, cx);
            }))
            .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| window.focus(&this.log_focus, cx)))
            .role(Role::Log)
            .aria_label("Install log")
            .relative()
            .w_full()
            .rounded(cx.theme().radius)
            .bg(p.log_bg)
            .overflow_hidden()
            .border_1()
            .border_color(cx.theme().border)
            .h(px(height.unwrap_or(240.0)))
            .child(if self.install.log.is_empty() {
                div().p(px(16.0)).font_family(cx.theme().mono_font_family.clone()).text_color(p.log_dim).child("-").into_any_element()
            } else {
                list.into_any_element()
            })
            .vertical_scrollbar(&self.log_scroll);

        v_flex().w_full().gap(px(8.0)).child(toolbar).child(area).into_any_element()
    }

    pub(crate) fn log_select_all(&mut self, _window: &mut Window, cx: &mut Context<App>) {
        self.log_selected_all = true;
        cx.notify();
    }

    pub(crate) fn log_copy(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let text = self.install.log.visible_text();
        let n = self.install.log.visible_len();
        self.copy_text(text, &format!("{n} log lines"), window, cx);
    }

    /// Line text for tests and export.
    #[allow(dead_code)]
    pub(crate) fn log_line_text(&self, i: usize) -> Option<String> {
        self.install.log.visible_line(i).map(format_line)
    }
}
