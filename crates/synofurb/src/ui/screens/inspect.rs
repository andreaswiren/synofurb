//! D3 Inspect: what was read from the unit. A DescriptionList for the facts, a Table for the disks, two collapsed
//! Accordion items for DSM's volumes and layout.

use crate::ui::app::App;
use crate::ui::copy::{self, Item};
use crate::ui::disclosure::info_tip;
use crate::ui::format::size_2dp;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::Themed;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Entity, IntoElement, Window, div, px};
use synofurb_core::model::{ArrayRole, Firmware, SECTOR, human_bytes};

fn label(id: &'static str, item: &'static Item, app: &Entity<App>) -> AnyElement {
    kit::labelled(id, item, app)
}

impl App {
    pub(crate) fn page_inspect(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let Some(f) = self.facts.clone() else {
            return Page::new(copy::INSPECT_TITLE.short, kit::muted(cx, "-")).left(vec![
                Button::new("back")
                    .ghost()
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Connect, window, cx)))
                    .into_any_element(),
            ]);
        };
        let verdict = self.compat_for_facts(&f);
        let name = if f.model.is_empty() { f.hostname.clone() } else { f.model.clone() };
        let serial = f.serial_console();
        let console = match &serial.raw {
            Some(raw) => raw.clone(),
            None => format!("{} {}", serial.tty, serial.speed),
        };
        let flash = match f.firmware {
            Firmware::Dom => match &f.dom {
                Some(d) => {
                    format!("{} \u{00b7} {}", human_bytes(d.sectors * SECTOR), if d.is_editable() { "editable" } else { "not editable" })
                }
                None => "-".into(),
            },
            _ => "none".into(),
        };
        let compat_row = h_flex()
            .gap(px(2.0))
            .items_center()
            .child(kit::compat_tag(&p, verdict.status))
            .child(info_tip("compat-info", &copy::INSPECT_COMPAT, &app))
            .into_any_element();
        let pin = self.host_pin(cx);
        let facts = DescriptionList::horizontal()
            .columns(2)
            .bordered(false)
            .child(
                gpui_kit::component::description_list::DescriptionItem::new(label("l-model", &copy::INSPECT_MODEL, &app))
                    .value(kit::strong(format!("{name} ({})", f.unique)).into_any_element()),
            )
            .child(
                gpui_kit::component::description_list::DescriptionItem::new(label("l-comp", &copy::INSPECT_COMPAT, &app)).value(compat_row),
            )
            .item(copy::INSPECT_CPU.short, f.cpu.clone(), 1)
            .item(copy::INSPECT_RAM.short, format!("{:.1} GiB", f.mem_gib()), 1)
            .item(copy::INSPECT_FIRMWARE.short, f.firmware.label().to_string(), 1)
            .child(gpui_kit::component::description_list::DescriptionItem::new(label("l-flash", &copy::INSPECT_FLASH, &app)).value(flash))
            .child(
                gpui_kit::component::description_list::DescriptionItem::new(label("l-console", &copy::INSPECT_CONSOLE, &app))
                    .value(kit::mono_sm(cx, console).into_any_element()),
            )
            .child(
                gpui_kit::component::description_list::DescriptionItem::new(label("l-backups", &copy::INSPECT_BACKUPS, &app))
                    .value(kit::chip(&p, "not checked").into_any_element()),
            )
            .child(
                gpui_kit::component::description_list::DescriptionItem::new(label("l-hostkey", &copy::HK_PIN, &app)).value(match pin {
                    Some(fp) => h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .child(kit::tag(&p, crate::ui::theme::Tone::Ok, Some(role::LOCK), copy::HK_PINNED_TAG.short))
                        .child(kit::mono_sm(cx, fp.clone()).min_w_0().truncate())
                        .child(gpui_kit::component::clipboard::Clipboard::new("hk-pin-copy").value(fp).tooltip("Copy"))
                        .into_any_element(),
                    None => kit::chip(&p, copy::HK_NOT_PINNED.short).into_any_element(),
                }),
            );

        let disks = Table::new()
            .child(
                TableHeader::new().child(
                    TableRow::new()
                        .child(TableHead::new().child(copy::INSPECT_DISK.short))
                        .child(TableHead::new().child(copy::INSPECT_COL_MODEL.short))
                        .child(TableHead::new().child(copy::INSPECT_COL_SERIAL.short))
                        .child(TableHead::new().child(copy::INSPECT_COL_USE.short))
                        .child(TableHead::new().text_right().child(copy::INSPECT_COL_SIZE.short)),
                ),
            )
            .child(TableBody::new().children(f.disks.iter().map(|d| {
                // every disk says whether it is an install target, or why it is not (never silently left out)
                let use_text = match d.exclusion() {
                    None => "Install target".to_string(),
                    Some(e) => format!("Not used: {}", e.reason().to_lowercase()),
                };
                TableRow::new()
                    .child(TableCell::new().child(kit::mono(cx, d.name.clone())))
                    .child(TableCell::new().child(d.model.clone()))
                    .child(TableCell::new().child(kit::mono_sm(cx, d.serial.clone())))
                    .child(TableCell::new().child(use_text))
                    .child(TableCell::new().text_right().child(kit::mono(cx, size_2dp(d.size_bytes()))))
            })));

        // DSM's volumes as the user knows them: /volume1, /volume2 with filesystem, size, use and what backs them.
        let vols = f.volumes();
        let volumes = if vols.is_empty() {
            div().child(copy::INSPECT_NO_VOLUMES.short).into_any_element()
        } else {
            Table::new()
                .child(
                    TableHeader::new().child(
                        TableRow::new()
                            .child(TableHead::new().child(copy::INSPECT_COL_VOLUME.short))
                            .child(TableHead::new().child(copy::INSPECT_COL_FS.short))
                            .child(TableHead::new().text_right().child(copy::INSPECT_COL_SIZE.short))
                            .child(TableHead::new().text_right().child(copy::INSPECT_COL_USED.short))
                            .child(TableHead::new().child(copy::INSPECT_COL_BACKED.short)),
                    ),
                )
                .child(TableBody::new().children(vols.iter().map(|v| {
                    let levels = v.levels.iter().map(|l| l.to_uppercase()).collect::<Vec<_>>().join(" + ");
                    let backed = if v.disks.is_empty() {
                        "unknown".to_string()
                    } else {
                        format!("{} \u{00b7} {}", if levels.is_empty() { "-".to_string() } else { levels }, v.disks.join(" "))
                    };
                    TableRow::new()
                        .child(TableCell::new().child(kit::strong(v.name.clone())))
                        .child(TableCell::new().child(v.fstype.clone()))
                        .child(TableCell::new().text_right().child(kit::mono(cx, size_2dp(v.size_bytes()))))
                        .child(
                            TableCell::new()
                                .text_right()
                                .child(kit::mono(cx, format!("{} ({}%)", size_2dp(v.used_bytes()), v.used_percent()))),
                        )
                        .child(TableCell::new().child(backed))
                })))
                .into_any_element()
        };

        let arrays = Table::new()
            .child(
                TableHeader::new().child(
                    TableRow::new()
                        .child(TableHead::new().child("md"))
                        .child(TableHead::new().child("RAID"))
                        .child(TableHead::new().child(copy::INSPECT_COL_USE.short))
                        .child(TableHead::new().child(copy::INSPECT_COL_STATUS.short))
                        .child(TableHead::new().text_right().child(copy::INSPECT_COL_SIZE.short)),
                ),
            )
            .child(TableBody::new().children(f.arrays.iter().map(|a| {
                let role_text = match a.role {
                    ArrayRole::DsmSystem => "DSM system".to_string(),
                    ArrayRole::DsmSwap => "DSM swap".to_string(),
                    ArrayRole::Data => format!("data{}", a.vg.as_ref().map(|v| format!(" \u{00b7} LVM {v}")).unwrap_or_default()),
                };
                TableRow::new()
                    .child(TableCell::new().child(kit::mono(cx, a.name.clone())))
                    .child(TableCell::new().child(a.level.to_uppercase()))
                    .child(TableCell::new().child(role_text))
                    .child(TableCell::new().child(a.health_note().unwrap_or_else(|| "OK".to_string())))
                    .child(TableCell::new().text_right().child(kit::mono(cx, size_2dp(a.sectors * SECTOR))))
            })));
        let layout = v_flex().gap(px(4.0)).children(f.disks.iter().map(|d| {
            let parts =
                d.parts.iter().map(|pt| format!("{} {}", pt.name, size_2dp(pt.sectors * SECTOR))).collect::<Vec<_>>().join("  \u{00b7}  ");
            h_flex().gap(px(12.0)).child(kit::mono_sm(cx, d.name.clone()).w(px(48.0))).child(kit::mono_sm(cx, parts))
        }));
        let accordion = Accordion::new("inspect-acc")
            .multiple(true)
            .bordered(true)
            .item(|i| i.open(true).title(h_flex().gap(px(2.0)).items_center().child(copy::INSPECT_VOLUMES.short)).child(volumes))
            .item(|i| i.title(copy::INSPECT_ARRAYS.short).child(arrays))
            .item(|i| i.title(copy::INSPECT_LAYOUT.short).child(layout));

        let mut body = v_flex().w_full().gap(px(16.0));
        if verdict.blocking {
            body = body
                .child(Alert::error("blocked", verdict.headline.clone()).title(copy::INSPECT_BLOCKED.short).icon(icons::icon(role::ERR)));
        }
        if f.candidate_disks().is_empty() {
            body = body.child(Alert::error("no-disks", "No disks visible").icon(icons::icon(role::ERR)));
        }
        for a in f.busy_arrays() {
            if let Some(note) = a.health_note() {
                body = body.child(
                    Alert::warning("busy-array", format!("{} is {note}. Let it finish before installing.", a.name))
                        .icon(icons::icon(role::WARN)),
                );
            }
        }
        body = body.child(facts).child(disks).child(accordion);
        let blocked = verdict.blocking || f.candidate_disks().is_empty();
        Page::new(copy::INSPECT_TITLE.short, body)
            .subtitle(copy::INSPECT_SUB.short)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Connect, window, cx)))
                    .into_any_element(),
            ])
            .right(vec![
                Button::new("next")
                    .primary()
                    .icon(icons::icon(role::NEXT))
                    .label(copy::INSPECT_CHOOSE.short)
                    .disabled(blocked)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
            ])
    }
}
