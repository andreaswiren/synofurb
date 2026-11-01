//! The log viewer shared by the install and failure screens: dark terminal palette in both themes, a virtualised
//! list, a minimum-level segmented filter with live counts, text search, follow-output, select all and copy.

use crate::ui::app::App;
use crate::ui::logview::{LEVELS, format_line, level_name, level_tag};
use crate::ui::theme::{self, Palette, Themed, radius, space};
use crate::ui::widgets::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Sizable, h_flex, v_flex};
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
    /// The whole viewer: toolbar plus list. `height` is the list height in px (`None` = fill).
    pub(crate) fn render_log_viewer(&mut self, height: Option<f32>, cx: &mut Context<App>) -> AnyElement {
        let p = cx.pal();
        let log = &self.install.log;
        let min = log.filter().min_level;

        let seg = h_flex().flex_none().rounded(px(radius::MD)).border_1().border_color(p.border_strong).overflow_hidden().children(
            LEVELS.iter().enumerate().map(|(i, lv)| {
                let on = *lv == min;
                let lv = *lv;
                h_flex()
                    .id(("log-level", i))
                    .h(px(28.0))
                    .px(px(space::S12))
                    .gap(px(space::S4))
                    .items_center()
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label(format!("Show {} and above, {} lines", level_name(lv), log.count_at_least(lv)))
                    .bg(if on { p.accent_bg } else { p.surface })
                    .hover(|s| s.bg(p.surface_2))
                    .when(i > 0, |d| d.border_l_1().border_color(p.border_strong))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.install.log.set_min_level(lv);
                        this.log_selected_all = false;
                        cx.notify();
                    }))
                    .child(
                        text(&p, theme::CAPTION, level_name(lv))
                            .text_color(if on { p.accent_fg } else { p.fg_2 })
                            .font_weight(FontWeight(if on { 700.0 } else { 400.0 })),
                    )
                    .child(text(&p, theme::CAPTION, log.count_at_least(lv).to_string()).text_color(p.placeholder))
            }),
        );

        let toolbar = h_flex()
            .w_full()
            .gap(px(space::S8))
            .items_center()
            .flex_wrap()
            .child(seg)
            .child(div().w(px(200.0)).child(Input::new(&self.inp.log_search).cleanable(true).aria_label("Search the log")))
            .child(Checkbox::new("log-follow").label("Follow output").checked(self.log_follow).on_click(cx.listener(
                |this, v: &bool, _, cx| {
                    this.log_follow = *v;
                    if *v {
                        this.scroll_log_to_end();
                    }
                    cx.notify();
                },
            )))
            .child(div().flex_1())
            .child(
                muted(&p, format!("{} lines, {} warnings or errors", log.len(), log.count_at_least(Level::Warn)))
                    .text_size(theme::CAPTION.size()),
            )
            .child(btn_secondary("log-first-problem", "First warning").small().icon(icon("triangle-alert")).on_click(cx.listener(
                |this, _, _, cx| {
                    if let Some(i) = this.install.log.first_problem() {
                        this.log_follow = false;
                        this.log_scroll.scroll_to_item(i, gpui_kit::ScrollStrategy::Top);
                    }
                    cx.notify();
                },
            )))
            .child(
                btn_secondary("log-select", "Select all")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.log_select_all(window, cx))),
            )
            .child(
                btn_secondary("log-copy", "Copy")
                    .small()
                    .icon(icon("copy"))
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
                            .gap(px(space::S8))
                            .pr(px(space::S8))
                            .when(selected, |d| d.bg(p.accent.opacity(0.28)))
                            .when(problem && !selected, |d| d.bg(if l.level == Level::Error { p.log_err_bg } else { p.log_wrn_bg }))
                            .child(div().w(px(3.0)).h_full().flex_none().when(problem, |d| d.bg(c)))
                            .child(mono(&p, l.ts.clone(), cx).text_size(theme::LOG.size()).w(px(70.0)).flex_none().text_color(p.log_dim))
                            .child(
                                h_flex()
                                    .w(px(50.0))
                                    .flex_none()
                                    .gap(px(space::S4))
                                    .child(mono(&p, glyph, cx).text_size(theme::LOG.size()).text_color(c))
                                    .child(mono(&p, word, cx).text_size(theme::LOG.size()).text_color(c).font_weight(FontWeight(600.0))),
                            )
                            .child(
                                mono(&p, l.msg.clone(), cx)
                                    .text_size(theme::LOG.size())
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(if l.level == Level::Debug { p.log_dim } else { p.log_fg }),
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
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.log_focus, cx);
                }),
            )
            .role(Role::Log)
            .aria_label("Install log")
            .w_full()
            .rounded(px(radius::MD))
            .bg(p.log_bg)
            .overflow_hidden()
            .border_1()
            .border_color(p.border)
            .when_some(height, |d, h| d.h(px(h)))
            .when(height.is_none(), |d| d.flex_1().min_h(px(160.0)))
            .child(if self.install.log.is_empty() {
                div().p(px(space::S16)).child(mono(&p, "No lines to show.", cx).text_color(p.log_dim)).into_any_element()
            } else {
                list.into_any_element()
            });

        v_flex().w_full().flex_1().min_h_0().gap(px(space::S8)).child(toolbar).child(area).into_any_element()
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
