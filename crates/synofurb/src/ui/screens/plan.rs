//! Step 5: the storage plan. Every control recomputes the plan and redraws the diagram; warnings and errors are
//! repeated as banners so the picture is never the only carrier of a message.

use crate::ui::app::App;
use crate::ui::diagram::{self, DiagramModel};
use crate::ui::format::{gib_from_mib, size_2dp};
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{self, Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::select::Select;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, IntoElement, Window, div, px};
use synofurb_core::config::KernelChannel;
use synofurb_core::plan::StorageMode;

impl App {
    pub(crate) fn page_plan(&mut self, window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        self.rebuild_plan();
        self.sync_system_slider(cx);
        let (Some(f), Some(plan)) = (self.facts.clone(), self.form.plan.clone()) else {
            return Page::new("Storage plan", muted(&p, "Inspect a unit first.")).buttons(vec![
                btn_secondary("back", "Back")
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
            ]);
        };
        let _ = window;
        let name = self.unit_name();
        let n_disks = f.candidate_disks().len();

        // ---- mode
        let wipe = choice_card(
            &p,
            "mode-wipe",
            self.form.req.mode == StorageMode::WipeAll,
            false,
            "Wipe everything",
            format!("New partition tables on all {n_disks} disks. All data currently on them, including every DSM volume, is destroyed."),
            cx.listener(|this, _, _, cx| {
                this.form.req.mode = StorageMode::WipeAll;
                this.rebuild_plan();
                cx.notify();
            }),
        );
        let keep = choice_card(
            &p,
            "mode-keep",
            self.form.req.mode == StorageMode::KeepVolumes,
            false,
            "Keep existing data volumes (best effort)",
            "Reuses only DSM's system and swap area (about 10 GiB per disk) for Debian and leaves the data arrays you tick below in place. There is no guarantee they survive.",
            cx.listener(|this, _, _, cx| {
                this.form.req.mode = StorageMode::KeepVolumes;
                this.rebuild_plan();
                cx.notify();
            }),
        );
        let mut mode = v_flex().w_full().gap(px(space::S8)).child(
            h_flex()
                .w_full()
                .items_stretch()
                .gap(px(space::S12))
                .child(div().flex_1().min_w_0().child(wipe))
                .child(div().flex_1().min_w_0().child(keep)),
        );
        if self.form.req.mode == StorageMode::KeepVolumes {
            let arrays: Vec<_> = f.data_arrays().into_iter().cloned().collect();
            let mut keep_list = v_flex().gap(px(space::S6)).pl(px(space::S12));
            if arrays.is_empty() {
                keep_list = keep_list.child(muted(&p, "No data arrays were found on this unit."));
            }
            for (i, a) in arrays.iter().enumerate() {
                let on = self.form.keep.contains(&a.name);
                let aname = a.name.clone();
                keep_list = keep_list.child(
                    Checkbox::new(("keep", i))
                        .checked(on)
                        .label(format!(
                            "Preserve {} - {} {}{}",
                            a.name,
                            a.level.to_uppercase(),
                            size_2dp(a.sectors * synofurb_core::model::SECTOR),
                            a.vg.as_ref().map(|v| format!(", LVM {v}")).unwrap_or_default()
                        ))
                        .on_click(cx.listener(move |this, v: &bool, _, cx| {
                            if *v {
                                this.form.keep.insert(aname.clone());
                            } else {
                                this.form.keep.remove(&aname);
                            }
                            this.rebuild_plan();
                            cx.notify();
                        })),
                );
            }
            keep_list = keep_list.child(muted(&p, "Preserved volumes are not mounted automatically; the management plane lists them under 'Import volumes' and mounts them read-only.").text_size(theme::CAPTION.size()));
            mode = mode.child(keep_list);
        }

        // ---- system card
        let (min, max, rec) = (
            plan.sizes.min_system_mib as f32 / 1024.0,
            plan.sizes.max_system_mib as f32 / 1024.0,
            plan.sizes.recommended_system_mib as f32 / 1024.0,
        );
        let size_ok = max >= min;
        let chosen = self.form.system_gib;
        let system_body = v_flex()
            .gap(px(space::S12))
            .child(
                h_flex()
                    .gap(px(space::S12))
                    .items_center()
                    .child(
                        text(&p, theme::DISPLAY, format!("{chosen:.0}"))
                            .text_size(theme::H1.size())
                            .child(muted(&p, " GiB"))
                            .into_any_element(),
                    )
                    .child(div().flex_1())
                    .child(
                        btn_secondary("rec-size", format!("Recommended: {rec:.0} GiB"))
                            .small()
                            .disabled(!size_ok)
                            .on_click(cx.listener(move |this, _, window, cx| this.set_system_gib(rec, window, cx))),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .py(px(space::S4))
                    .child(Slider::new(&self.form.system_slider).disabled(!size_ok).accessibility_label("System size in GiB")),
            )
            .child(
                h_flex()
                    .justify_between()
                    .child(muted(&p, format!("min {min:.0} GiB")).text_size(theme::CAPTION.size()))
                    .child(muted(&p, format!("recommended {rec:.0} GiB")).text_size(theme::CAPTION.size()))
                    .child(muted(&p, format!("max {max:.0} GiB")).text_size(theme::CAPTION.size())),
            )
            .child(muted(
                &p,
                format!(
                    "Root gets {} (xfs), swap {} MiB, the rest stays free in the system volume group.",
                    gib_from_mib(plan.sizes.root_mib),
                    plan.sizes.swap_mib
                ),
            ))
            .child(divider(&p))
            .child(
                h_flex()
                    .gap(px(space::S12))
                    .items_center()
                    .child(text(&p, theme::BODY_SM, "System copies (RAID 1 mirror)").font_weight(FontWeight(600.0)))
                    .child(Checkbox::new("copies-auto").label("automatic").checked(self.form.copies_auto).on_click(cx.listener(
                        |this, v: &bool, _, cx| {
                            this.form.copies_auto = *v;
                            this.rebuild_plan();
                            cx.notify();
                        },
                    ))),
            )
            .when(!self.form.copies_auto, |d| {
                d.child(
                    h_flex()
                        .gap(px(space::S12))
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Slider::new(&self.form.copies_slider).accessibility_label("Number of system copies")),
                        )
                        .child(text(&p, theme::BODY, format!("{} of {n_disks}", self.form.copies))),
                )
            })
            .when(n_disks > 2, |d| {
                d.child(
                    muted(&p, "With more than two disks the system area is always mirrored (at least 2 copies).")
                        .text_size(theme::CAPTION.size()),
                )
            });

        // ---- data card
        let free = self.form.req.data_free_percent;
        let data_body = v_flex()
            .gap(px(space::S12))
            .child(field(
                &p,
                "Data RAID",
                Some("Automatic picks the best level the disks allow."),
                div().w(px(280.0)).child(Select::new(&self.form.raid_select).accessibility_label("Data RAID level")),
            ))
            .child(field(
                &p,
                &format!("Leave {free}% of the data volume group free"),
                Some("Free space can later hold snapshots or new volumes."),
                div()
                    .w_full()
                    .py(px(space::S4))
                    .child(Slider::new(&self.form.free_slider).accessibility_label("Percent of the data volume group left free")),
            ))
            .child(
                h_flex()
                    .gap(px(space::S8))
                    .child(muted(&p, "Raw"))
                    .child(text(&p, theme::BODY_SM, size_2dp(plan.sizes.data_raw_bytes)).font_weight(FontWeight(600.0)))
                    .child(muted(&p, "Usable"))
                    .child(text(&p, theme::BODY_SM, size_2dp(plan.sizes.data_usable_bytes)).font_weight(FontWeight(600.0))),
            );

        // ---- kernel
        let kernel = h_flex()
            .w_full()
            .gap(px(space::S12))
            .child(div().flex_1().min_w_0().child(choice_card(
                &p,
                "kernel-newest",
                self.form.kernel == KernelChannel::Newest,
                false,
                "Newest (Linux 7.x)",
                "From trixie-backports. Best hardware support; updates arrive through Debian.",
                cx.listener(|this, _, _, cx| {
                    this.form.kernel = KernelChannel::Newest;
                    cx.notify();
                }),
            )))
            .child(div().flex_1().min_w_0().child(choice_card(
                &p,
                "kernel-stable",
                self.form.kernel == KernelChannel::Stable,
                false,
                "Stable (6.12 LTS)",
                "Debian 13's own kernel. Fewer changes between updates.",
                cx.listener(|this, _, _, cx| {
                    this.form.kernel = KernelChannel::Stable;
                    cx.notify();
                }),
            )));

        // ---- diagram and messages
        let model = DiagramModel::from_plan(&plan);
        let diagram = card(
            &p,
            Some("Resulting layout".into()),
            Some(chip(&p, format!("{} disks", model.disks.len())).into_any_element()),
            None,
            diagram::render(&p, &model, cx),
        );
        let mut notes = v_flex().w_full().gap(px(space::S8));
        for e in &plan.errors {
            notes = notes.child(
                banner(&p, Tone::Err, "Blocking: this plan cannot be installed", e.clone(), vec![])
                    .id(("plan-err", e.len()))
                    .role(gpui_kit::Role::Alert),
            );
        }
        for w in &plan.warnings {
            notes = notes.child(banner(&p, Tone::Warn, "Warning", w.clone(), vec![]));
        }
        let summary = v_flex().gap(px(space::S4)).children(plan.summary().into_iter().map(|l| {
            h_flex()
                .gap(px(space::S8))
                .child(icon_sized("chevron-right", 12.0, p.placeholder).mt(px(5.0)))
                .child(muted(&p, l).flex_1().min_w_0())
        }));

        let body = v_flex()
            .w_full()
            .gap(px(space::S16))
            .child(mode)
            .child(diagram)
            .child(card(&p, Some("In plain words".into()), None, None, summary))
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap(px(space::S16))
                    .child(div().flex_1().min_w_0().child(card(&p, Some("System".into()), None, None, system_body)))
                    .child(div().flex_1().min_w_0().child(card(&p, Some("Data".into()), None, None, data_body))),
            )
            .child(section(&p, "Linux kernel"))
            .child(kernel)
            .child(notes);

        let ok = self.plan_ok();
        let (errs, warns) = (plan.errors.len(), plan.warnings.len());
        let note = h_flex()
            .gap(px(space::S8))
            .items_center()
            .child(icon_sized(if ok { "circle-check" } else { "circle-x" }, 16.0, if ok { p.ok_fg } else { p.err_fg }))
            .child(
                text(
                    &p,
                    theme::BODY_SM,
                    if ok {
                        format!(
                            "Plan is valid{}. Nothing is written yet.",
                            if warns > 0 { format!(" with {warns} warning(s)") } else { String::new() }
                        )
                    } else {
                        format!("{errs} error(s) must be fixed to continue.")
                    },
                )
                .text_color(if ok { p.ok_fg } else { p.err_fg }),
            );
        Page::new("Storage plan", body)
            .subtitle(format!("{name}: {n_disks} disks -> RAID -> LVM -> xfs. Adjust the choices; the diagram and checks update live. Nothing is written yet."))
            .note(note)
            .buttons(vec![
                btn_secondary("back", "Back").icon(icon("arrow-left")).on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx))).into_any_element(),
                btn_primary("next", "Next: system settings")
                    .icon(icon("arrow-right"))
                    .disabled(!ok)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::System, window, cx)))
                    .into_any_element(),
            ])
    }
}
