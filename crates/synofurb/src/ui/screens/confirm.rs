//! Confirm erase (approved mockup, step 5): the destroyed / changed / preserved triad, the After row, the backup line, a
//! twin warning, and the danger card with the three confirmations and the typed phrase. Erase is a real disabled
//! `Button` with a sibling info popover until every condition holds. `Enter` never triggers Erase (the button swallows
//! it); Space and click do.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure::info_tip;
use crate::ui::format::size_2dp;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::raid;
use crate::ui::render::Page;
use crate::ui::state::*;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Hsla, IntoElement, StyleRefinement, Window, div, px};

impl App {
    /// A bordered card with an icon title and one value line (the triad).
    fn triad_card(&self, id: &'static str, title: AnyElement, border: Hsla, lines: Vec<AnyElement>) -> impl IntoElement {
        GroupBox::new()
            .outline()
            .id(id)
            .content_style(StyleRefinement::default().border_color(border).border_l_4())
            .title(title)
            .child(v_flex().gap(px(2.0)).children(lines))
    }

    pub(crate) fn page_confirm(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let (Some(f), Some(plan)) = (self.facts.clone(), self.form.plan.clone()) else {
            return Page::new(copy::CONF_TITLE.short, kit::muted(cx, "-")).left(vec![
                Button::new("back")
                    .ghost()
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Addons, window, cx)))
                    .into_any_element(),
            ]);
        };
        let name = self.unit_name();
        let phrase = self.phrase(cx);
        let typed = self.input_value(&self.inp.typed, cx);
        let gate = self.gate(&typed, &phrase, cx);
        let open = gate.is_open();
        let state = gate.phrase_state();
        let reason = gate.reason();
        let host = crate::ui::format::display_host(&self.input_value(&self.inp.host, cx));
        let twin = self.twin_of_selected(cx);
        let keep = plan.mode == synofurb_core::plan::StorageMode::KeepVolumes;
        let tail: String = f.serial.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        let unit_label = format!("{host} \u{00b7} \u{2026}{tail}");
        let raw_total: u64 = f.candidate_disks().iter().map(|d| d.size_bytes()).sum();

        // ---- left: backup line, twin, free-to-cancel, triad, After
        let backup = h_flex()
            .gap(px(2.0))
            .items_start()
            .child(
                Alert::warning("backup", if keep { copy::CONF_KEEP_RISK.short } else { copy::CONF_ERASED.short })
                    .title(copy::CONF_NO_BACKUP.short)
                    .icon(icons::icon(role::WARN))
                    .flex_1()
                    .min_w_0(),
            )
            .child(info_tip("backup-info", &copy::CONF_NO_BACKUP, &app));
        let scope0 = crate::ui::facts::DiskScope::of(&f);
        let destroyed_value = if keep {
            format!("{} GiB DSM system", f.candidate_disks().len() * 10)
        } else {
            format!("{} \u{00b7} {}", scope0.count_text(), size_2dp(raw_total))
        };
        let dest = self.triad_card(
            "tri-d",
            h_flex()
                .gap(px(6.0))
                .items_center()
                .text_color(p.err_fg)
                .child(icons::icon_sized(role::DELETE, 16.0, p.err_fg))
                .child(kit::strong(copy::CONF_DESTROYED.short))
                .into_any_element(),
            p.err_solid,
            vec![div().text_sm().child(destroyed_value).into_any_element()],
        );
        let dom = self.sys.boot.resolve(&f) == BootChoice::Dom;
        let chg = self.triad_card(
            "tri-c",
            h_flex()
                .gap(px(2.0))
                .items_center()
                .text_color(p.warn_fg)
                .child(icons::icon_sized(role::WARN, 16.0, p.warn_fg))
                .child(kit::strong(copy::CONF_CHANGED.short))
                .child(info_tip("chg-info", &copy::CONF_FLASH, &app))
                .into_any_element(),
            p.warn_solid,
            vec![
                div()
                    .text_sm()
                    .child(if dom {
                        "Boot flash \u{00b7} backed up first"
                    } else if keep {
                        copy::CONF_CHANGED_KEEP.short
                    } else {
                        copy::CONF_NOTHING_ELSE.short
                    })
                    .into_any_element(),
            ],
        );
        // Preserved: what stays of the DSM data (kept volumes) and the disks the install does not touch. The red card lists only what is
        // erased; a disk that is left alone is safe news and belongs here.
        let scope = crate::ui::facts::DiskScope::of(&f).with_plan(&plan);
        let mut kept: Vec<String> = plan.preserved.iter().map(|d| d.what.clone()).collect();
        kept.extend(scope.preserved_lines());
        let kept_lines: Vec<AnyElement> = if kept.is_empty() {
            vec![div().text_sm().child(copy::CONF_NOTHING_PRESERVED.short).into_any_element()]
        } else {
            kept.into_iter().map(|l| div().text_sm().child(l).into_any_element()).collect()
        };
        let pres = self.triad_card(
            "tri-p",
            h_flex()
                .gap(px(2.0))
                .items_center()
                .text_color(p.ok_fg)
                .child(icons::icon_sized(role::SAFE, 16.0, p.ok_fg))
                .child(kit::strong(copy::CONF_PRESERVED.short))
                .child(info_tip("pres-info", if keep { &copy::PLAN_PRESERVED_KEEP } else { &copy::CONF_PRESERVED }, &app))
                .into_any_element(),
            p.ok_solid,
            kept_lines,
        );
        let usable = size_2dp(plan.sizes.data_usable_bytes);
        let survives = raid::survives(&plan).map(|n| format!(" \u{00b7} survives {}", raid::survives_text(n))).unwrap_or_default();
        let hostname = self.input_value(&self.inp.hostname, cx);
        let sys_gib = if self.form.system_gib > 0.0 { self.form.system_gib } else { plan.sizes.recommended_system_mib as f32 / 1024.0 };
        let copies = plan.system_disks.len().max(1);
        let after_text = if keep {
            format!("Debian 13 \u{00b7} volume kept \u{00b7} system {:.0} GiB \u{00d7} {} \u{00b7} {hostname}", sys_gib, copies)
        } else {
            format!(
                "Debian 13 \u{00b7} {} \u{00b7} {usable} usable{survives} \u{00b7} system {:.0} GiB \u{00d7} {} \u{00b7} {hostname}",
                raid::resolved_label(&plan),
                sys_gib,
                copies
            )
        };
        let after = h_flex()
            .gap(px(8.0))
            .items_center()
            .px(px(12.0))
            .py(px(8.0))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(icons::icon(role::LAYERS))
            .child(div().text_sm().flex_1().min_w_0().child(after_text));
        // Add-ons chosen in Setup, however they were ticked (Add-ons tab or a Quick setup preset): the same engine text, so Quick setup
        // cannot skip it. One line of names, the lines that must not be missed in plain sight, the rest one click away.
        let addon_choices = self.addon_choices(cx);
        let addon_model = crate::ui::addons::confirm_model(
            &addon_choices,
            &synofurb_core::addons::AddonContext {
                offline: self.source_offline,
                root_mib: Some(plan.sizes.root_mib),
                has_data_volume: Some(plan.has_data_volume()),
            },
        );
        let priv_sig = synofurb_core::addons::consent_signature(&addon_choices);
        let addons_card = (!addon_model.names.is_empty()).then(|| {
            let loud = addon_model.loud.clone();
            let details = addon_model.details.clone();
            v_flex()
                .gap(px(4.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .flex_wrap()
                        .child(icons::icon(role::PACKAGE))
                        .child(div().text_sm().min_w_0().child(format!("{} {}", copy::CONF_ADDONS.short, addon_model.names.join(", "))))
                        .child(kit::untested(&p, "conf-untested", &app))
                        .child(crate::ui::disclosure::info_lines("conf-addon-info", copy::ADDON_ABOUT.short, details)),
                )
                .children(loud.into_iter().map(|l| {
                    h_flex()
                        .gap(px(6.0))
                        .items_start()
                        .text_sm()
                        .text_color(p.warn_fg)
                        .child(icons::icon_sized(role::WARN, 14.0, p.warn_fg))
                        .child(div().flex_1().min_w_0().child(l))
                }))
        });
        // What the plan itself warns about (a resyncing array, low RAM, a hybrid layout), on the Quick setup path too: the last screen
        // before the erase must not forget what the earlier one said. The not-touched sentence is in Preserved, a straddled array has its
        // own alert.
        let straddle_lines: Vec<String> = plan.straddling.iter().map(|s| s.warning()).collect();
        let plan_warnings: Vec<String> = plan
            .warnings
            .iter()
            .filter(|w| !w.starts_with("Not used and not touched") && !straddle_lines.contains(w))
            .cloned()
            .collect();
        let warnings_card = (!plan_warnings.is_empty()).then(|| {
            v_flex()
                .id("plan-warnings")
                .gap(px(2.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .border_l_4()
                .border_color(p.warn_solid)
                .child(
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .text_color(p.warn_fg)
                        .child(icons::icon_sized(role::WARN, 14.0, p.warn_fg))
                        .child(kit::strong(copy::CONF_WARNINGS.short)),
                )
                .children(plan_warnings.into_iter().map(|w| div().text_sm().child(w)))
        });
        // The disks that change, by name and size, always visible (B4): not behind "Details".
        let cand = f.candidate_disks();
        let shown = cand.len().min(crate::ui::facts::DESTROYED_LINES_MAX);
        let disk_list =
            v_flex()
                .id("destroyed-list")
                .gap(px(2.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .border_l_4()
                .border_color(p.err_solid)
                .child(
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .text_color(p.err_fg)
                        .child(icons::icon_sized(role::DELETE, 14.0, p.err_fg))
                        .child(kit::strong(if keep { copy::CONF_LIST_KEEP.short } else { copy::CONF_LIST.short })),
                )
                .children(cand.iter().take(shown).map(|d| {
                    kit::mono_sm(cx, crate::ui::facts::destroyed_disk_line(&d.name, &d.model, &d.serial, &size_2dp(d.size_bytes())))
                }))
                .when(cand.len() > shown, |c| c.child(kit::caption(cx, format!("and {} more", cand.len() - shown))));
        let details = v_flex().gap(px(2.0)).children(plan.destroyed.iter().map(|d| div().text_xs().child(d.clone())));
        let acc = Accordion::new("conf-details").bordered(true).item(|i| i.title(copy::CONF_DETAILS.short).child(details));

        let mut left = v_flex().gap(px(12.0)).min_w_0().flex_1().child(backup);
        if let Some(t) = &twin {
            left = left.child(
                Alert::warning(
                    "twin",
                    format!(
                        "{} \u{00b7} {} \u{00b7} SN \u{2026}{}",
                        copy::INV_TWIN.short,
                        t.primary_ip().map(|i| i.to_string()).unwrap_or_default(),
                        t.serial.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>()
                    ),
                )
                .icon(icons::icon(role::WARN)),
            );
        }
        left = left
            .child(h_flex().gap(px(2.0)).items_center().child(kit::muted(cx, copy::CONF_FALLBACK.short)).child(info_tip(
                "recov-info",
                &copy::CONF_HOW_RECOVERY,
                &app,
            )))
            .child(
                h_flex()
                    .gap(px(8.0))
                    .items_start()
                    .child(div().flex_1().min_w_0().child(dest))
                    .child(div().flex_1().min_w_0().child(chg))
                    .child(div().flex_1().min_w_0().child(pres)),
            )
            .child(disk_list)
            .children(plan.straddling.iter().enumerate().map(|(i, st)| {
                Alert::error(("straddle", i), st.warning()).icon(icons::icon(role::ERR))
            }))
            .children(warnings_card)
            .child(after)
            .children(addons_card)
            .child(acc);

        // ---- right: the danger card
        let match_tag: Option<AnyElement> = match state {
            PhraseMatch::Match => Some(kit::tag(&p, Tone::Ok, Some(role::OK), copy::CONF_MATCH.short).into_any_element()),
            PhraseMatch::Mismatch { .. } => Some(kit::tag(&p, Tone::Err, Some(role::ERR), copy::CONF_NO_MATCH.short).into_any_element()),
            PhraseMatch::Partial { n, total } => Some(kit::chip(&p, format!("{n} of {total}")).into_any_element()),
            PhraseMatch::Empty => None,
        };
        let card = GroupBox::new()
            .outline()
            .id("danger")
            .content_style(StyleRefinement::default().border_color(p.err_solid).border_2())
            .title(
                h_flex().gap(px(8.0)).items_center().text_color(p.err_fg).child(icons::icon_sized(role::STOP, 18.0, p.err_fg)).child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(phrase.clone()),
                ),
            )
            .child(
                v_flex()
                    .gap(px(8.0))
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
                    .when(!priv_sig.is_empty(), |d| {
                        let sig = priv_sig.clone();
                        let line = crate::ui::addons::privilege_line(&addon_model.privileges).unwrap_or_default();
                        d.child(
                            v_flex()
                                .gap(px(2.0))
                                .child(
                                    Checkbox::new("ack-priv")
                                        .label(copy::CONF_ADDON_ACK.short)
                                        .checked(self.ack_priv.as_deref() == Some(priv_sig.as_str()))
                                        .on_click(cx.listener(move |this, v: &bool, _, cx| {
                                            this.ack_priv = v.then(|| sig.clone());
                                            cx.notify();
                                        })),
                                )
                                .child(div().pl(px(28.0)).child(kit::caption(cx, line))),
                        )
                    })
                    .child(Checkbox::new("ack-unit").label(copy::CONF_UNIT.fmt(&[&unit_label])).checked(self.ack_unit).on_click(
                        cx.listener(|this, v: &bool, _, cx| {
                            this.ack_unit = *v;
                            cx.notify();
                        }),
                    ))
                    .child(kit::chip(&p, phrase.clone()).font_family(cx.theme().mono_font_family.clone()))
                    .child(
                        h_flex()
                            .gap(px(6.0))
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.inp.typed).aria_label(copy::CONF_PHRASE.short)))
                            .children(match_tag),
                    )
                    .when(twin.is_some() && state != PhraseMatch::Match, |d| d.child(kit::caption(cx, copy::CONF_ADD_SERIAL.short))),
            );

        let mut body = v_flex()
            .w_full()
            .gap(px(16.0))
            .child(h_flex().w_full().items_start().gap(px(16.0)).child(left).child(div().w(px(320.0)).flex_none().child(card)));
        if let Some(e) = &self.error {
            body = body.child(Alert::error("start-error", e.clone()).icon(icons::icon(role::ERR)));
        }
        let erase = Button::new("erase")
            .danger()
            .icon(icons::icon(role::DELETE))
            .label(copy::CONF_ERASE.short)
            .accessibility_label(format!("{} {}", copy::CONF_ERASE.short, name))
            .on_click(cx.listener(|this, _, window, cx| this.start_install(window, cx)));
        // Enter must never start the erase: the wrapper swallows it before the button sees it (Space and click still work).
        let erase_el = div()
            .capture_key_down(|ev, _, cx| {
                if ev.keystroke.key == "enter" {
                    cx.stop_propagation();
                }
            })
            .child(kit::with_reason("erase-why", erase, (!open).then(|| reason.unwrap_or_default())));
        Page::new(copy::CONF_TITLE.short, body)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Addons, window, cx)))
                    .into_any_element(),
                self.copy_command_button(cx),
            ])
            .right(vec![erase_el.into_any_element()])
    }
}
