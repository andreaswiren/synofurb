//! Boot manager mode: add a small SSH-controllable boot environment to the unit's internal boot flash. Disks are
//! not touched; the flash is backed up to this computer first and DSM stays the fail-safe default.

use crate::ui::app::{App, Job};
use crate::ui::format::percent;
use crate::ui::render::Page;
use crate::ui::state::{DOM_FLOW, Screen};
use crate::ui::theme::{self, Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Role, Window, div, px};

impl App {
    pub(crate) fn page_bootmgr(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let name = self.unit_name();
        let avail = self.can_boot_manager();
        let touched = card(
            &p,
            Some("What changes, and what does not".into()),
            Some(badge(&p, Tone::Ok, "shield-check", "Disks untouched, reversible", false).into_any_element()),
            Some(p.ok_solid),
            v_flex()
                .gap(px(space::S6))
                .child(muted(&p, "Changes: the unit's internal boot flash gets a small boot agent as an extra entry. The whole flash is backed up to this computer first and checked against the NAS."))
                .child(muted(&p, "Does not change: your disks, your volumes and DSM. DSM remains the default boot entry; the agent resets the default to DSM itself as its first action."))
                .child(muted(&p, "Afterwards you control the bootloader over SSH and can netboot or kexec any OS, always with DSM as the way back.")),
        );
        let flow = card(
            &p,
            Some("What happens, in order".into()),
            None,
            None,
            v_flex().gap(px(space::S6)).children(DOM_FLOW.iter().enumerate().map(|(i, s)| {
                h_flex()
                    .gap(px(space::S8))
                    .child(text(&p, theme::CAPTION, format!("{}", i + 1)).text_color(p.accent_fg).w(px(14.0)).flex_none())
                    .child(muted(&p, *s).flex_1().min_w_0())
            })),
        );
        let state: gpui_kit::AnyElement = match &self.bootmgr {
            Job::Idle => muted(&p, "Nothing has been changed yet.").into_any_element(),
            Job::Running => {
                let (what, f) = self.install.progress.clone().unwrap_or(("Working...".into(), 0.0));
                v_flex().gap(px(space::S4)).child(muted(&p, what)).child(h_flex().gap(px(space::S8)).items_center().child(div().flex_1().min_w_0().child(Progress::new("bm-progress").value(f * 100.0))).child(text(&p, theme::CAPTION, percent(f)))).into_any_element()
            }
            Job::Done(Ok(())) => banner(&p, Tone::Ok, "Boot agent installed", "DSM is still the default. Backup of the boot flash is in the dom-backups folder. To boot into the agent, use `synofurb boot-manager enter`.", vec![]).into_any_element(),
            Job::Done(Err(e)) => banner(&p, Tone::Err, "Could not install the boot manager", e.clone(), vec![]).id("bm-err").role(Role::Alert).into_any_element(),
        };
        let mut body = v_flex().w_full().gap(px(space::S16)).child(touched).child(flow);
        if let Err(why) = avail {
            body = body.child(banner(&p, Tone::Warn, "Not available for this unit", why, vec![]));
        }
        body = body.child(state);
        let running = self.bootmgr.running();
        Page::new("Boot manager mode", body)
            .subtitle(format!("{name}. Try other operating systems with DSM always one reboot away."))
            .note(muted(&p, "Your disks are not touched."))
            .buttons(vec![
                btn_secondary("back", "Back")
                    .icon(icon("arrow-left"))
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Goal, window, cx)))
                    .into_any_element(),
                Button::new("install-bm")
                    .primary()
                    .icon(icon("download"))
                    .label("Install boot manager")
                    .loading(running)
                    .disabled(avail.is_err() || running)
                    .on_click(cx.listener(|this, _, _, cx| this.start_boot_manager(cx)))
                    .into_any_element(),
            ])
    }
}
