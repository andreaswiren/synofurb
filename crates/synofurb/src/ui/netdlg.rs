//! "Networks to scan": one checkbox row per network of this computer (adapter, CIDR, kind, "recommended"), custom networks the
//! person typed, Select recommended / Select all. Every change is kept at once in `ui-settings.json` (see `netsel.rs`).

use super::app::App;
use super::copy;
use super::icons::{self, role};
use super::kit;
use super::theme::{Themed, Tone, layout};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, Context, Entity, IntoElement, Subscription, WeakEntity, Window, div, px};
use synofurb_core::discovery::networks::NetworkKind;

pub struct NetView {
    app: WeakEntity<App>,
    input: Entity<InputState>,
    error: Option<String>,
    _sub: Subscription,
}

/// The tone of a kind Tag: a LAN is fine, a VPN is information, the rest is neutral or a warning.
pub fn kind_tone(k: NetworkKind) -> Tone {
    match k {
        NetworkKind::Lan => Tone::Ok,
        NetworkKind::Vpn => Tone::Info,
        NetworkKind::LinkLocal => Tone::Warn,
        NetworkKind::Virtual | NetworkKind::Other => Tone::Neutral,
    }
}

impl NetView {
    fn new(app: WeakEntity<App>, input: Entity<InputState>, window: &mut Window, cx: &mut Context<NetView>) -> NetView {
        let sub = cx.subscribe_in(&input, window, |this: &mut NetView, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } => this.add(window, cx),
            InputEvent::Change if this.error.take().is_some() => cx.notify(),
            _ => {}
        });
        NetView { app, input, error: None, _sub: sub }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<NetView>) {
        let text = self.input.read(cx).value().to_string();
        let Some(app) = self.app.upgrade() else { return };
        let r = app.update(cx, |a, cx| {
            let r = a.settings.scan_networks.add_custom(&text);
            if r.is_ok() {
                a.save_settings();
            }
            cx.notify();
            r
        });
        match r {
            Ok(_) => {
                self.error = None;
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
            }
            Err(e) => self.error = Some(e),
        }
        cx.notify();
    }

    fn with_app(&self, cx: &mut Context<NetView>, f: impl FnOnce(&mut App, &mut Context<App>)) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |a, cx| {
                f(a, cx);
                a.save_settings();
                cx.notify();
            });
        }
    }
}

impl gpui_kit::Render for NetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.pal();
        let muted = cx.theme().muted_foreground;
        let Some(app) = self.app.upgrade() else { return div().into_any_element() };
        let (nets, sel) = {
            let a = app.read(cx);
            (a.nets.clone(), a.settings.scan_networks.clone())
        };
        let eff = sel.effective(&nets);
        let mut col = v_flex().gap(px(8.0)).child(div().text_sm().child(copy::NET_TITLE.body));
        let mut list = v_flex().gap(px(2.0));
        for (i, n) in nets.iter().enumerate() {
            let on = eff.ids.iter().any(|x| x == &n.id);
            let id = n.id.clone();
            list = list.child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .py(px(4.0))
                    .child(div().flex_1().min_w_0().child(Checkbox::new(("net", i)).label(n.interface.clone()).checked(on).on_click(
                        cx.listener(move |this, _: &bool, _, cx| {
                            let id = id.clone();
                            this.with_app(cx, |a, _| {
                                let nets = a.nets.clone();
                                a.settings.scan_networks.toggle(&nets, &id);
                            });
                            cx.notify();
                        }),
                    )))
                    .child(div().w(px(128.0)).flex_none().child(kit::mono_sm(cx, n.cidr.clone())))
                    .child(div().w(px(76.0)).flex_none().child(h_flex().child(kit::tag(&p, kind_tone(n.kind), None, n.kind.label()))))
                    .child(
                        div()
                            .w(px(112.0))
                            .flex_none()
                            .when(n.recommended, |d| d.child(h_flex().child(kit::chip(&p, copy::NET_RECOMMENDED.short)))),
                    ),
            );
        }
        for (i, c) in sel.custom.iter().enumerate() {
            let cidr = c.cidr.clone();
            let cidr2 = c.cidr.clone();
            list = list.child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .py(px(4.0))
                    .child(div().flex_1().min_w_0().child(Checkbox::new(("netc", i)).label(copy::NET_CUSTOM.short).checked(c.on).on_click(
                        cx.listener(move |this, _: &bool, _, cx| {
                            let cidr = cidr.clone();
                            this.with_app(cx, |a, _| a.settings.scan_networks.toggle_custom(&cidr));
                            cx.notify();
                        }),
                    )))
                    .child(div().w(px(128.0)).flex_none().child(kit::mono_sm(cx, c.cidr.clone())))
                    .child(
                        div().w(px(76.0)).flex_none().child(
                            Button::new(("netc-rm", i))
                                .ghost()
                                .xsmall()
                                .icon(icons::icon(role::DELETE))
                                .tooltip(copy::INV_REMOVE.tip)
                                .accessibility_label(format!("{} {}", copy::INV_REMOVE.short, c.cidr))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let cidr = cidr2.clone();
                                    this.with_app(cx, |a, _| a.settings.scan_networks.remove_custom(&cidr));
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(div().w(px(112.0)).flex_none()),
            );
        }
        col = col.child(list);
        if eff.fell_back {
            col = col.child(kit::caption(cx, copy::NET_GONE.short));
        }
        let none = eff.ids.is_empty() && eff.custom.is_empty();
        if none {
            col = col.child(
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .text_sm()
                    .text_color(p.err_fg)
                    .child(icons::icon_sized(role::ERR, 14.0, p.err_fg))
                    .child(copy::NET_NONE.short),
            );
        }
        col = col.child(kit::caption(cx, copy::NET_ADD_LABEL.short)).child(
            h_flex()
                .gap(px(8.0))
                .items_start()
                .child(div().flex_1().min_w_0().child(Input::new(&self.input).aria_label(copy::NET_ADD_LABEL.short)))
                .child(
                    Button::new("net-add")
                        .outline()
                        .label(copy::NET_ADD_BTN.short)
                        .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                ),
        );
        if let Some(e) = &self.error {
            col = col.child(
                h_flex()
                    .id("net-error")
                    .gap(px(6.0))
                    .items_start()
                    .text_sm()
                    .text_color(p.err_fg)
                    .child(icons::icon_sized(role::ERR, 14.0, p.err_fg))
                    .child(div().flex_1().min_w_0().child(e.clone())),
            );
        }
        let _ = muted;
        col.into_any_element()
    }
}

impl App {
    pub fn open_networks_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        if self.demo.is_none() {
            self.nets = synofurb_core::discovery::networks::networks();
        }
        let weak = cx.weak_entity();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(copy::NET_ADD_HINT.short));
        let view = cx.new(|cx| NetView::new(weak, input, window, cx));
        let app = cx.entity();
        window.open_dialog(cx, move |d, _, cx| {
            let (v1, a1, a2) = (view.clone(), app.clone(), app.clone());
            let none = {
                let a = app.read(cx);
                let e = a.settings.scan_networks.effective(&a.nets);
                e.ids.is_empty() && e.custom.is_empty()
            };
            d.bg(cx.theme().popover)
                .title(copy::NET_TITLE.short)
                .w(px(layout::DIALOG_W_WIDE))
                .child(view.clone())
                // Enter in the field adds the network; it does not close the dialog
                .on_ok(move |_, window, cx| {
                    v1.update(cx, |v, cx| v.add(window, cx));
                    false
                })
                .footer(
                    DialogFooter::new()
                        .child(Button::new("net-rec").ghost().label(copy::NET_SEL_REC.short).on_click(move |_, _, cx| {
                            a1.update(cx, |a, cx| {
                                a.settings.scan_networks.select_recommended();
                                a.save_settings();
                                cx.notify();
                            })
                        }))
                        .child(Button::new("net-all").ghost().label(copy::NET_SEL_ALL.short).on_click(move |_, _, cx| {
                            a2.update(cx, |a, cx| {
                                let nets = a.nets.clone();
                                a.settings.scan_networks.select_all(&nets);
                                a.save_settings();
                                cx.notify();
                            })
                        }))
                        .child(kit::with_reason(
                            "net-done-why",
                            Button::new("net-done").primary().label(copy::NET_DONE.short).on_click(|_, window, cx| window.close_dialog(cx)),
                            none.then(|| copy::NET_NONE.short.to_string()),
                        )),
                )
        });
    }
}
