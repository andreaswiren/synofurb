//! The update line shown in Settings and on the About page: one component, so every place that mentions updates shows
//! the same state. Checking (spinner), up to date, update available, or "Couldn't check: <reason>" with the full reason
//! behind an (i) and a "Check again" button. A click on "Check for updates" always ends in one of these and stays.
//! Pure decisions live in `ui::updates`; this file only draws them.

use crate::ui::app::{App, UpdateState};
use crate::ui::copy;
use crate::ui::disclosure;
use crate::ui::icons::{self, role};
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App as GpuiApp, Entity, IntoElement, SharedString, div, px};

/// Is a (clickable) update waiting? Drives the title-bar dot and which button of the screen is primary.
pub fn update_pending(state: &UpdateState) -> bool {
    matches!(state, UpdateState::Available(_) | UpdateState::Downloading(..) | UpdateState::Verified(..) | UpdateState::Failed(..))
}

/// The update line. `id` keeps the controls of two screens apart.
pub fn status_row(app: &Entity<App>, id: &'static str, cx: &GpuiApp) -> AnyElement {
    let a = app.read(cx);
    let p = cx.pal();
    let muted = cx.theme().muted_foreground;
    let busy = matches!(a.update, UpdateState::Checking);
    let version = crate::ui::app::version();

    let (icon, text, info, note): (Option<AnyElement>, String, Option<AnyElement>, Option<String>) = match &a.update {
        UpdateState::Unknown => (None, copy::UPD_ST_NONE.short.into(), None, None),
        UpdateState::Checking => (Some(Spinner::new().small().into_any_element()), copy::UPD_ST_CHECKING.short.into(), None, None),
        UpdateState::UpToDate => (
            Some(icons::icon_sized(role::OK, 16.0, p.tone(Tone::Ok).fg).into_any_element()),
            copy::UPD_ST_CURRENT.fmt(&[version]),
            None,
            None,
        ),
        UpdateState::CheckFailed(f) => (
            Some(icons::icon_sized(role::WARN, 16.0, p.tone(Tone::Warn).fg).into_any_element()),
            copy::UPD_ST_FAILED.fmt(&[f.reason().short]),
            Some(disclosure::info_note(SharedString::from(format!("{id}-why")), f.reason().short, f.full()).into_any_element()),
            None,
        ),
        UpdateState::Available(i) | UpdateState::Downloading(i, _) | UpdateState::Verified(i, _) | UpdateState::Failed(i, _) => (
            Some(icons::icon_sized(role::INFO, 16.0, p.tone(Tone::Accent).fg).into_any_element()),
            copy::UPD_ST_AVAILABLE.fmt(&[&i.latest]),
            None,
            a.update_unsigned().then(|| copy::UPD_ST_UNSIGNED.short.to_string()),
        ),
    };

    let pending = update_pending(&a.update);
    let button: AnyElement = if pending {
        let app2 = app.clone();
        Button::new(SharedString::from(format!("{id}-details")))
            .primary()
            .small()
            .label(copy::UPD_DETAILS.short)
            .on_click(move |_, window, cx| app2.update(cx, |a, cx| a.open_update_dialog(window, cx)))
            .into_any_element()
    } else {
        let app2 = app.clone();
        let label = if matches!(a.update, UpdateState::CheckFailed(_)) { &copy::UPD_CHECK_AGAIN } else { &copy::UPD_CHECK };
        Button::new(SharedString::from(format!("{id}-check")))
            .outline()
            .small()
            .icon(icons::icon(role::REFRESH))
            .label(label.short)
            .loading(busy)
            .disabled(busy)
            .on_click(move |_, _, cx| app2.update(cx, |a, cx| a.check_updates_now(cx)))
            .into_any_element()
    };

    h_flex()
        .w_full()
        .gap(px(12.0))
        .items_center()
        .justify_between()
        .child(
            v_flex()
                .gap(px(2.0))
                .min_w_0()
                .child(h_flex().gap(px(6.0)).items_center().text_sm().children(icon).child(div().child(text)).children(info))
                .children(note.map(|n| div().text_sm().text_color(muted).child(n))),
        )
        .child(button)
        .into_any_element()
}
