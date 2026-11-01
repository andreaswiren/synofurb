//! Step 1: find the NAS. Live scan results with a compatibility badge per unit; converted units are listed apart.

use crate::ui::app::App;
use crate::ui::format::plural;
use crate::ui::render::Page;
use crate::ui::theme::{self, Themed, Tone, radius, space};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, IntoElement, Window, div, px};
use synofurb_core::discovery::Found;

impl App {
    pub(crate) fn page_inventory(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let scanning = self.scan.running;

        let toolbar = h_flex()
            .w_full()
            .gap(px(space::S12))
            .items_center()
            .flex_wrap()
            .child(
                Button::new("scan")
                    .primary()
                    .icon(icon("radar"))
                    .label(if scanning { "Scanning..." } else { "Scan network" })
                    .loading(scanning)
                    .disabled(scanning)
                    .on_click(cx.listener(|this, _, _, cx| this.start_scan(cx))),
            )
            .child(muted(&p, "or enter an address:"))
            .child(div().w(px(200.0)).child(Input::new(&self.inp.manual_ip)))
            .child(
                btn_secondary("use-address", "Use this address")
                    .disabled(self.input_value(&self.inp.manual_ip, cx).trim().is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.use_manual_address(window, cx))),
            );

        // Per-method status while and after scanning: a failing method is a warning chip, not a page error.
        let methods = h_flex().gap(px(space::S8)).flex_wrap().items_center().children(self.scan.methods.iter().map(|(name, r)| match r {
            Ok(n) => badge(&p, Tone::Ok, "check", format!("{name}: {}", plural(*n, "result", "results")), false),
            Err(e) => badge(&p, Tone::Warn, "triangle-alert", format!("{name}: {}", e.chars().take(60).collect::<String>()), false),
        }));
        let status = h_flex()
            .gap(px(space::S12))
            .items_center()
            .flex_wrap()
            .when(scanning, |d| d.child(Spinner::new().small().color(p.accent)).child(muted(&p, "Looking for units on this network...")))
            .child(methods);

        let units: Vec<Found> = self.inv.devices.iter().filter(|d| d.synology && !d.synofurb_debian).cloned().collect();
        let converted: Vec<Found> = self.inv.devices.iter().filter(|d| d.synofurb_debian).cloned().collect();

        let mut list = v_flex().w_full().gap(px(space::S8));
        if units.is_empty() && !scanning {
            list = list.child(self.empty_state(&p, cx));
        }
        for (i, d) in units.iter().enumerate() {
            list = list.child(self.unit_row(&p, i, d, cx));
        }

        let mut body = v_flex().w_full().gap(px(space::S16)).child(toolbar).child(status);
        if !units.is_empty() || scanning {
            body = body
                .child(section(&p, format!("Synology units ({})", units.len())))
                .child(list.when(units.is_empty(), |d| d.child(muted(&p, "Nothing found yet."))));
        } else {
            body = body.child(list);
        }
        if !converted.is_empty() {
            body = body.child(section(&p, "Already running Debian")).child(v_flex().w_full().gap(px(space::S8)).children(
                converted.iter().enumerate().map(|(i, d)| {
                    let ip = d.primary_ip().map(|i| i.to_string()).unwrap_or_default();
                    let url = format!("https://{ip}:8443");
                    h_flex()
                        .w_full()
                        .gap(px(space::S12))
                        .items_center()
                        .p(px(space::S12))
                        .rounded(px(radius::MD))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.surface)
                        .child(icon_sized("circle-check", 18.0, p.ok_fg))
                        .child(
                            v_flex()
                                .flex_1()
                                .child(
                                    text(
                                        &p,
                                        theme::BODY,
                                        if d.hostname.is_empty() { d.title() } else { format!("{} ({})", d.hostname, ip) },
                                    )
                                    .font_weight(FontWeight(600.0)),
                                )
                                .child(muted(
                                    &p,
                                    format!(
                                        "{}  - Debian installed by synofurb",
                                        if d.model.is_empty() { "Synology hardware" } else { &d.model }
                                    ),
                                )),
                        )
                        .child(
                            btn_secondary(("manage", i), "Manage")
                                .small()
                                .icon(icon("external-link"))
                                .tooltip("Opens the unit's management page in your browser")
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                }),
            ));
        }

        let selected = self.selected_ip.clone();
        Page::new("Find your Synology NAS", body)
            .subtitle("Scans the way Synology's own tools do: Synology Assistant broadcast, Bonjour/mDNS, UPnP/SSDP, the DSM web API on port 5000 and Synology MAC prefixes. Nothing is changed on any unit.")
            .note(muted(&p, if selected.is_some() { "A unit is selected." } else { "Select a unit to continue." }))
            .buttons(vec![
                btn_primary("next-connect", "Connect")
                    .icon(icon("arrow-right"))
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(ip) = this.selected_ip.clone() {
                            this.select_host(&ip, window, cx);
                        }
                    }))
                    .into_any_element(),
            ])
    }

    fn empty_state(&self, p: &theme::Palette, cx: &mut Context<App>) -> impl IntoElement {
        card(
            p,
            None,
            None,
            None,
            v_flex()
                .gap(px(space::S8))
                .child(h2(p, "No Synology found on this network"))
                .child(muted(p, "Likely causes:"))
                .child(muted(p, "- The unit is on another network or VLAN than this computer."))
                .child(muted(p, "- The unit is powered off or still starting (give it five minutes)."))
                .child(muted(p, "- A firewall blocks multicast and UDP 9999; the subnet sweep may still find it."))
                .child(
                    h_flex()
                        .gap(px(space::S8))
                        .pt(px(space::S4))
                        .child(btn_secondary("scan-again", "Scan again").on_click(cx.listener(|this, _, _, cx| this.start_scan(cx)))),
                ),
        )
    }

    fn unit_row(&self, p: &theme::Palette, i: usize, d: &Found, cx: &mut Context<App>) -> impl IntoElement {
        let ip = d.primary_ip().map(|i| i.to_string()).unwrap_or_default();
        let (status, reason) = self.compat_for_found(d);
        let selected = self.selected_ip.as_deref() == Some(ip.as_str());
        let name = if d.model.is_empty() { "Synology".to_string() } else { d.model.clone() };
        let meta = format!(
            "MAC {}   DSM {}   serial {}   via {}",
            d.mac.clone().unwrap_or_else(|| "?".into()),
            if d.dsm_version.is_empty() { "?" } else { &d.dsm_version },
            if d.serial.is_empty() { "?" } else { &d.serial },
            d.sources.join(", ")
        );
        let ipc = ip.clone();
        let ipc2 = ip.clone();
        h_flex()
            .id(("unit", i))
            .w_full()
            .items_stretch()
            .rounded(px(radius::MD))
            .border_1()
            .border_color(if selected { p.accent } else { p.border })
            .bg(if selected { p.accent_bg } else { p.surface })
            .overflow_hidden()
            .cursor_pointer()
            .hover({
                let hb = if selected { p.accent_bg } else { p.surface_2 };
                move |s| s.bg(hb)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_ip = Some(ipc.clone());
                cx.notify();
            }))
            .child(div().w(px(3.0)).flex_none().bg(if selected { p.accent } else { p.border }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .p(px(space::S12))
                    .gap(px(space::S12))
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(space::S4))
                            .child(
                                h_flex()
                                    .gap(px(space::S8))
                                    .items_center()
                                    .flex_wrap()
                                    .child(text(p, theme::H2, name))
                                    .child(mono(p, ip.clone(), cx).text_color(p.fg_2))
                                    .child(compat_badge(p, status))
                                    .when(selected, |d| d.child(chip(p, "selected"))),
                            )
                            .when_some(reason.filter(|_| status != synofurb_core::compat::Status::Verified), |d, r| {
                                d.child(muted(p, r).text_size(theme::BODY_SM.size()))
                            })
                            .child(muted(p, meta).text_size(theme::CAPTION.size())),
                    )
                    .child(
                        btn_secondary(("select", i), "Select")
                            .small()
                            .on_click(cx.listener(move |this, _, window, cx| this.select_host(&ipc2, window, cx))),
                    ),
            )
    }

    /// Used by the goal screen and others to name the selected unit.
    pub(crate) fn unit_name(&self) -> String {
        self.facts
            .as_ref()
            .map(|f| if f.model.is_empty() { f.hostname.clone() } else { f.model.clone() })
            .unwrap_or_else(|| "the NAS".into())
    }
}
