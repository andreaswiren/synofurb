//! D2 Connect: sign in over SSH (read-only). Fields, one ToggleGroup, one Alert for errors.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::form::Form;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div, px};

impl App {
    pub(crate) fn page_connect(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let app = cx.entity();
        let inspecting = self.inspecting;

        let signin = kit::segmented("sign-in", &[copy::CONN_PASSWORD.short, copy::CONN_KEY.short], usize::from(self.use_key), {
            let app = app.clone();
            move |ix, _, cx| {
                app.update(cx, |a, cx| {
                    a.use_key = ix == 1;
                    cx.notify();
                })
            }
        });
        let secret: gpui_kit::AnyElement = if self.use_key {
            h_flex()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().child(Input::new(&self.inp.key_path).cleanable(true)))
                .child(Button::new("browse-key").outline().label(copy::CONN_BROWSE.short).icon(icons::icon(role::FOLDER)).on_click(
                    cx.listener(|this, _, window, cx| {
                        let t = this.inp.key_path.clone();
                        this.browse_file(t, window, cx)
                    }),
                ))
                .into_any_element()
        } else {
            self.password_input().into_any_element()
        };

        let host = crate::ui::format::display_host(&self.input_value(&self.inp.host, cx));

        let mut form = Form::vertical()
            .child(kit::field_plain(copy::CONN_PORT.short, div().w(px(120.0)).child(Input::new(&self.inp.port))))
            .child(kit::field("f-user", &copy::CONN_USER, &app, Input::new(&self.inp.user)))
            .child(kit::field_plain(copy::CONN_SIGNIN.short, signin))
            .child(kit::field_plain(if self.use_key { copy::CONN_KEY_FILE.short } else { copy::CONN_PASSWORD.short }, secret));
        form = form.child(kit::field_plain(
            "",
            v_flex()
                .gap(px(8.0))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(2.0))
                        .child(Checkbox::new("same-sudo").label(copy::CONN_SUDO.short).checked(self.same_sudo).on_click(cx.listener(
                            |this, v: &bool, _, cx| {
                                this.same_sudo = *v;
                                cx.notify();
                            },
                        )))
                        .child(crate::ui::disclosure::info_tip("sudo-info", &copy::CONN_SUDO, &app)),
                )
                .when(!self.same_sudo, |d| d.child(Input::new(&self.inp.sudo).mask_toggle())),
        ));

        let mut body = v_flex().w_full().max_w(px(560.0)).gap(px(16.0));
        if let Some(e) = &self.error {
            let off = e.to_lowercase().contains("refused") || e.to_lowercase().contains("timed out");
            let item = if off { &copy::CONN_SSH_OFF } else { &copy::CONN_LOGIN_FAILED };
            body = body.child(
                v_flex().gap(px(4.0)).child(Alert::error("connect-error", e.clone()).title(item.short).icon(icons::icon(role::ERR))).child(
                    h_flex().gap(px(2.0)).items_center().child(kit::caption(cx, item.tip)).child(
                        Button::new("conn-help")
                            .link()
                            .small()
                            .label(copy::BTN_LEARN_MORE.short)
                            .on_click(cx.listener(|this, _, window, cx| this.open_help_topic(Some("troubleshooting"), window, cx))),
                    ),
                ),
            );
        }
        body = body.child(form);
        let host_empty = host.is_empty();
        Page::new(copy::CONN_TITLE.short, body)
            .title_info(&copy::CONN_TITLE)
            .subtitle(copy::CONN_SUB.short)
            .subtitle_tip(&copy::CONN_SUB)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Inventory, window, cx)))
                    .into_any_element(),
            ])
            .right(vec![
                Button::new("inspect")
                    .primary()
                    .label(copy::INV_CONNECT.short)
                    .loading(inspecting)
                    .disabled(inspecting || host_empty)
                    .on_click(cx.listener(|this, _, window, cx| this.start_inspect(window, cx)))
                    .into_any_element(),
            ])
    }

    /// `(model, serial, status)` of the selected unit, from the discovery inventory or the inspection.
    pub(crate) fn selected_unit_model(&self) -> (String, String, synofurb_core::compat::Status) {
        if let Some(f) = &self.facts {
            return (
                if f.model.is_empty() { f.hostname.clone() } else { f.model.clone() },
                f.serial.clone(),
                self.compat_for_facts(f).status,
            );
        }
        let ip = self.selected_ip.clone().unwrap_or_default();
        match self.inv.devices.iter().find(|d| d.primary_ip().map(|i| i.to_string()) == Some(ip.clone())) {
            Some(d) => (if d.model.is_empty() { "Synology".into() } else { d.model.clone() }, d.serial.clone(), self.compat_for_found(d).0),
            None => ("NAS".into(), String::new(), synofurb_core::compat::Status::Untested),
        }
    }
}
