//! D15 Manage: the page of a converted unit (Overview, Logins, Recovery, History as a TabBar).

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::empty::{Empty, EmptyHeader, EmptyTitle};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, px};

impl App {
    pub(crate) fn page_manage(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let unit = self.manage_unit.clone();
        let title = unit.as_ref().map(|d| if d.model.is_empty() { d.title() } else { d.model.clone() }).unwrap_or_else(|| "NAS".into());
        let ip = unit.as_ref().and_then(|d| d.primary_ip()).map(|i| i.to_string()).unwrap_or_default();
        let url = format!("https://{ip}:8443");
        let label = |id: &'static str, it: &'static copy::Item| kit::labelled(id, it, &app);
        let tab = self.manage_tab;
        let content: gpui_kit::AnyElement = match tab {
            0 => DescriptionList::horizontal()
                .columns(1)
                .bordered(false)
                .child(DescriptionItem::new(label("m-addr", &copy::MAN_ADDRESS)).value(kit::mono(cx, ip.clone()).into_any_element()))
                .child(DescriptionItem::new(label("m-boot", &copy::MAN_DEFAULT_BOOT)).value("Debian"))
                .child(DescriptionItem::new(label("m-web", &copy::MAN_WEB_UI)).value(kit::mono(cx, url.clone()).into_any_element()))
                .into_any_element(),
            1 => v_flex()
                .gap(px(10.0))
                .child(
                    h_flex()
                        .gap(px(4.0))
                        .items_center()
                        .child(kit::labelled("m-ssh", &copy::MAN_SSH_CMD, &app))
                        .child(kit::mono(cx, format!("ssh <user>@{ip}")))
                        .child(Clipboard::new("m-copy").value(format!("ssh <user>@{ip}"))),
                )
                .child(
                    h_flex()
                        .gap(px(2.0))
                        .child(kit::tag(&p, Tone::Neutral, Some(role::LOCK), copy::MAN_PW_NOT_KEPT.short))
                        .child(crate::ui::disclosure::info_tip("m-pw", &copy::MAN_PW_NOT_KEPT, &app)),
                )
                .into_any_element(),
            2 => Button::new("m-rec")
                .outline()
                .label(copy::MAN_RECOVERY.short)
                .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Recover, window, cx)))
                .into_any_element(),
            _ => Empty::new().header(EmptyHeader::new().title(EmptyTitle::new().child(copy::MAN_NO_HISTORY.short))).into_any_element(),
        };
        let tabs = TabBar::new("manage-tabs")
            .underline()
            .selected_index(tab)
            .children(
                [copy::MAN_OVERVIEW.short, copy::MAN_LOGINS.short, copy::MAN_RECOVERY.short, copy::MAN_HISTORY.short]
                    .map(|l| Tab::new().label(l)),
            )
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.manage_tab = *ix;
                cx.notify();
            }));
        let body = v_flex().w_full().gap(px(16.0)).child(tabs).child(content);
        Page::new(title, body)
            .subtitle(copy::MAN_SUB.short)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(this.return_to, window, cx)))
                    .into_any_element(),
            ])
            .right(vec![
                Button::new("m-diag")
                    .outline()
                    .icon(icons::icon(role::COPY))
                    .label(copy::MAN_DIAG.short)
                    .tooltip(copy::MAN_DIAG.tip)
                    .on_click(cx.listener(|this, _, window, cx| this.copy_diagnostics(window, cx)))
                    .into_any_element(),
                Button::new("m-open")
                    .primary()
                    .icon(icons::icon(role::EXTERNAL))
                    .label(copy::MAN_OPEN_WEB.short)
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .into_any_element(),
            ])
    }
}
