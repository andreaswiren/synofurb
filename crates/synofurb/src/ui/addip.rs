//! The "Add NAS" dialog and what happens after it: the typed address is validated, then checked off the UI thread with visible
//! progress and a Cancel; the outcome becomes a row on Find (see `addhost.rs` for the rules and the report).
//!
//! The dialog is one entity (`AddView`) that owns its input, so a click or Enter always reads the text that is in the field at
//! that moment. Nothing is ever ignored silently: an empty or invalid address shows its reason under the field and the dialog
//! stays open.

use super::addhost::{self, AddError, AddedHost, CheckResult, CheckState, ProbeReport, Target};
use super::app::App;
use super::copy;
use super::icons::{self, role};
use super::state::AddDemo;
use super::theme::{Themed, Tone, layout};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, Context, Entity, IntoElement, Subscription, WeakEntity, Window, div, px};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddState {
    /// Waiting for an address; the error (if any) is shown under the field.
    Idle(Option<AddError>),
    /// Checking the address.
    Probing,
}

pub struct AddView {
    input: Entity<InputState>,
    pub state: AddState,
    names: Vec<String>,
    done: Vec<Option<CheckResult>>,
    cancel: Option<Arc<AtomicBool>>,
    /// The probe this dialog started (`App::add_gen` when it began).
    pub ticket: u64,
    app: WeakEntity<App>,
    reduce_motion: bool,
    _sub: Subscription,
}

impl AddView {
    fn new(
        app: WeakEntity<App>,
        input: Entity<InputState>,
        reduce_motion: bool,
        window: &mut Window,
        cx: &mut Context<AddView>,
    ) -> AddView {
        // Enter in the field does what the Add button does; typing takes the error away.
        let sub = cx.subscribe_in(&input, window, |this: &mut AddView, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } => this.submit(window, cx),
            InputEvent::Change => {
                if matches!(this.state, AddState::Idle(Some(_))) {
                    this.state = AddState::Idle(None);
                    cx.notify();
                }
            }
            _ => {}
        });
        AddView { input, state: AddState::Idle(None), names: vec![], done: vec![], cancel: None, ticket: 0, app, reduce_motion, _sub: sub }
    }

    pub fn probing(&self) -> bool {
        self.state == AddState::Probing
    }

    pub fn typed(&self, cx: &gpui_kit::App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Show why the typed address is not valid (nothing is started).
    pub fn show_error_for_typed(&mut self, _window: &mut Window, cx: &mut Context<AddView>) {
        if let Err(e) = addhost::parse_target(&self.typed(cx)) {
            self.state = AddState::Idle(Some(e));
            cx.notify();
        }
    }

    /// Validate what is in the field now; start checking it, or show why not. Safe to call from a click and from Enter.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<AddView>) {
        if self.probing() {
            return;
        }
        let text = self.typed(cx);
        match addhost::parse_target(&text) {
            Err(e) => {
                self.state = AddState::Idle(Some(e));
                let f = gpui_kit::Focusable::focus_handle(&self.input, cx);
                window.focus(&f, cx);
                cx.notify();
            }
            Ok(t) => {
                let shown = t.display();
                self.input.update(cx, |s, cx| s.set_value(shown, window, cx));
                self.begin(t, cx);
            }
        }
    }

    /// Ask the app to start checking `t` (a click or Enter: nobody holds the app open).
    fn begin(&mut self, t: Target, cx: &mut Context<AddView>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let Some(app) = self.app.upgrade() else { return };
        let (t2, c2) = (t.clone(), cancel.clone());
        let ticket = app.update(cx, |a, cx| a.start_add_probe(t2, c2, cx));
        self.enter_probing(&t, cancel, ticket, cx);
    }

    /// Show the checks of `t` as running (the probe itself was started by the app).
    pub fn enter_probing(&mut self, t: &Target, cancel: Arc<AtomicBool>, ticket: u64, cx: &mut Context<AddView>) {
        self.names = addhost::check_names(t);
        self.done = vec![None; self.names.len()];
        self.state = AddState::Probing;
        self.cancel = Some(cancel);
        self.ticket = ticket;
        cx.notify();
    }

    /// Stop checking and go back to the form with the address still typed.
    pub fn stop(&mut self, cx: &mut Context<AddView>) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        if self.probing()
            && let Some(app) = self.app.upgrade()
        {
            app.update(cx, |a, _| a.add_gen += 1); // a late answer is ignored
        }
        self.state = AddState::Idle(None);
        cx.notify();
    }

    pub fn on_step(&mut self, idx: usize, r: CheckResult, cx: &mut Context<AddView>) {
        if let Some(slot) = self.done.get_mut(idx) {
            *slot = Some(r);
            cx.notify();
        }
    }
}

fn state_icon(s: CheckState, p: &super::theme::Palette) -> gpui_kit::component::Icon {
    match s {
        CheckState::Answered => icons::icon_sized(role::OK, 14.0, p.tone(Tone::Ok).fg),
        CheckState::Closed => icons::icon_sized(role::INFO, 14.0, p.tone(Tone::Info).fg),
        CheckState::NoReply => icons::icon_sized(role::PENDING, 14.0, p.tone(Tone::Neutral).fg),
        CheckState::Failed => icons::icon_sized(role::ERR, 14.0, p.tone(Tone::Err).fg),
    }
}

impl gpui_kit::Render for AddView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.pal();
        let muted = cx.theme().muted_foreground;
        let probing = self.probing();
        let mut col = v_flex().gap(px(8.0)).child(div().text_sm().child(copy::INV_ADD_TITLE.body));
        col = col.child(Input::new(&self.input).cleanable(!probing).disabled(probing).aria_label(copy::INV_ADD_ADDR.short));
        if let AddState::Idle(Some(e)) = self.state {
            col = col.child(
                h_flex()
                    .id("add-error")
                    .gap(px(6.0))
                    .items_center()
                    .text_sm()
                    .text_color(p.err_fg)
                    .child(icons::icon_sized(role::ERR, 14.0, p.err_fg))
                    .child(copy::add_error(e)),
            );
        }
        if probing {
            col = col.child(v_flex().gap(px(4.0)).children(self.names.iter().enumerate().map(|(i, n)| {
                let r = self.done.get(i).cloned().flatten();
                let lead = match &r {
                    Some(r) => state_icon(r.state, &p).into_any_element(),
                    None if self.reduce_motion => icons::icon_sized(role::RUNNING, 14.0, muted).into_any_element(),
                    None => Spinner::new().xsmall().into_any_element(),
                };
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .text_sm()
                    .child(div().w(px(16.0)).flex_none().child(lead))
                    .child(div().flex_none().child(n.clone()))
                    .when_some(r, |d, r| d.child(div().flex_1().min_w_0().truncate().text_xs().text_color(muted).child(r.detail)))
            })));
        }
        col
    }
}

impl App {
    /// "Add NAS": type an address, it is checked, the result becomes a row. `prefill` fills the field and checks it at once
    /// (Check again); `demo` opens the dialog in one of its states for screenshots.
    pub fn open_add_dialog(&mut self, prefill: Option<Target>, demo: Option<AddDemo>, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let weak = cx.weak_entity();
        let reduce = self.settings.reduce_motion;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(copy::INV_ADD_HINT.short));
        let view = cx.new(|cx| AddView::new(weak, input.clone(), reduce, window, cx));
        self.add_view = Some(view.downgrade());
        let _ = app;
        let view2 = view.clone();
        window.open_dialog(cx, move |d, _, cx| {
            let (v_ok, v_cancel, v_close, v_enter) = (view2.clone(), view2.clone(), view2.clone(), view2.clone());
            let probing = view2.read(cx).probing();
            d.bg(cx.theme().popover)
                .title(copy::INV_ADD_TITLE.short)
                .w(px(layout::DIALOG_W))
                .child(view2.clone())
                // Enter must never just close the dialog: it checks the address, like the Add button
                .on_ok(move |_, window, cx| {
                    v_enter.update(cx, |v, cx| v.submit(window, cx));
                    false
                })
                .on_close(move |_, _, cx| v_close.update(cx, |v, cx| v.stop(cx)))
                .footer(
                    DialogFooter::new()
                        .child(Button::new("add-cancel").outline().label(copy::BTN_CANCEL.short).on_click(move |_, window, cx| {
                            if v_cancel.read(cx).probing() {
                                v_cancel.update(cx, |v, cx| v.stop(cx));
                            } else {
                                window.close_dialog(cx);
                            }
                        }))
                        .child(
                            Button::new("add-ok")
                                .primary()
                                .label(copy::INV_ADD_BTN.short)
                                .loading(probing)
                                .disabled(probing)
                                .on_click(move |_, window, cx| v_ok.update(cx, |v, cx| v.submit(window, cx))),
                        ),
                )
        });
        // the field has the keyboard as soon as the dialog opens
        let f = gpui_kit::Focusable::focus_handle(&input, cx);
        cx.defer_in(window, move |_, window, cx| window.focus(&f, cx));
        // Check again, and the demo states, start from inside the app's own update: the probe is started here, not through the view.
        let start = match (prefill, demo) {
            (Some(t), _) => Some(t),
            (None, Some(AddDemo::Probing)) => addhost::parse_target("192.168.1.150").ok(),
            (None, Some(AddDemo::Invalid)) => {
                input.update(cx, |s, cx| s.set_value("192.168.1", window, cx));
                view.update(cx, |v, cx| v.show_error_for_typed(window, cx));
                None
            }
            _ => None,
        };
        if let Some(t) = start {
            let shown = t.display();
            input.update(cx, |s, cx| s.set_value(shown, window, cx));
            let cancel = Arc::new(AtomicBool::new(false));
            let ticket = self.start_add_probe(t.clone(), cancel.clone(), cx);
            view.update(cx, |v, cx| v.enter_probing(&t, cancel, ticket, cx));
        }
    }

    /// Start the check of an address; returns the number that ties the answers to this check.
    pub fn start_add_probe(&mut self, t: Target, cancel: Arc<AtomicBool>, _cx: &mut Context<App>) -> u64 {
        self.add_gen += 1;
        let ticket = self.add_gen;
        if self.demo.is_some() {
            super::demo::spawn_add_script(self.tx.clone(), ticket, t, cancel);
        } else {
            super::worker::spawn_add_probe(self.tx.clone(), ticket, t, cancel);
        }
        ticket
    }

    pub fn on_add_step(&mut self, ticket: u64, idx: usize, r: CheckResult, cx: &mut Context<App>) {
        if ticket != self.add_gen {
            return;
        }
        if let Some(v) = self.add_view.as_ref().and_then(|v| v.upgrade()) {
            v.update(cx, |v, cx| v.on_step(idx, r, cx));
        }
    }

    /// The check ended: keep the outcome as a row, select it and close the dialog. The Connect screen is not opened; the person
    /// presses Connect.
    pub fn on_add_done(&mut self, ticket: u64, target: Target, report: ProbeReport, window: &mut Window, cx: &mut Context<App>) {
        if ticket != self.add_gen {
            return;
        }
        let Some(v) = self.add_view.as_ref().and_then(|v| v.upgrade()) else { return };
        if !v.read(cx).probing() {
            return;
        }
        let key = target.host.clone();
        let port = target.port;
        let host = AddedHost { target, report, checked: super::worker::now_text() };
        self.added.upsert(host);
        self.save_added();
        self.select_row(&key, port, window, cx);
        v.update(cx, |v, _| v.state = AddState::Idle(None));
        window.close_dialog(cx);
        cx.notify();
    }

    /// Select a row of Find without leaving the screen.
    pub fn select_row(&mut self, key: &str, port: Option<u16>, window: &mut Window, cx: &mut Context<App>) {
        if self.selected_ip.as_deref() != Some(key) {
            self.reset_unit_session();
        }
        self.selected_ip = Some(key.to_string());
        self.set_input(&self.inp.host.clone(), key, window, cx);
        if let Some(p) = port {
            self.set_input(&self.inp.port.clone(), &p.to_string(), window, cx);
        }
        cx.notify();
    }

    pub fn save_added(&self) {
        if self.demo.is_none() {
            let _ = self.added.save(&self.cfg_dir.join("added-hosts.json"));
        }
    }

    pub fn remove_added(&mut self, key: &str, cx: &mut Context<App>) {
        if self.added.remove(key) {
            self.save_added();
            if self.selected_ip.as_deref() == Some(key) {
                self.selected_ip = None;
            }
            cx.notify();
        }
    }

    /// Check an address again (the row's "Check again").
    pub fn recheck_added(&mut self, key: &str, window: &mut Window, cx: &mut Context<App>) {
        if let Some(h) = self.added.get(key) {
            let t = h.target.clone();
            self.open_add_dialog(Some(t), None, window, cx);
        }
    }
}
