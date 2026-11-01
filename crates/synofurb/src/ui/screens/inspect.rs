//! Step 3: the inspection result and the compatibility panel.

use crate::ui::app::App;
use crate::ui::format::{plural, size_2dp};
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{self, Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, IntoElement, Window, div, px};
use synofurb_core::model::{ArrayRole, Firmware, SECTOR, human_bytes};
use synofurb_core::platform;

impl App {
    pub(crate) fn page_inspect(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let Some(f) = self.facts.clone() else {
            return Page::new("Inspection result", muted(&p, "No unit has been inspected yet.")).buttons(vec![
                btn_secondary("back", "Back")
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Connect, window, cx)))
                    .into_any_element(),
            ]);
        };
        let info = platform::classify(&f.unique, &f.uname_m);
        let verdict = self.compat_for_facts(&f);
        let arch_tag = match f.arch {
            synofurb_core::model::Arch::X86_64 => "x86_64",
            synofurb_core::model::Arch::Arm64 => "arm64",
            synofurb_core::model::Arch::Armhf => "armv7",
            synofurb_core::model::Arch::Unsupported => "unsupported",
        };
        let name = if f.model.is_empty() { f.hostname.clone() } else { f.model.clone() };

        // --- compatibility panel
        let (tone, _, _) = status_style(verdict.status);
        let mut compat_body = v_flex().gap(px(space::S8)).child(muted(&p, verdict.status.explain()));
        for d in &verdict.details {
            compat_body = compat_body.child(
                h_flex()
                    .gap(px(space::S8))
                    .child(icon_sized("circle-dot", 10.0, p.placeholder).mt(px(6.0)))
                    .child(muted(&p, d.clone()).flex_1().min_w_0()),
            );
        }
        let compat_card = card(
            &p,
            Some("Compatibility".into()),
            Some(compat_badge(&p, verdict.status).into_any_element()),
            Some(p.tone(tone).solid),
            v_flex()
                .gap(px(space::S8))
                .child(text(&p, theme::BODY, verdict.headline.clone()).font_weight(FontWeight(600.0)))
                .child(compat_body),
        );

        // --- hardware
        let serial = f.serial_console();
        let serial_text = match &serial.raw {
            Some(raw) => format!("{raw} (raw MMIO console: AMD boards have no legacy UART)"),
            None => format!("{} at {} baud", serial.tty, serial.speed),
        };
        let boot_text = match f.firmware {
            Firmware::Dom => match &f.dom {
                Some(d) => format!(
                    "{} - {} flash module, {}",
                    f.firmware.label(),
                    human_bytes(d.sectors * SECTOR),
                    if d.is_editable() { "editable" } else { "not editable" }
                ),
                None => f.firmware.label().to_string(),
            },
            other => other.label().to_string(),
        };
        let hw = v_flex()
            .child(kv(&p, "Model", text(&p, theme::BODY, format!("{}  ({})", name, f.unique)).font_weight(FontWeight(600.0))))
            .child(kv(&p, "Platform", cell_text(&p, format!("{} - {}", info.platform, info.soc))))
            .child(kv(&p, "CPU", cell_text(&p, f.cpu.clone())))
            .child(kv(&p, "Memory", cell_text(&p, format!("{:.1} GiB RAM", f.mem_gib()))))
            .child(kv(&p, "DSM", cell_text(&p, format!("{}  (kernel {})", f.dsm_version, f.kernel))))
            .child(kv(&p, "Firmware", text(&p, theme::BODY_SM, boot_text).font_weight(FontWeight(600.0)).into_any_element()))
            .child(kv(&p, "kexec", cell_text(&p, if f.kexec { "supported by the DSM kernel" } else { "not supported by the DSM kernel" })))
            .child(kv(&p, "Serial console", mono(&p, serial_text, cx).into_any_element()))
            .child(kv(
                &p,
                "Network",
                cell_text(&p, f.nics.iter().map(|n| format!("{} {}", n.name, n.mac)).collect::<Vec<_>>().join(",  ")),
            ));
        let chips = h_flex()
            .gap(px(space::S8))
            .items_center()
            .child(vendor_chip(&p, f.cpu_vendor.label(), arch_tag, cx))
            .child(compat_badge(&p, verdict.status))
            .child(chip(&p, plural(f.candidate_disks().len(), "disk", "disks")))
            .child(chip(&p, format!("{:.0} GiB RAM", f.mem_gib())));

        // --- disks
        let cols = [
            Col::new("Disk", 90.0),
            Col::new("Model", 0.0),
            Col::new("Serial", 150.0),
            Col::new("Type", 70.0),
            Col::new("Table", 70.0),
            Col::new("Size", 100.0).right(),
            Col::new("Use", 190.0),
        ];
        let rows = f
            .disks
            .iter()
            .map(|d| {
                let parts = d
                    .parts
                    .iter()
                    .filter_map(|pt| f.arrays.iter().find(|a| a.members.contains(&pt.name)).map(|a| a.name.clone()))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ");
                let usage = if d.is_candidate() {
                    format!("install target{}", if parts.is_empty() { String::new() } else { format!(" (in {parts})") })
                } else {
                    "not an install target".to_string()
                };
                (
                    if d.is_candidate() { None } else { Some(p.border_strong) },
                    vec![
                        mono(&p, d.name.clone(), cx).into_any_element(),
                        cell_text(&p, d.model.clone()),
                        mono(&p, d.serial.clone(), cx).text_size(theme::CAPTION.size()).into_any_element(),
                        cell_text(&p, if d.rotational { "HDD" } else { "SSD" }),
                        cell_text(&p, format!("{:?}", d.table).to_uppercase()),
                        mono(&p, size_2dp(d.size_bytes()), cx).into_any_element(),
                        muted(&p, usage).text_size(theme::CAPTION.size()).into_any_element(),
                    ],
                )
            })
            .collect();
        let disks = data_table(&p, &cols, rows);

        // --- DSM layout
        let acols = [
            Col::new("Array", 90.0),
            Col::new("Level", 80.0),
            Col::new("Role", 0.0),
            Col::new("Size", 110.0).right(),
            Col::new("Members", 220.0),
        ];
        let arows = f
            .arrays
            .iter()
            .map(|a| {
                let role = match a.role {
                    ArrayRole::DsmSystem => "DSM system (reclaimed by the install)".to_string(),
                    ArrayRole::DsmSwap => "DSM swap (reclaimed)".to_string(),
                    ArrayRole::Data => format!("data volume{}", a.vg.as_ref().map(|v| format!(", LVM {v}")).unwrap_or_default()),
                };
                (
                    None,
                    vec![
                        mono(&p, a.name.clone(), cx).into_any_element(),
                        cell_text(&p, a.level.to_uppercase()),
                        cell_text(&p, role),
                        mono(&p, size_2dp(a.sectors * SECTOR), cx).into_any_element(),
                        mono(&p, a.members.join(" "), cx).text_size(theme::CAPTION.size()).into_any_element(),
                    ],
                )
            })
            .collect();
        let layout = data_table(&p, &acols, arows);

        let mut body = v_flex().w_full().gap(px(space::S16)).child(chips);
        if verdict.blocking {
            body = body.child(banner(
                &p,
                Tone::Err,
                format!("{name} cannot be converted"),
                format!("{} You can still use Restore and recover for this unit.", info.note),
                vec![],
            ));
        }
        body = body
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap(px(space::S16))
                    .child(div().flex_1().min_w_0().child(compat_card))
                    .child(div().flex_1().min_w_0().child(card(&p, Some("Hardware".into()), None, None, hw))),
            )
            .child(section(&p, format!("Disks ({})", f.disks.len())))
            .child(disks)
            .child(section(&p, "Current DSM layout"))
            .child(layout);

        Page::new(format!("{name}: inspection result"), body)
            .subtitle(format!(
                "Read-only. {} at {}. Nothing on the NAS has been changed.",
                name,
                super::super::format::display_host(&self.input_value(&self.inp.host, cx))
            ))
            .note(muted(&p, "This is everything synofurb knows about the unit before you choose a goal."))
            .buttons(vec![
                btn_secondary("back", "Back")
                    .icon(icon("arrow-left"))
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Connect, window, cx)))
                    .into_any_element(),
                btn_primary("next", "Choose a goal")
                    .icon(icon("arrow-right"))
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
            ])
    }
}
