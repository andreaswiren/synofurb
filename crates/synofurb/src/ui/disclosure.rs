//! Progressive disclosure (BRIEF-v2): the screen shows little; the rest sits one deliberate action away.
//!
//! | Need | Helper |
//! |---|---|
//! | one sentence about a label or a disabled control | [`info_tip`] (tooltip on hover; Enter/Space on the icon opens the same text) |
//! | a paragraph plus a way to more | [`info_popover`] (same icon, no tooltip, the popover is the point) |
//! | the full explanation or a decision | [`dialog`] (a real modal, one primary action) |
//! | reference material | the Help sheet (`screens/help.rs`), opened with F1, `?` or the "Learn more" link |
//!
//! gpui-component's tooltip is pointer-only. To keep a keyboard path, the icon is a focusable button whose
//! accessible name already contains the tip (so a screen reader reads it on focus) and whose activation opens the
//! same sentence in a popover. That is the "hover AND keyboard focus" requirement, met with real widgets.

use super::app::App;
use super::copy::{self, Item};
use super::icons::{self, role};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, AnyElement, App as GpuiApp, ElementId, Entity, FontWeight, IntoElement, SharedString, Window, div, px};

/// Width of tooltip-sized popovers.
const POPOVER_W: f32 = 300.0;

/// Paragraphs of a `body` text (blank-line separated).
pub fn paragraphs(text: &'static str, cx: &GpuiApp) -> AnyElement {
    v_flex()
        .gap(px(8.0))
        .text_sm()
        .text_color(cx.theme().foreground)
        .children(text.split("\n\n").map(|p| div().child(SharedString::from(p.trim().to_string()))))
        .into_any_element()
}

/// The `(i)` icon. Hover: tooltip with the item's `tip`. Click / Enter / Space: popover with the tip, the body and a
/// "Learn more" link to the item's Help topic.
pub fn info_tip(id: impl Into<ElementId>, item: &'static Item, app: &Entity<App>) -> impl IntoElement {
    popover_for(id.into(), item, app, false)
}

/// Like [`info_tip`], but without tooltip: the popover is the point.
pub fn info_popover(id: impl Into<ElementId>, item: &'static Item, app: &Entity<App>) -> impl IntoElement {
    popover_for(id.into(), item, app, true)
}

fn popover_for(id: ElementId, item: &'static Item, app: &Entity<App>, no_tooltip: bool) -> Popover {
    let app = app.clone();
    let label: SharedString = if item.tip.is_empty() { item.short.into() } else { format!("{}: {}", item.short, item.tip).into() };
    let mut trigger = Button::new("info").ghost().xsmall().icon(icons::icon(role::TIP)).accessibility_label(label);
    if !no_tooltip && !item.tip.is_empty() {
        trigger = trigger.tooltip(item.tip);
    }
    Popover::new(id).anchor(Anchor::TopLeft).trigger(trigger).content(move |_, _, cx| {
        let popover = cx.entity();
        let app = app.clone();
        v_flex()
            .w(px(POPOVER_W))
            .gap(px(8.0))
            .text_sm()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(item.short))
            .when(!item.tip.is_empty(), |d| d.child(item.tip))
            .when(!item.body.is_empty(), |d| d.child(paragraphs(item.body, cx)))
            .when(!item.topic.is_empty(), |d| {
                d.child(h_flex().justify_end().child(
                    Button::new("learn-more").link().small().label(copy::BTN_LEARN_MORE.short).icon(icons::icon(role::BOOK)).on_click(
                        move |_, window, cx| {
                            popover.update(cx, |s, cx| s.dismiss(window, cx));
                            app.update(cx, |a, cx| a.open_help_topic(Some(item.topic), window, cx));
                        },
                    ),
                ))
            })
    })
}

/// A modal with the item's title and paragraphs and a Close button; a Help link when the item has a topic.
pub fn dialog(window: &mut Window, cx: &mut GpuiApp, item: &'static Item, app: &Entity<App>) {
    let app = app.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let app = app.clone();
        dialog
            .title(item.short)
            .w(px(super::theme::layout::DIALOG_W))
            .child(paragraphs(if item.body.is_empty() { item.tip } else { item.body }, cx))
            .footer(
                DialogFooter::new()
                    .when(!item.topic.is_empty(), |f| {
                        f.child(Button::new("dlg-help").ghost().icon(icons::icon(role::HELP)).label(copy::BTN_OPEN_HELP.short).on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                app.update(cx, |a, cx| a.open_help_topic(Some(item.topic), window, cx));
                            },
                        ))
                    })
                    .child(
                        Button::new("dlg-close").primary().label(copy::BTN_CLOSE.short).on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}

/// Like [`dialog`], but the primary button runs `on_ok` (and closes); "Cancel" closes without it.
pub fn dialog_with(
    window: &mut Window,
    cx: &mut GpuiApp,
    item: &'static Item,
    app: &Entity<App>,
    ok_label: &'static str,
    on_ok: impl Fn(&mut GpuiApp) + 'static,
) {
    let _ = app;
    let on_ok = std::rc::Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, cx| {
        let on_ok = on_ok.clone();
        dialog
            .title(item.short)
            .w(px(super::theme::layout::DIALOG_W))
            .child(paragraphs(if item.body.is_empty() { item.tip } else { item.body }, cx))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("dw-cancel").outline().label(copy::BTN_CANCEL.short).on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("dw-ok").primary().label(ok_label).on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        on_ok(cx);
                    })),
            )
    });
}
