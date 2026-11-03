//! Setup > Add-ons (docs/ADDONS.md, "Installer integration"): the list of Docker apps the installer can set up after the first
//! boot. A row is a checkbox, a one-line summary and a few tags (needs Docker, needs Internet, privileges); the settings that
//! matter at install time appear under a ticked row. Docker is on and locked, with the reason in plain sight, while something
//! ticked needs it. Everything else (cameras, media folders, accounts) is done later in the web interface.

use crate::ui::app::App;
use crate::ui::copy::{self, Item};
use crate::ui::disclosure::info_tip;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::form::Form;
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme, Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div, px};
use synofurb_core::addons;

/// The label and explanation of an install-time setting.
pub fn setting_item(id: &str, key: &str) -> Option<&'static Item> {
    match (id, key) {
        ("frigate", "retain_days") => Some(&copy::ADDON_RETAIN),
        ("plex", "plex_claim") => Some(&copy::ADDON_PLEX_CLAIM),
        _ => None,
    }
}

/// The tag text and explanation of a privilege.
pub fn privilege_item(privilege: &str) -> &'static Item {
    match privilege {
        "docker-socket" => &copy::ADDON_PRIV_SOCKET,
        "host-network" => &copy::ADDON_PRIV_HOSTNET,
        "privileged" => &copy::ADDON_PRIV_FULL,
        _ => &copy::ADDON_PRIV_OTHER,
    }
}

impl App {
    pub(crate) fn page_addons(&mut self, window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        if self.facts.is_none() {
            return Page::new(copy::STEP_SETUP.short, kit::muted(cx, "-")).left(vec![
                Button::new("back")
                    .ghost()
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Plan, window, cx)))
                    .into_any_element(),
            ]);
        }
        self.ensure_addon_inputs(window, cx);
        let (picks, secrets) = self.addon_state(cx);
        let errors = picks.field_errors(&secrets);
        let blocker = self.addons_blocker(cx);

        let mut list = v_flex().gap(px(8.0));
        for (ix, e) in addons::catalog().iter().enumerate() {
            let id = e.id;
            let on = picks.is_on(id);
            let required = picks.required_by(id);
            let locked = !required.is_empty();
            let box_ = Checkbox::new(("addon", ix)).label(e.name).checked(on).disabled(locked).on_click(cx.listener(
                move |this, v: &bool, _, cx| {
                    this.addons.picks.set(id, *v);
                    cx.notify();
                },
            ));
            let head = h_flex()
                .gap(px(8.0))
                .items_center()
                .flex_wrap()
                .child(box_)
                .when(locked, |d| d.child(kit::tag(&p, Tone::Info, Some(role::LINK), copy::ADDON_REQUIRED.fmt(&[&required.join(", ")]))))
                .when(!e.requires.is_empty(), |d| d.child(kit::chip(&p, copy::ADDON_NEEDS_DOCKER.short)))
                .when(e.needs_internet, |d| {
                    d.child(
                        h_flex()
                            .gap(px(2.0))
                            .items_center()
                            .child(kit::tag(&p, Tone::Neutral, Some(role::NETWORK), copy::ADDON_NEEDS_NET.short))
                            .child(info_tip(("net-info", ix), &copy::ADDON_NEEDS_NET, &app)),
                    )
                })
                .children(e.privileges.iter().enumerate().map(|(j, pr)| {
                    let item = privilege_item(pr);
                    h_flex().gap(px(2.0)).items_center().child(kit::tag(&p, Tone::Warn, Some(role::WARN), item.short)).child(info_tip(
                        ("priv-info", ix * 8 + j),
                        item,
                        &app,
                    ))
                }));
            let mut row = v_flex()
                .gap(px(6.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(if on { p.border_strong } else { cx.theme().border })
                .child(head)
                .child(div().pl(px(28.0)).child(kit::caption(cx, e.summary)));
            if picks.is_picked(id) && !e.settings.is_empty() {
                let mut form = Form::vertical();
                for (j, s) in e.settings.iter().enumerate() {
                    let (Some(item), Some(st)) = (setting_item(id, s.key), self.addons.inputs.get(&(id, s.key))) else { continue };
                    let err = errors.iter().find(|(k, _)| *k == (id, s.key)).map(|(_, m)| m.clone());
                    let mut control = v_flex().gap(px(4.0)).child(div().w(px(320.0)).child({
                        let i = Input::new(st).aria_label(item.short);
                        if s.is_secret() { i.mask_toggle() } else { i }
                    }));
                    if let Some(m) = err {
                        control = control.child(div().text_xs().text_color(p.err_fg).child(m));
                    }
                    form = form.child(kit::field(("f-addon", ix * 8 + j), item, &app, control));
                }
                row = row.child(div().pl(px(28.0)).child(form));
            }
            list = list.child(row);
        }

        let mut body =
            v_flex().w_full().max_w(px(900.0)).gap(px(16.0)).child(self.setup_tabs(cx)).child(kit::muted(cx, copy::ADDONS_INTRO.short));
        if !picks.is_empty() && !self.sys.enable_mgmt {
            body = body.child(Alert::warning("addons-web", copy::ADDON_NEEDS_WEB.short).icon(icons::icon(role::WARN)));
        }
        if !picks.is_empty() && self.source_offline {
            body = body.child(Alert::warning("addons-offline", copy::CONF_ADDONS_OFFLINE.short).icon(icons::icon(role::WARN)));
        }
        body = body.child(list);

        Page::new(copy::STEP_SETUP.short, body)
            .subtitle(copy::ADDONS_SUB.short)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::System, window, cx)))
                    .into_any_element(),
                self.copy_command_button(cx),
            ])
            .right(vec![kit::with_reason(
                "next-why",
                Button::new("next")
                    .primary()
                    .icon(icons::icon(role::NEXT))
                    .label(copy::SYS_NEXT.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Confirm, window, cx))),
                blocker,
            )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::addons::setting;

    #[test]
    fn every_privilege_in_the_catalog_has_a_label() {
        for e in addons::catalog() {
            for p in e.privileges {
                let it = privilege_item(p);
                assert!(!it.short.is_empty() && !it.tip.is_empty(), "{p}");
                assert_ne!(it.key, "ADDON_PRIV_OTHER", "{} has an unknown privilege {p}", e.id);
            }
        }
    }

    #[test]
    fn setting_copy_matches_the_catalog_and_secrets_say_so() {
        for e in addons::catalog() {
            for s in e.settings {
                let it = setting_item(e.id, s.key).unwrap_or_else(|| panic!("{}.{}", e.id, s.key));
                assert_eq!(it.short, s.label, "{}.{}: the GUI label and the catalog label differ", e.id, s.key);
                assert!(setting(e.id, s.key).is_some());
            }
        }
        // the claim token's explanation sits behind the info button: its body says how long it is valid
        assert!(copy::ADDON_PLEX_CLAIM.body.contains("4 minutes"));
    }
}
