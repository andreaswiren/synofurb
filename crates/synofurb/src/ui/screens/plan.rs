//! D5 Plan: mode, system size, mirror copies, data RAID, free space, the layout diagram and the Destroyed / Changed /
//! Preserved summary. Every control recomputes the plan; warnings and errors are Alerts, never only the picture.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::diagram::{self, DiagramModel};
use crate::ui::format::{plural, size_2dp};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::form::Form;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::component::select::Select;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, IntoElement, Window, div, px};
use synofurb_core::plan::StorageMode;

impl App {
    pub(crate) fn page_plan(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        self.rebuild_plan();
        self.sync_system_slider(cx);
        let (Some(f), Some(plan)) = (self.facts.clone(), self.form.plan.clone()) else {
            return Page::new(copy::PLAN_TITLE.short, kit::muted(cx, "-")).left(vec![
                Button::new("back")
                    .ghost()
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
            ]);
        };
        let n_disks = f.candidate_disks().len();
        let keep = self.form.req.mode == StorageMode::KeepVolumes;

        // ---- mode: one visible option, the rest behind Advanced
        let mode = h_flex()
            .gap(px(12.0))
            .items_center()
            .child(kit::segmented("mode", &[if keep { copy::PLAN_KEEP.short } else { copy::PLAN_WIPE.short }], 0, |_, _, _| {}))
            .child(
                Button::new("advanced")
                    .link()
                    .label(copy::PLAN_ADVANCED.short)
                    .on_click(cx.listener(|this, _, window, cx| this.open_keep_dialog(window, cx))),
            );
        let mode_tip = {
            let mut it = copy::PLAN_MODE.fmt(&[&n_disks.to_string()]);
            if keep {
                it = copy::PLAN_KEEP.tip.to_string();
            }
            it
        };
        let _ = mode_tip;

        // ---- system size, copies, raid, free space
        let (min, max, rec) = (
            plan.sizes.min_system_mib as f32 / 1024.0,
            plan.sizes.max_system_mib as f32 / 1024.0,
            plan.sizes.recommended_system_mib as f32 / 1024.0,
        );
        let size_ok = max >= min;
        let chosen = self.form.system_gib;
        let size_row = h_flex()
            .gap(px(12.0))
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Slider::new(&self.form.system_slider).disabled(!size_ok).accessibility_label(copy::PLAN_SYSTEM.short)),
            )
            .child(kit::mono(cx, format!("{chosen:.0} GiB")).w(px(72.0)))
            .child(
                Button::new("rec-size")
                    .ghost()
                    .small()
                    .icon(icons::icon(role::SPARKLES))
                    .accessibility_label("Recommended")
                    .tooltip(format!("Recommended: {rec:.0} GiB"))
                    .disabled(!size_ok)
                    .on_click(cx.listener(move |this, _, window, cx| this.set_system_gib(rec, window, cx))),
            );
        let labels: Vec<&'static str> = ["Auto", "1", "2", "3", "4"].into_iter().take((n_disks + 1).min(5)).collect();
        let sel = if self.form.copies_auto { 0 } else { self.form.copies.min(labels.len() - 1) };
        let copies = kit::segmented("copies", &labels, sel, {
            let app = app.clone();
            move |ix, _, cx| {
                app.update(cx, |a, cx| {
                    a.form.copies_auto = ix == 0;
                    if ix > 0 {
                        a.form.copies = ix;
                    }
                    a.rebuild_plan();
                    cx.notify();
                })
            }
        });
        let free = self.form.req.data_free_percent;
        let controls = Form::vertical()
            .columns(2)
            .child(kit::field("f-mode", &copy::PLAN_MODE, &app, mode))
            .child(kit::field("f-size", &copy::PLAN_SYSTEM, &app, size_row))
            .child(kit::field("f-copies", &copy::PLAN_COPIES, &app, copies))
            .child(kit::field(
                "f-raid",
                &copy::PLAN_RAID,
                &app,
                div().w(px(300.0)).child(Select::new(&self.form.raid_select).accessibility_label(copy::PLAN_RAID.short)),
            ))
            .child(kit::field(
                "f-free",
                &copy::PLAN_FREE,
                &app,
                h_flex()
                    .gap(px(12.0))
                    .items_center()
                    .child(div().flex_1().child(Slider::new(&self.form.free_slider).accessibility_label(copy::PLAN_FREE.short)))
                    .child(kit::mono(cx, format!("{free} %")).w(px(48.0))),
            ));

        // ---- diagram (custom: no widget draws partition stacks) with its legend as a Tooltip
        let model = DiagramModel::from_plan(&plan);
        let legend = div()
            .id("diagram-legend")
            .child(icons::icon(role::TIP))
            .tooltip(|window, cx| Tooltip::new(copy::PLAN_LAYOUT_LEGEND.tip).build(window, cx));
        let diagram_box = GroupBox::new()
            .outline()
            .id("layout")
            .title(h_flex().gap(px(6.0)).items_center().child(kit::heading(copy::PLAN_LAYOUT_LEGEND.short)).child(legend))
            .child(diagram::render(&p, &model, cx));

        // ---- Destroyed / Changed / Preserved (names and sizes behind a HoverCard)
        let raw_total: u64 = f.candidate_disks().iter().map(|d| d.size_bytes()).sum();
        let summary_tag =
            |id: &'static str, tone: Tone, icon, item: &'static copy::Item, count: String, lines: Vec<String>| -> AnyElement {
                let p2 = p;
                HoverCard::new(id)
                    .trigger(div().id(id).child(kit::tag(&p, tone, Some(icon), format!("{} \u{00b7} {}", item.short, count))))
                    .content(move |_, _, _| {
                        let _ = &p2;
                        v_flex().p(px(10.0)).gap(px(4.0)).max_w(px(360.0)).text_sm().children(lines.iter().map(|l| div().child(l.clone())))
                    })
                    .into_any_element()
            };
        let changed_lines =
            if self.sys.boot.resolve(&f) == crate::ui::state::BootChoice::Dom { vec![copy::CONF_FLASH.short.to_string()] } else { vec![] };
        let summary = h_flex()
            .gap(px(8.0))
            .items_center()
            .flex_wrap()
            .child(summary_tag(
                "sum-destroyed",
                Tone::Err,
                role::DELETE,
                &copy::PLAN_DESTROYED,
                format!("{} \u{00b7} {}", plural(n_disks, "disk", "disks"), size_2dp(raw_total)),
                plan.destroyed.clone(),
            ))
            .child(summary_tag(
                "sum-changed",
                Tone::Warn,
                role::WARN,
                &copy::PLAN_CHANGED,
                if changed_lines.is_empty() { "none".into() } else { "boot flash".into() },
                changed_lines,
            ))
            .child(summary_tag(
                "sum-preserved",
                Tone::Neutral,
                role::SAFE,
                if keep { &copy::PLAN_PRESERVED_KEEP } else { &copy::PLAN_PRESERVED },
                if plan.preserved.is_empty() { "none".into() } else { plural(plan.preserved.len(), "volume", "volumes") },
                plan.preserved.iter().map(|d| format!("{} \u{00b7} {}", d.what, d.detail)).collect(),
            ));

        let surv = crate::ui::raid::survives(&plan);
        let hybrid = crate::ui::raid::is_hybrid(self.form.req.data_raid);
        let preview = h_flex()
            .gap(px(8.0))
            .items_center()
            .flex_wrap()
            .child(
                h_flex()
                    .gap(px(2.0))
                    .items_center()
                    .child(kit::tag(
                        &p,
                        Tone::Neutral,
                        None,
                        format!("{} \u{00b7} {}", copy::PLAN_USABLE.short, size_2dp(plan.sizes.data_usable_bytes)),
                    ))
                    .child(crate::ui::disclosure::info_tip("usable-info", &copy::PLAN_USABLE, &app)),
            )
            .when_some(surv, |d, n| {
                d.child(
                    h_flex()
                        .gap(px(2.0))
                        .items_center()
                        .child(kit::tag(
                            &p,
                            Tone::Neutral,
                            None,
                            format!("{} \u{00b7} {}", copy::PLAN_SURVIVES.short, crate::ui::raid::survives_text(n)),
                        ))
                        .child(crate::ui::disclosure::info_tip("survives-info", &copy::PLAN_SURVIVES, &app)),
                )
            })
            .when(hybrid, |d| {
                d.child(
                    Button::new("slices")
                        .link()
                        .small()
                        .label(copy::PLAN_SLICES.short)
                        .tooltip(copy::PLAN_SLICES.tip)
                        .on_click(cx.listener(|this, _, window, cx| this.open_slices_dialog(window, cx))),
                )
            })
            .when(hybrid, |d| {
                d.child(
                    h_flex()
                        .gap(px(2.0))
                        .items_center()
                        .child(kit::chip(&p, copy::PLAN_HYBRID_TIP.short))
                        .child(crate::ui::disclosure::info_tip("hybrid-info", &copy::PLAN_HYBRID_TIP, &app)),
                )
            });
        let tabs = self.setup_tabs(cx);
        let mut body = v_flex().w_full().gap(px(16.0)).child(tabs).child(controls).child(preview).child(diagram_box).child(summary);
        if let Some(shr) = &plan.shr {
            for (i, w) in shr.layout.warnings.iter().enumerate() {
                body = body
                    .child(Alert::warning(("shr-warn", i), w.clone()).title(copy::PLAN_WEAKER.short).small().icon(icons::icon(role::WARN)));
            }
        }
        for e in &plan.errors {
            body = body.child(Alert::error(("plan-err", e.len()), e.clone()).title(copy::PLAN_ERROR.short).icon(icons::icon(role::ERR)));
        }
        for (i, w) in plan.warnings.iter().enumerate() {
            body = body.child(Alert::warning(("plan-warn", i), w.clone()).icon(icons::icon(role::WARN)));
        }
        let ok = self.plan_ok();
        Page::new(copy::STEP_SETUP.short, body)
            .subtitle(copy::PLAN_SUB.short)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
                self.copy_command_button(cx),
            ])
            .right(vec![kit::with_reason(
                "next-why",
                Button::new("next")
                    .primary()
                    .icon(icons::icon(role::NEXT))
                    .label(copy::PLAN_NEXT.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::System, window, cx))),
                (!ok).then(|| plan.errors.first().cloned().unwrap_or_else(|| copy::PLAN_NEXT.tip.to_string())),
            )])
    }
}

impl App {
    /// The Storage | System tabs of the Setup step (stock `TabBar`).
    pub(crate) fn setup_tabs(&self, cx: &mut Context<App>) -> impl IntoElement {
        use gpui_kit::component::tab::{Tab, TabBar};
        TabBar::new("setup-tabs")
            .underline()
            .selected_index(usize::from(self.screen == Screen::System))
            .children([Tab::new().label(copy::SETUP_STORAGE.short), Tab::new().label(copy::SETUP_SYSTEM.short)])
            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                let target = if *ix == 1 { Screen::System } else { Screen::Plan };
                if this.screen != target {
                    this.go(target, window, cx);
                }
            }))
    }
}
