//! D7 Confirm: the data-loss gate. Identity, Destroyed / Changed / Preserved as names and sizes, three confirmations,
//! the typed phrase. The red button stays disabled (reason as tooltip) until every condition holds; paste is allowed,
//! `Enter` never triggers it.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure::{dialog, info_tip};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::*;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, AnyElement, Context, Hsla, IntoElement, StyleRefinement, Window, div, px};

impl App {
    fn gate_box(&self, title: AnyElement, border: Hsla, lines: Vec<AnyElement>) -> impl IntoElement {
        GroupBox::new()
            .outline()
            .content_style(StyleRefinement::default().border_color(border).border_2())
            .title(title)
            .child(v_flex().gap(px(4.0)).children(lines))
            .id(border.h.to_bits() as usize)
    }

    pub(crate) fn page_confirm(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let (Some(f), Some(plan)) = (self.facts.clone(), self.form.plan.clone()) else {
            return Page::new(copy::CONF_TITLE.short, kit::muted(cx, "-")).left(vec![
                Button::new("back")
                    .ghost()
                    .label(copy::BTN_BACK.short)
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
        let verdict = self.compat_for_facts(&f);
        let host = crate::ui::format::display_host(&self.input_value(&self.inp.host, cx));
        let identity = kit::identity_line(cx, &p, &name, &host, &f.serial, Some(verdict.status));

        let line = |t: String| div().text_xs().child(t).into_any_element();
        let destroyed = self.gate_box(
            h_flex()
                .gap(px(6.0))
                .items_center()
                .child(icons::icon_sized(role::DELETE, 16.0, p.err_fg))
                .child(kit::strong(copy::CONF_DESTROYED.short))
                .into_any_element(),
            p.err_solid,
            plan.destroyed.iter().cloned().map(line).collect(),
        );
        let dom = self.sys.boot.resolve(&f) == BootChoice::Dom;
        let changed_lines: Vec<AnyElement> = if dom {
            vec![
                HoverCard::new("flash-card")
                    .trigger(div().id("flash-trigger").text_sm().child(format!("{} \u{00b7} backed up", copy::CONF_FLASH.short)))
                    .content(|_, _, _| div().p(px(10.0)).max_w(px(300.0)).text_sm().child(copy::CONF_FLASH.tip))
                    .into_any_element(),
            ]
        } else {
            vec![kit::caption(cx, "-").into_any_element()]
        };
        let changed = self.gate_box(
            h_flex()
                .gap(px(6.0))
                .items_center()
                .child(icons::icon_sized(role::WARN, 16.0, p.warn_fg))
                .child(kit::strong(copy::CONF_CHANGED.short))
                .into_any_element(),
            p.warn_solid,
            changed_lines,
        );
        let mut kept: Vec<AnyElement> = plan.preserved.iter().map(|d| line(format!("{} \u{00b7} {}", d.what, d.detail))).collect();
        if kept.is_empty() {
            kept.push(line(copy::CONF_NOTHING_PRESERVED.short.into()));
        }
        let preserved = self.gate_box(
            h_flex()
                .gap(px(2.0))
                .items_center()
                .child(icons::icon_sized(role::SAFE, 16.0, p.ok_fg))
                .child(kit::strong(copy::CONF_PRESERVED.short))
                .child(info_tip("pres-info", &copy::CONF_PRESERVED, &app))
                .into_any_element(),
            p.ok_solid,
            kept,
        );

        // tags: status (tooltip through hover card), time (popover with the checklist)
        let status_tag = HoverCard::new("status-tag")
            .trigger(div().id("status-tag").child(kit::compat_tag(&p, verdict.status)))
            .content(|_, _, _| div().p(px(10.0)).max_w(px(300.0)).text_sm().child(copy::CONF_UNTESTED.tip));
        let time = Popover::new("time-pop")
            .anchor(Anchor::TopLeft)
            .trigger(Button::new("time-btn").ghost().small().icon(icons::icon(role::CLOCK)).label(copy::CONF_ABOUT_TIME.short))
            .content({
                let app = app.clone();
                move |_, _, cx| {
                    let popover = cx.entity();
                    let app = app.clone();
                    v_flex()
                        .w(px(300.0))
                        .gap(px(8.0))
                        .text_sm()
                        .child(kit::strong(copy::CONF_ABOUT_TIME.tip))
                        .child(crate::ui::disclosure::paragraphs(copy::CONF_ABOUT_TIME.body, cx))
                        .child(h_flex().justify_end().child(Button::new("lm").link().small().label(copy::BTN_LEARN_MORE.short).on_click(
                            move |_, window, cx| {
                                popover.update(cx, |s, cx| s.dismiss(window, cx));
                                app.update(cx, |a, cx| a.open_help_topic(Some("first-install"), window, cx));
                            },
                        )))
                }
            });
        let twin = {
            let ip = self.selected_ip.clone().unwrap_or_default();
            self.inv
                .devices
                .iter()
                .find(|d| {
                    !d.synofurb_debian
                        && !f.model.is_empty()
                        && d.model == f.model
                        && d.primary_ip().map(|i| i.to_string()).is_some_and(|i| i != ip && !host.contains(&i))
                })
                .cloned()
        };

        let match_tag: Option<AnyElement> = match state {
            PhraseMatch::Match => Some(kit::tag(&p, Tone::Ok, Some(role::OK), copy::CONF_MATCH.short).into_any_element()),
            PhraseMatch::Mismatch { .. } => Some(kit::tag(&p, Tone::Err, Some(role::ERR), copy::CONF_NO_MATCH.short).into_any_element()),
            _ => None,
        };
        let checks = v_flex()
            .gap(px(10.0))
            .child(Checkbox::new("ack-backup").label(copy::CONF_BACKUP.short).checked(self.ack_backup).on_click(cx.listener(
                |this, v: &bool, _, cx| {
                    this.ack_backup = *v;
                    cx.notify();
                },
            )))
            .child(Checkbox::new("ack-loss").label(copy::CONF_LOSS.short).checked(self.ack_loss).on_click(cx.listener(
                |this, v: &bool, _, cx| {
                    this.ack_loss = *v;
                    cx.notify();
                },
            )))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(Checkbox::new("ack-unit").label(copy::CONF_UNIT.short).checked(self.ack_unit).on_click(cx.listener(
                        |this, v: &bool, _, cx| {
                            this.ack_unit = *v;
                            cx.notify();
                        },
                    )))
                    .child(info_tip("unit-info", &copy::CONF_UNIT, &app)),
            )
            .child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .flex_wrap()
                    .child(kit::labelled("phrase-label", &copy::CONF_PHRASE, &app))
                    .child(kit::chip(&p, phrase.clone()).font_family(cx.theme().mono_font_family.clone()))
                    .child(div().w(px(240.0)).child(Input::new(&self.inp.typed).aria_label(copy::CONF_PHRASE.short)))
                    .children(match_tag),
            );

        let mut body = v_flex().w_full().gap(px(14.0)).child(identity).child(
            h_flex()
                .w_full()
                .items_start()
                .gap(px(12.0))
                .child(div().flex_grow(2.0).w(px(0.0)).min_w_0().child(destroyed))
                .child(div().flex_grow(1.0).w(px(0.0)).min_w_0().child(changed))
                .child(div().flex_grow(1.0).w(px(0.0)).min_w_0().child(preserved)),
        );
        body = body.child(h_flex().gap(px(8.0)).items_center().flex_wrap().child(status_tag).child(time));
        if let Some(t) = twin {
            body = body.child(
                Alert::warning("twin", format!("{} \u{00b7} {}", t.model, t.primary_ip().map(|i| i.to_string()).unwrap_or_default()))
                    .title(copy::CONF_TWIN.short)
                    .small()
                    .icon(icons::icon(role::WARN)),
            );
        }
        body = body.child(checks).child(h_flex().gap(px(8.0)).items_center().child(kit::muted(cx, copy::CONF_FALLBACK.short)).child(
            Button::new("how-recovery").link().small().label(copy::CONF_HOW_RECOVERY.short).on_click(cx.listener(|this, _, window, cx| {
                let app = cx.entity();
                let _ = this;
                dialog(window, cx, &copy::DLG_RECOVERY, &app);
            })),
        ));
        if let Some(e) = &self.error {
            body = body.child(Alert::error("start-error", e.clone()).icon(icons::icon(role::ERR)));
        }
        let erase_tip = reason.unwrap_or_default();
        Page::new(copy::CONF_TITLE.short, body)
            .subtitle(copy::CONF_SUB.short)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::System, window, cx)))
                    .into_any_element(),
                self.copy_command_button(cx),
            ])
            .right(vec![
                Button::new("erase")
                    .danger()
                    .icon(icons::icon(role::DELETE))
                    .label(copy::CONF_ERASE.short)
                    .accessibility_label(format!("{} {}", copy::CONF_ERASE.short, name))
                    .disabled(!open)
                    .tooltip(erase_tip)
                    .on_click(cx.listener(|this, _, window, cx| this.start_install(window, cx)))
                    .into_any_element(),
            ])
    }
}
