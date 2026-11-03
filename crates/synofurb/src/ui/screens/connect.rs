//! D2 Connect: sign in over SSH (read-only). Fields, one ToggleGroup, one Alert for errors.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::Themed;
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
        let pin = self.host_pin(cx);
        let p = cx.pal();

        let mut form = Form::vertical()
            .child(kit::field_plain(copy::CONN_PORT.short, div().w(px(120.0)).child(Input::new(&self.inp.port))))
            .child(kit::field("f-user", &copy::CONN_USER, &app, Input::new(&self.inp.user)))
            .child(kit::field_plain(copy::CONN_SIGNIN.short, signin))
            .child(kit::field_plain(if self.use_key { copy::CONN_KEY_FILE.short } else { copy::CONN_PASSWORD.short }, secret));
        if let Some(fp) = pin {
            form = form.child(kit::field(
                "f-hostkey",
                &copy::HK_PIN,
                &app,
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(kit::tag(&p, crate::ui::theme::Tone::Ok, Some(role::LOCK), copy::HK_PINNED_TAG.short))
                    .child(kit::mono_sm(cx, fp).min_w_0().truncate()),
            ));
        }
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
        // An address the user added that answered but was not recognised: said again here, before anything is sent to it.
        let unknown = self
            .selected_ip
            .as_deref()
            .and_then(|k| self.added.get(k))
            .is_some_and(|h| h.outcome() == crate::ui::addhost::Outcome::Unknown);
        if unknown {
            body = body.child(Alert::warning("unknown-device", copy::INV_UNKNOWN_WARN.short).icon(icons::icon(role::WARN)));
        }
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
            .right(vec![kit::with_reason(
                "inspect-why",
                Button::new("inspect")
                    .primary()
                    .label(copy::INV_CONNECT.short)
                    .loading(inspecting)
                    .disabled(inspecting)
                    .on_click(cx.listener(|this, _, window, cx| this.start_inspect(window, cx))),
                host_empty.then(|| copy::CONN_HOST_WHY.tip.to_string()),
            )])
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
        if let Some(f) = self.added.get(&ip).and_then(|h| h.report.found.as_ref()) {
            return (if f.model.is_empty() { "Synology".into() } else { f.model.clone() }, f.serial.clone(), self.compat_for_found(f).0);
        }
        match self.inv.devices.iter().find(|d| d.primary_ip().map(|i| i.to_string()) == Some(ip.clone())) {
            Some(d) => (if d.model.is_empty() { "Synology".into() } else { d.model.clone() }, d.serial.clone(), self.compat_for_found(d).0),
            None => ("NAS".into(), String::new(), synofurb_core::compat::Status::Untested),
        }
    }
}
