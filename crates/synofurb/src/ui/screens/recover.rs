//! Restore and recover. Only honest options: each says whether it is possible and why not when it is not.

use crate::ui::app::{App, Job};
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{self, Themed, Tone, radius, space};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, IntoElement, Role, Window, px};

impl App {
    pub(crate) fn page_recover(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let name = self.unit_name();
        let dom = self.facts.as_ref().is_some_and(|f| f.firmware == synofurb_core::model::Firmware::Dom);
        let dom_ok: Result<(), &str> =
            if dom { Ok(()) } else { Err("this unit has no internal boot flash (DOM): there is no boot setting to restore from here") };
        let option = |title: &str, detail: &str, avail: Result<(), &str>| {
            h_flex()
                .w_full()
                .gap(px(space::S12))
                .items_start()
                .p(px(space::S12))
                .rounded(px(radius::MD))
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                .child(
                    icon_sized(
                        if avail.is_ok() { "circle-check" } else { "ban" },
                        18.0,
                        if avail.is_ok() { p.ok_fg } else { p.placeholder },
                    )
                    .mt(px(2.0)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(space::S2))
                        .child(text(&p, theme::BODY, title.to_string()).font_weight(FontWeight(600.0)).text_color(if avail.is_ok() {
                            p.fg
                        } else {
                            p.placeholder
                        }))
                        .child(muted(&p, detail.to_string()))
                        .when_some(avail.err(), |d, why| {
                            d.child(text(&p, theme::CAPTION, format!("Not available: {why}")).text_color(p.fg_2))
                        }),
                )
        };

        let status: gpui_kit::AnyElement = match &self.recover {
            Job::Idle => muted(&p, "Nothing has been changed yet.").into_any_element(),
            Job::Running => banner(&p, Tone::Info, "Restoring...", "Changing the boot setting over SSH. Do not power off the NAS.", vec![]).into_any_element(),
            Job::Done(Ok(s)) => banner(&p, Tone::Ok, "The boot setting points at DSM again", format!("{} Reboot the unit to return to DSM. If DSM's system partition is gone (a failed install), DSM starts in its recovery mode: open https://find.synology.com and choose to reinstall DSM; Synology keeps data volumes on a reinstall.", s.trim()), vec![]).into_any_element(),
            Job::Done(Err(e)) => banner(&p, Tone::Err, "Could not restore", e.clone(), vec![]).id("recover-err").role(Role::Alert).into_any_element(),
        };

        let body = v_flex()
            .w_full()
            .gap(px(space::S12))
            .child(option("Return to original DSM", "Sets the unit's default boot entry back to DSM (boot-flash units). Works from DSM, from the installed Debian or from the boot agent. Uses the connection from step 2. If DSM's operating system was already removed by an install, only its loader runs: see the reinstall steps after you restore.", dom_ok))
            .child(option("Open a rescue shell", "Opens the rescue shell the installer keeps after a failure.", Err("use the NAS console (VGA or serial) after a failed install; the app cannot attach to it yet")))
            .child(option("Restore disk headers", "Writes back the first sectors of every disk saved before an install.", Err("the metadata backup is saved on this computer before every install; restoring it is done by hand with dd, see the README next to the files")))
            .child(option("Remove the boot manager", "Takes the boot agent out of the internal boot flash again.", Err("this version has no remove step; restore the boot-flash backup from the dom-backups folder")))
            .child(status);

        let running = self.recover.running();
        Page::new("Restore and recover", body)
            .subtitle(format!("{name}. Only options that can work are enabled; the others say why not."))
            .note(muted(&p, "This changes the unit's boot setting only. It does not touch the disks."))
            .buttons(vec![
                btn_secondary("back", "Back")
                    .icon(icon("arrow-left"))
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
                Button::new("restore")
                    .primary()
                    .icon(icon("life-buoy"))
                    .label("Return to DSM")
                    .loading(running)
                    .disabled(running || !dom)
                    .on_click(cx.listener(|this, _, _, cx| this.start_recover(cx)))
                    .into_any_element(),
            ])
    }
}
