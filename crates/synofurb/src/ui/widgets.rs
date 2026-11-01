//! Small building blocks shared by every screen: text styles, icons, badges, banners, cards, form rows.
//! All colours and sizes come from [`super::theme`]; nothing in here (or in the screens) hard-codes a colour.

use super::theme::{self, Palette, Tone, TypeStyle, radius, space};
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, FontWeight, Hsla, SharedString, Stateful, Window, div, px};
use synofurb_core::compat::Status;

// ------------------------------------------------------------------------------------------------ text

/// Text in one of the design's type styles, in the primary text colour.
pub fn text(p: &Palette, ts: TypeStyle, s: impl Into<SharedString>) -> Div {
    div().text_size(ts.size()).line_height(ts.line()).font_weight(FontWeight(ts.weight)).text_color(p.fg).child(s.into())
}

pub fn h1(p: &Palette, s: impl Into<SharedString>) -> Div {
    text(p, theme::H1, s)
}

pub fn h2(p: &Palette, s: impl Into<SharedString>) -> Div {
    text(p, theme::H2, s)
}

/// Body text (14 px).
pub fn body(p: &Palette, s: impl Into<SharedString>) -> Div {
    text(p, theme::BODY, s)
}

/// Secondary, smaller text (13 px, `fg_2`).
pub fn muted(p: &Palette, s: impl Into<SharedString>) -> Div {
    text(p, theme::BODY_SM, s).text_color(p.fg_2)
}

/// Caption (12 px, semibold): labels, table headers.
pub fn caption(p: &Palette, s: impl Into<SharedString>) -> Div {
    text(p, theme::CAPTION, s).text_color(p.fg_2)
}

/// Monospace text for addresses, device paths, fingerprints and commands.
pub fn mono(p: &Palette, s: impl Into<SharedString>, cx: &App) -> Div {
    use gpui_kit::component::ActiveTheme;
    text(p, theme::MONO, s).font_family(cx.theme().mono_font_family.clone()).font_weight(FontWeight(400.0))
}

// ------------------------------------------------------------------------------------------------ icons

/// A bundled icon by file stem (`icon("shield-check")`), 16 px, current text colour.
pub fn icon(name: &str) -> Icon {
    Icon::empty().path(format!("icons/{name}.svg")).size(px(16.0))
}

pub fn icon_sized(name: &str, size: f32, color: Hsla) -> Icon {
    Icon::empty().path(format!("icons/{name}.svg")).size(px(size)).text_color(color)
}

/// Icon that goes with a status tone (status is always icon + word + colour).
pub fn tone_icon(tone: Tone) -> &'static str {
    match tone {
        Tone::Ok => "circle-check",
        Tone::Warn => "triangle-alert",
        Tone::Err => "circle-x",
        Tone::Info => "info",
        Tone::Accent => "sparkles",
        Tone::Neutral => "circle-dot",
    }
}

// ------------------------------------------------------------------------------------------------ badges and chips

/// Tinted badge: icon + word, 1 px border at the tone's solid colour, optionally dashed (Untested).
pub fn badge(p: &Palette, tone: Tone, icon_name: &str, label: impl Into<SharedString>, dashed: bool) -> Div {
    let c = p.tone(tone);
    let b = h_flex()
        .h(px(22.0))
        .flex_none()
        .items_center()
        .gap(px(space::S4))
        .px(px(space::S6))
        .rounded(px(radius::SM))
        .border_1()
        .border_color(c.border)
        .bg(c.bg)
        .text_color(c.fg)
        .text_size(theme::CAPTION.size())
        .font_weight(FontWeight(600.0))
        .child(icon_sized(icon_name, 14.0, c.fg))
        .child(label.into());
    if dashed { b.border_dashed() } else { b }
}

/// Tone and icon for a compatibility status (exact mapping of the design system).
pub fn status_style(s: Status) -> (Tone, &'static str, bool) {
    match s {
        Status::Verified => (Tone::Ok, "shield-check", false),
        Status::Community => (Tone::Info, "users", false),
        Status::Untested => (Tone::Neutral, "circle-question-mark", true),
        Status::KnownIssues => (Tone::Warn, "triangle-alert", false),
        Status::Unsupported => (Tone::Err, "ban", false),
    }
}

/// The compatibility badge: icon + the fixed status term, never colour alone.
pub fn compat_badge(p: &Palette, s: Status) -> Div {
    let (tone, ic, dashed) = status_style(s);
    badge(p, tone, ic, s.label(), dashed)
}

/// Vendor chip: neutral outline, bold vendor name + mono architecture tag. No logos, no brand colours.
pub fn vendor_chip(p: &Palette, vendor: &str, arch_tag: &str, cx: &App) -> Div {
    h_flex()
        .h(px(22.0))
        .flex_none()
        .items_center()
        .gap(px(space::S6))
        .px(px(space::S8))
        .rounded(px(radius::SM))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.surface)
        .child(text(p, theme::CAPTION, vendor.to_string()))
        .child(mono(p, arch_tag.to_string(), cx).text_size(theme::CAPTION.size()).text_color(p.fg_2))
}

/// Neutral pill for small facts ("4 disks").
pub fn chip(p: &Palette, s: impl Into<SharedString>) -> Div {
    h_flex()
        .h(px(22.0))
        .flex_none()
        .items_center()
        .px(px(space::S8))
        .rounded(px(radius::SM))
        .bg(p.surface_2)
        .border_1()
        .border_color(p.border)
        .text_size(theme::CAPTION.size())
        .font_weight(FontWeight(600.0))
        .text_color(p.fg_2)
        .child(s.into())
}

// ------------------------------------------------------------------------------------------------ banners and cards

/// Banner: tinted fill, 1 px border in the tone's colour, icon + bold title + one sentence, actions on the right.
pub fn banner(p: &Palette, tone: Tone, title: impl Into<SharedString>, detail: impl Into<SharedString>, actions: Vec<AnyElement>) -> Div {
    let c = p.tone(tone);
    h_flex()
        .w_full()
        .items_start()
        .gap(px(space::S12))
        .p(px(space::S12))
        .rounded(px(radius::MD))
        .border_1()
        .border_color(c.border)
        .bg(c.bg)
        .child(icon_sized(tone_icon(tone), 18.0, c.fg).mt(px(1.0)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(space::S2))
                .child(text(p, theme::BODY, title.into()).font_weight(FontWeight(700.0)).text_color(c.fg))
                .child(text(p, theme::BODY_SM, detail.into()).text_color(c.fg)),
        )
        .when(!actions.is_empty(), |d| d.child(h_flex().flex_none().gap(px(space::S8)).children(actions)))
}

/// Card: surface, 1 px border, radius 12; optional header (title, right slot) and a coloured 4 px top edge.
pub fn card(p: &Palette, title: Option<SharedString>, right: Option<AnyElement>, top_edge: Option<Hsla>, body: impl IntoElement) -> Div {
    v_flex()
        .w_full()
        .rounded(px(radius::LG))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .overflow_hidden()
        .when_some(top_edge, |d, c| d.child(div().h(px(4.0)).w_full().bg(c)))
        .when_some(title, |d, t| {
            d.child(
                h_flex()
                    .px(px(space::S16))
                    .py(px(space::S12))
                    .gap(px(space::S8))
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(p.border)
                    .child(text(p, theme::BODY, t).font_weight(FontWeight(600.0)))
                    .when_some(right, |d, r| d.child(r)),
            )
        })
        .child(div().p(px(space::S16)).child(body))
}

/// Section label inside a page.
pub fn section(p: &Palette, s: impl Into<SharedString>) -> Div {
    h2(p, s).mt(px(space::S8))
}

/// A labelled form field: label above, control, helper text below.
pub fn field(p: &Palette, label: &str, helper: Option<&str>, control: impl IntoElement) -> Div {
    v_flex()
        .gap(px(space::S6))
        .w_full()
        .child(text(p, theme::BODY_SM, label.to_string()).font_weight(FontWeight(600.0)))
        .child(control)
        .when_some(helper, |d, h| d.child(muted(p, h.to_string()).text_size(theme::CAPTION.size()).font_weight(FontWeight(400.0))))
}

/// A form row: label column on the left, control and helper on the right.
pub fn form_row(p: &Palette, label: &str, helper: Option<SharedString>, control: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .items_start()
        .gap(px(space::S16))
        .child(
            div()
                .w(px(theme::layout::FORM_LABEL_W))
                .flex_none()
                .pt(px(8.0))
                .child(text(p, theme::BODY_SM, label.to_string()).font_weight(FontWeight(600.0))),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(space::S4))
                .child(control)
                .when_some(helper, |d, h| d.child(muted(p, h).text_size(theme::CAPTION.size()).font_weight(FontWeight(400.0)))),
        )
}

/// Key/value row for summaries (`Host name   nas`).
pub fn kv(p: &Palette, key: &str, value: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .items_start()
        .gap(px(space::S16))
        .py(px(space::S4))
        .child(div().w(px(theme::layout::FORM_LABEL_W)).flex_none().child(muted(p, key.to_string())))
        .child(div().flex_1().min_w_0().child(value))
}

/// Dark code/command box (always the log palette: terminal convention).
pub fn code_box(p: &Palette, lines: &[String], cx: &App) -> Div {
    v_flex().w_full().gap(px(space::S2)).p(px(space::S12)).rounded(px(radius::MD)).bg(p.log_bg).children(
        lines.iter().map(|l| mono(p, l.clone(), cx).text_size(theme::LOG.size()).line_height(theme::LOG.line()).text_color(p.log_fg)),
    )
}

pub fn divider(p: &Palette) -> Div {
    div().w_full().h(px(1.0)).bg(p.border)
}

/// Selectable card with a dot: a radio that hits as a whole card (mode, goal, boot method...).
pub fn choice_card(
    p: &Palette,
    id: impl Into<gpui_kit::ElementId>,
    selected: bool,
    disabled: bool,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    on_click: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let on_click = std::rc::Rc::new(on_click);
    let border = if selected { p.accent } else { p.border_strong };
    let mut d = h_flex()
        .id(id)
        .w_full()
        .items_start()
        .gap(px(space::S12))
        .p(px(space::S12))
        .rounded(px(radius::MD))
        .border_1()
        .border_color(border)
        .bg(if selected { p.accent_bg } else { p.surface })
        .when(selected, |d| d.border_2())
        .child(
            div()
                .flex_none()
                .mt(px(2.0))
                .size(px(18.0))
                .rounded_full()
                .border_2()
                .border_color(if selected { p.accent } else { p.border_strong })
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(8.0)).rounded_full().when(selected, |d| d.bg(p.accent))),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(space::S2))
                .child(text(p, theme::BODY, title.into()).font_weight(FontWeight(600.0)).when(disabled, |d| d.text_color(p.placeholder)))
                .child(muted(p, detail.into()).when(disabled, |d| d.text_color(p.placeholder))),
        );
    if disabled {
        d = d.opacity(0.75);
    } else {
        let hover_bg = if selected { p.accent_bg } else { p.surface_2 };
        let (ring, pressed) = (p.ring, p.accent_bg);
        let (h1, h2) = (on_click.clone(), on_click);
        d = d
            .cursor_pointer()
            .tab_index(0)
            .hover(move |s| s.bg(hover_bg))
            .active(move |s| s.bg(pressed))
            .focus_visible(move |s| s.border_color(ring).border_2())
            .on_click(move |_, w, cx| h1(&(), w, cx))
            // Space selects, like a radio button; Enter stays the screen's primary action.
            .on_key_down(move |ev, w, cx| {
                if ev.keystroke.key == "space" {
                    h2(&(), w, cx);
                    cx.stop_propagation();
                }
            });
    }
    d
}

// ------------------------------------------------------------------------------------------------ buttons

/// Secondary button (outline, surface fill). Primary/danger use the Kit's variants directly.
pub fn btn_secondary(id: impl Into<gpui_kit::ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).label(label).outline()
}

pub fn btn_ghost(id: impl Into<gpui_kit::ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).label(label).ghost()
}

pub fn btn_primary(id: impl Into<gpui_kit::ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).label(label).primary()
}

// ------------------------------------------------------------------------------------------------ tables

/// Column of [`data_table`]: header text, fixed width in px (0 = share the rest), right-aligned numbers.
pub struct Col {
    pub title: &'static str,
    pub w: f32,
    pub right: bool,
}

impl Col {
    pub fn new(title: &'static str, w: f32) -> Col {
        Col { title, w, right: false }
    }
    pub fn right(mut self) -> Col {
        self.right = true;
        self
    }
}

fn cell(c: &Col, child: AnyElement) -> Div {
    let d = div().px(px(14.0)).py(px(10.0)).min_w_0().child(child);
    let d = if c.w > 0.0 { d.w(px(c.w)).flex_none() } else { d.flex_1() };
    if c.right { d.flex().justify_end() } else { d }
}

/// A simple read-only table: `surface_2` header, hairline row dividers, optional 3 px left edge per row (attention).
pub fn data_table(p: &Palette, cols: &[Col], rows: Vec<(Option<Hsla>, Vec<AnyElement>)>) -> Div {
    let header = h_flex()
        .w_full()
        .bg(p.surface_2)
        .border_b_1()
        .border_color(p.border)
        .children(cols.iter().map(|c| cell(c, text(p, theme::CAPTION, c.title).text_color(p.fg_2).into_any_element()).py(px(8.0))));
    let n = rows.len();
    v_flex().w_full().rounded(px(radius::LG)).border_1().border_color(p.border).bg(p.surface).overflow_hidden().child(header).children(
        rows.into_iter().enumerate().map(|(i, (edge, cells))| {
            h_flex()
                .w_full()
                .items_center()
                .when(i + 1 < n, |d| d.border_b_1().border_color(p.border))
                .child(div().w(px(3.0)).self_stretch().flex_none().when_some(edge, |d, c| d.bg(c)))
                .children(cells.into_iter().zip(cols.iter()).map(|(c, col)| cell(col, c)))
        }),
    )
}

/// Plain body-small text as an element for table cells.
pub fn cell_text(p: &Palette, s: impl Into<SharedString>) -> AnyElement {
    text(p, theme::BODY_SM, s).into_any_element()
}
