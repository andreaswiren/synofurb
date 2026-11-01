//! Step 7: the data-loss gate. A page, not a modal. Order is fixed: title with the unit name, error banner, destroyed
//! vs preserved, backup finding, two checkboxes, typed phrase, then the action bar where the red button stays
//! disabled (with the reason beside it) until everything is satisfied. Paste into the phrase field is allowed.

use crate::ui::app::App;
use crate::ui::format::{plural, size_2dp};
use crate::ui::render::Page;
use crate::ui::state::*;
use crate::ui::theme::{self, Themed, Tone, radius, space};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, IntoElement, Role, Window, div, px};
use synofurb_core::plan::StorageMode;

impl App {
    pub(crate) fn page_confirm(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let (Some(f), Some(plan)) = (self.facts.clone(), self.form.plan.clone()) else {
            return Page::new("Confirm data loss", muted(&p, "Nothing to confirm yet.")).buttons(vec![
                btn_secondary("back", "Back")
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::System, window, cx)))
                    .into_any_element(),
            ]);
        };
        let name = self.unit_name();
        let phrase = self.phrase();
        let typed = self.input_value(&self.inp.typed, cx);
        let gate = self.gate(&typed, &phrase);
        let open = gate.is_open();
        let reason = gate.reason();
        let state = gate.phrase_state();
        let raw_total: u64 = f.candidate_disks().iter().map(|d| d.size_bytes()).sum();
        let mode_word = match plan.mode {
            StorageMode::WipeAll => "Wipe everything",
            StorageMode::KeepVolumes => "Keep existing volumes (best effort)",
        };

        let headline = banner(
            &p,
            Tone::Err,
            format!("Everything on the selected disks of {name} will be erased"),
            format!(
                "{} ({} raw) are repartitioned. There is no undo, and no guarantee that any data survives, even volumes marked as preserved. Back up first: a backup you have not tested is a hope, not a backup.",
                plural(f.candidate_disks().len(), "disk", "disks"),
                size_2dp(raw_total)
            ),
            vec![],
        )
        .id("loss-banner").role(Role::Alert);

        let strip = h_flex()
            .w_full()
            .rounded(px(radius::MD))
            .overflow_hidden()
            .border_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .flex_1()
                    .gap(px(space::S8))
                    .items_center()
                    .px(px(space::S12))
                    .py(px(space::S6))
                    .bg(p.ok_bg)
                    .child(icon_sized("circle-check", 14.0, p.ok_fg))
                    .child(
                        text(&p, theme::CAPTION, "Before the point of no return: Cancel is free, the disks are untouched")
                            .text_color(p.ok_fg),
                    ),
            )
            .child(div().w(px(2.0)).bg(p.err_solid))
            .child(
                h_flex()
                    .flex_1()
                    .gap(px(space::S8))
                    .items_center()
                    .px(px(space::S12))
                    .py(px(space::S6))
                    .bg(p.err_bg)
                    .child(icon_sized("flag", 14.0, p.err_fg))
                    .child(text(&p, theme::CAPTION, "After it (step 3 of 12): DSM is gone; the disks have changed").text_color(p.err_fg)),
            );

        let destroyed = card(
            &p,
            Some("Will be destroyed".into()),
            Some(badge(&p, Tone::Err, "circle-x", plural(plan.destroyed.len(), "item", "items"), false).into_any_element()),
            Some(p.err_solid),
            v_flex().gap(px(space::S8)).children(plan.destroyed.iter().map(|d| {
                h_flex()
                    .gap(px(space::S8))
                    .items_start()
                    .child(icon_sized("trash", 14.0, p.err_fg).mt(px(3.0)))
                    .child(text(&p, theme::BODY_SM, d.clone()).flex_1().min_w_0())
            })),
        );
        let mut kept = v_flex().gap(px(space::S8));
        if plan.preserved.is_empty() {
            kept = kept.child(muted(&p, format!("{mode_word}: no data on the disks of {name} is preserved.")));
        }
        for d in &plan.preserved {
            kept = kept.child(
                h_flex().gap(px(space::S8)).items_start().child(icon_sized("shield-check", 14.0, p.ok_fg).mt(px(3.0))).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(text(&p, theme::BODY_SM, d.what.clone()).font_weight(FontWeight(600.0)))
                        .child(muted(&p, d.detail.clone()).text_size(theme::CAPTION.size())),
                ),
            );
        }
        for l in ["Everything on this computer, including the keys and logs synofurb writes", "Other devices on your network"] {
            kept = kept.child(
                h_flex()
                    .gap(px(space::S8))
                    .items_start()
                    .child(icon_sized("shield-check", 14.0, p.ok_fg).mt(px(3.0)))
                    .child(text(&p, theme::BODY_SM, l).flex_1().min_w_0()),
            );
        }
        let preserved = card(
            &p,
            Some(if plan.preserved.is_empty() {
                "Will be preserved (outside the NAS)".into()
            } else {
                "Will be preserved (best effort, no guarantee)".into()
            }),
            Some(badge(&p, Tone::Ok, "check", plural(plan.preserved.len() + 2, "item", "items"), false).into_any_element()),
            Some(p.ok_solid),
            kept,
        );

        let layout_lines = v_flex().gap(px(space::S4)).children(plan.summary().into_iter().map(|l| muted(&p, l)));
        let mut notes = v_flex().gap(px(space::S8));
        for w in &plan.warnings {
            notes = notes.child(banner(&p, Tone::Warn, "Warning", w.clone(), vec![]));
        }

        let backup_banner = banner(
            &p,
            Tone::Warn,
            "synofurb cannot check whether you have a backup",
            "It does not look inside DSM's backup tasks. That does not mean you have none elsewhere; only you can confirm it.",
            vec![],
        );

        let fb = fallback_lines(f.firmware);
        let fallback = card(
            &p,
            Some("If something goes wrong: the fallback plan".into()),
            None,
            None,
            v_flex().gap(px(space::S6)).children(fb.iter().enumerate().map(|(i, l)| muted(&p, format!("{}. {l}", i + 1)))),
        );

        let phrase_status: gpui_kit::AnyElement = match state {
            PhraseMatch::Match => h_flex()
                .gap(px(space::S6))
                .items_center()
                .child(icon_sized("circle-check", 14.0, p.ok_fg))
                .child(text(&p, theme::BODY_SM, "Matches").text_color(p.ok_fg).font_weight(FontWeight(600.0)))
                .into_any_element(),
            PhraseMatch::Empty => muted(&p, format!("Type {phrase} exactly. Case matters. You can paste it.")).into_any_element(),
            PhraseMatch::Partial { n, total } => muted(&p, format!("{n} of {total} characters match - keep typing.")).into_any_element(),
            PhraseMatch::Mismatch { at } => h_flex()
                .gap(px(space::S6))
                .items_center()
                .child(icon_sized("circle-x", 14.0, p.err_fg))
                .child(text(&p, theme::BODY_SM, format!("Does not match at character {}. Expected {phrase}.", at + 1)).text_color(p.err_fg))
                .into_any_element(),
        };
        let checks = card(
            &p,
            Some("Confirm".into()),
            None,
            None,
            v_flex()
                .gap(px(space::S12))
                .child(Checkbox::new("ack-backup").label("I have a complete, verified backup of everything on this NAS that I want to keep.").checked(self.ack_backup).on_click(cx.listener(|this, v: &bool, _, cx| {
                    this.ack_backup = *v;
                    cx.notify();
                })))
                .child(Checkbox::new("ack-loss").label(format!("I understand that all data on {name}'s disks will be erased, that data marked as preserved can also be lost, and I accept this risk.")).checked(self.ack_loss).on_click(cx.listener(|this, v: &bool, _, cx| {
                    this.ack_loss = *v;
                    cx.notify();
                })))
                .child(divider(&p))
                .child(
                    h_flex()
                        .gap(px(space::S8))
                        .items_center()
                        .flex_wrap()
                        .child(muted(&p, "This unit:"))
                        .child(chip(&p, name.clone()))
                        .child(chip(&p, format!("address {}", super::super::format::display_host(&self.input_value(&self.inp.host, cx)))))
                        .child(chip(&p, format!("serial {}", if f.serial.is_empty() { "unknown".to_string() } else { f.serial.clone() }))),
                )
                .child(field(&p, &format!("Type {phrase} to unlock the button"), None, div().max_w(px(420.0)).child(Input::new(&self.inp.typed).aria_label("Confirmation phrase"))))
                .child(div().id("phrase-status").role(Role::Status).child(phrase_status)),
        );

        let mut body = v_flex().w_full().gap(px(space::S16)).child(headline).child(strip);
        body = body
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap(px(space::S16))
                    .child(div().flex_1().min_w_0().child(destroyed))
                    .child(div().flex_1().min_w_0().child(preserved)),
            )
            .child(card(
                &p,
                Some(format!("New layout ({mode_word})").into()),
                None,
                None,
                v_flex().gap(px(space::S8)).child(layout_lines).child(notes),
            ))
            .child(backup_banner)
            .child(fallback)
            .child(checks);
        if let Some(e) = &self.error {
            body = body.child(banner(&p, Tone::Err, "Could not start", e.clone(), vec![]).id("start-error").role(Role::Alert));
        }

        let note: gpui_kit::AnyElement = match reason {
            Some(r) => h_flex()
                .gap(px(space::S8))
                .items_center()
                .child(icon_sized("lock", 14.0, p.fg_2))
                .child(muted(&p, r).flex_1().min_w_0())
                .into_any_element(),
            None => h_flex()
                .gap(px(space::S8))
                .items_center()
                .child(icon_sized("triangle-alert", 14.0, p.err_fg))
                .child(
                    text(&p, theme::BODY_SM, "Ready. The next click starts the installation.")
                        .text_color(p.err_fg)
                        .font_weight(FontWeight(600.0)),
                )
                .into_any_element(),
        };
        let erase_label = format!("Erase {name} and install Debian");
        Page::new(format!("Confirm data loss: {name}"), body)
            .subtitle(format!("Install Debian 13 on {name}. Read both lists. This cannot be undone."))
            .note(note)
            .buttons(vec![
                btn_primary("back-plan", "Back to the plan")
                    .icon(icon("arrow-left"))
                    .large()
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Plan, window, cx)))
                    .into_any_element(),
                btn_secondary("cancel", "Cancel")
                    .large()
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
                Button::new("erase")
                    .danger()
                    .large()
                    .icon(icon("trash"))
                    .label(erase_label.clone())
                    .accessibility_label(erase_label)
                    .disabled(!open)
                    .on_click(cx.listener(|this, _, window, cx| this.start_install(window, cx)))
                    .into_any_element(),
            ])
    }
}
