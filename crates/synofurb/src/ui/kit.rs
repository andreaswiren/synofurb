//! Small helpers on top of the real gpui-component widgets: text styles, status tags, field labels with an info icon.
//! Nothing in here imitates a widget; each function returns (or configures) a stock one. Colours come from the
//! theme (`cx.theme()`) or, for the few status tones the Kit has no token for, from [`super::theme::Palette`].

use super::app::App;
use super::copy::Item;
use super::disclosure::info_tip;
use super::icons::{self, role};
use super::theme::{self, Palette, Themed, Tone};
use gpui_kit::assets::IconName;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App as GpuiApp, Div, ElementId, Entity, FontWeight, IntoElement, SharedString, div, px};
use synofurb_core::compat::Status;

// ------------------------------------------------------------------------------------------------ text

/// Muted caption text (12 px).
pub fn caption(cx: &GpuiApp, s: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(cx.theme().muted_foreground).child(s.into())
}

/// Secondary body text (14 px, muted).
pub fn muted(cx: &GpuiApp, s: impl Into<SharedString>) -> Div {
    div().text_sm().text_color(cx.theme().muted_foreground).child(s.into())
}

/// Semibold body text.
pub fn strong(s: impl Into<SharedString>) -> Div {
    div().text_sm().font_weight(FontWeight::SEMIBOLD).child(s.into())
}

/// Section heading (16 px semibold).
pub fn heading(s: impl Into<SharedString>) -> Div {
    div().text_size(theme::SECTION.size()).font_weight(FontWeight(theme::SECTION.weight)).child(s.into())
}

/// Monospace text (JetBrains Mono) for addresses, device paths, serials, hashes and commands.
pub fn mono(cx: &GpuiApp, s: impl Into<SharedString>) -> Div {
    div().font_family(cx.theme().mono_font_family.clone()).text_size(theme::MONO.size()).child(s.into())
}

/// Small monospace text (12 px).
pub fn mono_sm(cx: &GpuiApp, s: impl Into<SharedString>) -> Div {
    div().font_family(cx.theme().mono_font_family.clone()).text_xs().child(s.into())
}

// ------------------------------------------------------------------------------------------------ tags

/// A status [`Tag`]: icon plus word, in a tone family. Never colour alone.
pub fn tag(p: &Palette, tone: Tone, icon: Option<IconName>, label: impl Into<SharedString>) -> Tag {
    let c = p.tone(tone);
    let t = Tag::custom(c.bg, c.fg, c.border);
    let row = h_flex().gap(px(4.0)).items_center().when_some(icon, |d, i| d.child(icons::icon_sized(i, 12.0, c.fg))).child(label.into());
    t.child(row)
}

/// Neutral tag without an icon (facts such as "4 disks").
pub fn chip(p: &Palette, label: impl Into<SharedString>) -> Tag {
    tag(p, Tone::Neutral, None, label)
}

/// Tone and icon for a compatibility status.
pub fn status_style(s: Status) -> (Tone, IconName) {
    match s {
        Status::Verified => (Tone::Ok, role::COMPAT_VERIFIED),
        Status::Community => (Tone::Info, role::COMPAT_COMMUNITY),
        Status::Untested => (Tone::Neutral, role::COMPAT_UNTESTED),
        Status::KnownIssues => (Tone::Warn, role::COMPAT_ISSUES),
        Status::Unsupported => (Tone::Err, role::COMPAT_UNSUPPORTED),
    }
}

/// The compatibility [`Tag`]: icon + the fixed status term.
pub fn compat_tag(p: &Palette, s: Status) -> Tag {
    let (tone, icon) = status_style(s);
    tag(p, tone, Some(icon), s.label())
}

/// Icon that goes with a status tone.
pub fn tone_icon(tone: Tone) -> IconName {
    match tone {
        Tone::Ok => role::OK,
        Tone::Warn => role::WARN,
        Tone::Err => role::ERR,
        Tone::Info => role::INFO,
        Tone::Accent => role::SPARKLES,
        Tone::Neutral => role::CURRENT,
    }
}

// ------------------------------------------------------------------------------------------------ labels with an info icon

/// `Label (i)`: the field label followed by its info icon (tooltip on hover, popover on Enter, link to a Help topic).
pub fn labelled(id: impl Into<ElementId>, item: &'static Item, app: &Entity<App>) -> AnyElement {
    let id: ElementId = id.into();
    h_flex()
        .gap(px(2.0))
        .items_center()
        .child(item.short)
        .when(!item.tip.is_empty() || !item.topic.is_empty(), |d| d.child(info_tip(id, item, app)))
        .into_any_element()
}

// ------------------------------------------------------------------------------------------------ segmented choice

/// A segmented choice (stock `ToggleGroup`, outline variant): exactly one option is on; clicking another selects it.
///
/// Rendering (UI review B2): a visible container border in `border_strong`, and the selected segment filled with the
/// accent (white label, 700 weight), which is at least 3:1 against its neighbours and is told apart without colour by
/// the weight. Unselected segments keep the surface fill and a regular label.
#[derive(IntoElement)]
pub struct Segmented {
    id: ElementId,
    labels: Vec<SharedString>,
    selected: usize,
    on_select: std::rc::Rc<dyn Fn(usize, &mut gpui_kit::Window, &mut GpuiApp)>,
}

pub fn segmented<I: Into<ElementId>, L: Into<SharedString> + Clone, F: Fn(usize, &mut gpui_kit::Window, &mut GpuiApp) + 'static>(
    id: I,
    labels: &[L],
    selected: usize,
    on_select: F,
) -> Segmented {
    Segmented {
        id: id.into(),
        labels: labels.iter().cloned().map(Into::into).collect(),
        selected,
        on_select: std::rc::Rc::new(on_select),
    }
}

impl RenderOnce for Segmented {
    fn render(self, _window: &mut gpui_kit::Window, cx: &mut GpuiApp) -> impl IntoElement {
        use gpui_kit::component::button::{Toggle, ToggleGroup, ToggleVariants as _};
        let p = cx.pal();
        let (on_bg, on_fg) = (cx.theme().primary, cx.theme().primary_foreground);
        let selected = self.selected;
        let on_select = self.on_select;
        ToggleGroup::new(self.id)
            .segmented()
            .outline()
            .children(self.labels.into_iter().enumerate().map(|(i, l)| {
                let t = Toggle::new(i).label(l).checked(i == selected).border_color(p.border_strong);
                if i == selected { t.bg(on_bg).text_color(on_fg).font_weight(FontWeight::BOLD) } else { t.text_color(p.fg) }
            }))
            .on_click(move |checks: &Vec<bool>, window, cx| {
                if let Some(ix) = checks.iter().enumerate().find(|(i, on)| **on && *i != selected).map(|(i, _)| i) {
                    on_select(ix, window, cx);
                }
            })
    }
}

// ------------------------------------------------------------------------------------------------ forms

/// A stock form [`Field`](gpui_kit::component::form::Field) whose label carries the info icon of `item`.
pub fn field(
    id: impl Into<ElementId>,
    item: &'static Item,
    app: &Entity<App>,
    control: impl IntoElement,
) -> gpui_kit::component::form::Field {
    let (id, app): (ElementId, Entity<App>) = (id.into(), app.clone());
    gpui_kit::component::form::Field::new().label_fn(move |_, _| labelled(id.clone(), item, &app)).child(control)
}

/// A field without an info icon.
pub fn field_plain(label: &'static str, control: impl IntoElement) -> gpui_kit::component::form::Field {
    gpui_kit::component::form::Field::new().label(label).child(control)
}

/// The identity line used on Connect and Confirm: `{model · address · SN serial}` plus the compatibility tag.
pub fn identity_line(cx: &GpuiApp, p: &Palette, model: &str, host: &str, serial: &str, status: Option<Status>) -> AnyElement {
    let mut s = format!("{model} \u{00b7} {host}");
    if !serial.is_empty() {
        s.push_str(&format!(" \u{00b7} SN {serial}"));
    }
    h_flex()
        .gap(px(8.0))
        .items_center()
        .flex_wrap()
        .child(mono(cx, s))
        .when_some(status, |d, st| d.child(compat_tag(p, st)))
        .into_any_element()
}

// ------------------------------------------------------------------------------------------------ disabled with a reason

/// The disabled-reason pattern (DEV-UI-2): a REAL disabled `Button` (not a Tab stop, no click) followed by a sibling
/// info button whose `Popover` carries the reason. A tooltip on a disabled control would be unreachable by keyboard and
/// must never be the only carrier of the reason. When `reason` is `None` the button is returned alone, enabled.
pub fn with_reason(id: impl Into<ElementId>, button: gpui_kit::component::button::Button, reason: Option<String>) -> AnyElement {
    use gpui_kit::component::button::ButtonVariants as _;
    use gpui_kit::component::popover::Popover;
    use gpui_kit::component::{Disableable, Sizable};
    let id: ElementId = id.into();
    match reason {
        None => button.into_any_element(),
        Some(text) => {
            let label: SharedString = format!("Why unavailable: {text}").into();
            let body = text.clone();
            h_flex()
                .gap(px(4.0))
                .items_center()
                .child(button.disabled(true))
                .child(
                    Popover::new(id)
                        .anchor(gpui_kit::Anchor::BottomRight)
                        .trigger(
                            gpui_kit::component::button::Button::new("why")
                                .ghost()
                                .xsmall()
                                .icon(icons::icon(role::TIP))
                                .accessibility_label(label),
                        )
                        .content(move |_, _, _| div().w(px(280.0)).text_sm().child(body.clone())),
                )
                .into_any_element()
        }
    }
}
