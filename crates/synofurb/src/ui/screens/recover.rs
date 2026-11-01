//! D14 Recovery: only what can work is enabled; every other row says why not (tooltip on the disabled button).

use crate::ui::app::{App, Job};
use crate::ui::copy::{self, Item};
use crate::ui::disclosure::{dialog, info_tip};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::Themed;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::table::{Table, TableBody, TableCell, TableRow};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, px};
use synofurb_core::model::Firmware;

impl App {
    pub(crate) fn page_recover(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let dom = self.facts.as_ref().is_some_and(|f| f.firmware == Firmware::Dom);
        let flash_editable = self.facts.as_ref().and_then(|f| f.dom.as_ref()).is_some_and(|d| d.is_editable());
        let disks_changed = self.install.past_no_return;
        let facts = h_flex()
            .gap(px(8.0))
            .flex_wrap()
            .child(kit::chip(&p, format!("Flash: {}", if flash_editable { "editable" } else { "none" })))
            .child(kit::chip(&p, format!("Disks: {}", if disks_changed { "modified" } else { "untouched" })));
        let running = self.recover.running();
        let no_dom = "This unit has no internal boot flash, so there is no boot setting to restore.";
        // (item, run handler enabled?, reason when disabled)
        struct Row {
            item: &'static Item,
            enabled: bool,
            reason: &'static str,
            id: &'static str,
        }
        let rows = [
            Row { item: &copy::REC_BOOT_DSM, enabled: dom && !running, reason: no_dom, id: "r-dsm" },
            Row { item: &copy::REC_OPEN_AGENT, enabled: false, reason: "Run `synofurb-cli boot-manager enter` for this.", id: "r-agent" },
            Row {
                item: &copy::REC_RESTORE_FLASH,
                enabled: false,
                reason: "Needs the saved image on the unit: run `synofurb-dom restore` there.",
                id: "r-flash",
            },
            Row {
                item: &copy::REC_REMOVE,
                enabled: false,
                reason: "This version has no remove step; restore the boot-flash backup instead.",
                id: "r-remove",
            },
            Row { item: &copy::REC_REINSTALL, enabled: true, reason: "", id: "r-reinstall" },
        ];
        let first_enabled = rows.iter().position(|r| r.enabled);
        let list = Table::new().child(TableBody::new().children(rows.iter().enumerate().map(|(i, r)| {
            let id = r.id;
            let mut b =
                Button::new(id).small().label(copy::REC_RUN.short).disabled(!r.enabled).tooltip(if r.enabled { "" } else { r.reason });
            b = if Some(i) == first_enabled { b.primary() } else { b.outline() };
            if i == 0 {
                b = b.loading(running).on_click(cx.listener(|this, _, _, cx| this.start_recover(cx)));
            } else if i == 4 {
                b = b.on_click(cx.listener(|this, _, window, cx| {
                    let app = cx.entity();
                    dialog(window, cx, &copy::DLG_REINSTALL, &app);
                    let _ = this;
                }));
            }
            TableRow::new()
                .child(TableCell::new().child(h_flex().gap(px(2.0)).items_center().child(kit::strong(r.item.short)).child(info_tip(
                    ("rec-i", i),
                    r.item,
                    &app,
                ))))
                .child(TableCell::new().text_right().child(b))
        })));
        let status: gpui_kit::AnyElement = match &self.recover {
            Job::Done(Ok(s)) => Alert::success("rec-ok", s.trim().to_string()).small().icon(icons::icon(role::OK)).into_any_element(),
            Job::Done(Err(e)) => Alert::error("rec-err", e.clone()).small().icon(icons::icon(role::ERR)).into_any_element(),
            _ => gpui_kit::div().into_any_element(),
        };
        let backups = self.cfg_dir.join("dom-backups").display().to_string();
        let logs = self.cfg_dir.join("logs");
        let saved = v_flex().gap(px(6.0)).child(kit::mono_sm(cx, backups)).child(kit::mono_sm(cx, logs.display().to_string())).child(
            h_flex().child(
                Button::new("open-folder").outline().small().icon(icons::icon(role::FOLDER)).label(copy::REC_OPEN_FOLDER.short).on_click(
                    move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&logs);
                        cx.reveal_path(&logs)
                    },
                ),
            ),
        );
        let acc = Accordion::new("rec-acc")
            .bordered(true)
            .item(|i| i.title(h_flex().gap(px(2.0)).items_center().child(copy::REC_SAVED.short)).child(saved));
        let body = v_flex().w_full().max_w(px(640.0)).gap(px(14.0)).child(facts).child(list).child(status).child(acc);
        Page::new(copy::REC_TITLE.short, body).subtitle(copy::REC_SUB.short).left(vec![
            Button::new("back")
                .ghost()
                .icon(icons::icon(role::BACK))
                .label(copy::BTN_BACK.short)
                .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                .into_any_element(),
        ])
    }
}
