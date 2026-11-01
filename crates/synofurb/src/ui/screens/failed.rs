//! Failure and recovery: what happened, were the disks touched, what to do next, in that order.

use crate::ui::app::{App, Job};
use crate::ui::render::Page;
use crate::ui::theme::{self, Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FontWeight, IntoElement, Role, Window, div, px};

impl App {
    pub(crate) fn page_failed(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let name = self.unit_name();
        let touched = self.install.past_no_return;
        let err = match &self.result {
            Some(Err(e)) => e.clone(),
            _ => self.install.installer_failed.clone().unwrap_or_else(|| "The installer stopped.".into()),
        };
        let step = if self.install.step_total > 0 {
            format!("Install stopped at step {} of {}.", self.install.step_n, self.install.step_total)
        } else {
            "The install stopped before the installer started.".to_string()
        };
        let headline =
            if touched { format!("{step} Your disks have been changed.") } else { format!("{step} Your disks were not changed.") };

        let status =
            banner(&p, if touched { Tone::Err } else { Tone::Ok }, headline, err.clone(), vec![]).id("fail-banner").role(Role::Alert);
        let touched_card = card(
            &p,
            Some("Were the disks touched?".into()),
            Some(badge(&p, if touched { Tone::Err } else { Tone::Ok }, if touched { "triangle-alert" } else { "shield-check" }, if touched { "Disks touched: Yes" } else { "Disks touched: No" }, false).into_any_element()),
            Some(if touched { p.err_solid } else { p.ok_solid }),
            v_flex()
                .gap(px(space::S6))
                .child(muted(&p, if touched {
                    "The point of no return passed before the failure, so what was on the reclaimed areas is gone. The installer halted instead of guessing. The first and last MiB of every changed disk (partition tables, never your files) were saved on this computer first."
                } else {
                    "The failure happened before the point of no return. Nothing was written to the disks, and DSM boots as before once the NAS restarts."
                }))
                .child(muted(&p, "The full log is saved on this computer.")),
        );

        // next steps, ordered by cost; wording follows docs/RECOVERY.md
        let dom = self.facts.as_ref().is_some_and(|f| f.firmware == synofurb_core::model::Firmware::Dom);
        let next: Vec<(&str, String)> = if touched {
            vec![
                ("Read the log", "The last error lines are below. Search for ERR to find the first problem. The log and the disk metadata backup are on this computer.".to_string()),
                (
                    "Know what the NAS does next",
                    if dom {
                        "The default boot entry is set back to DSM and the unit reboots after 15 min so you can read the log. Only the DSM loader runs: DSM's operating system on the reclaimed partitions is gone.".to_string()
                    } else {
                        "The installer keeps its log and a rescue shell on the NAS console (VGA or serial). It does not retry on its own.".to_string()
                    },
                ),
                (
                    "Reinstall DSM if you want it back",
                    "Open https://find.synology.com (or Synology Assistant), find the unit and install DSM with the .pat file for your model. Synology documents keeping data volumes on a reinstall; synofurb cannot promise that any data survives.".to_string(),
                ),
            ]
        } else {
            vec![
                (
                    "Wait a minute and check again",
                    if dom {
                        "The installer's failure handler sets the default to DSM and reboots the unit after 60 s.".to_string()
                    } else {
                        "The NAS may still be rebooting into or out of the installer.".to_string()
                    },
                ),
                ("Power-cycle the NAS", "DSM comes back because the disks were not changed.".to_string()),
                (
                    "Try the installer kit instead",
                    "Choose \"installer kit\" under System settings and boot the NAS from it yourself.".to_string(),
                ),
            ]
        };
        let steps = card(
            &p,
            Some("What to do next".into()),
            None,
            None,
            v_flex().gap(px(space::S8)).children(next.iter().enumerate().map(|(i, (t, d))| {
                h_flex()
                    .gap(px(space::S12))
                    .items_start()
                    .child(
                        div()
                            .size(px(22.0))
                            .flex_none()
                            .rounded_full()
                            .border_1()
                            .border_color(p.border_strong)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text(&p, theme::CAPTION, (i + 1).to_string())),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(text(&p, theme::BODY, t.to_string()).font_weight(FontWeight(600.0)))
                            .child(muted(&p, d.clone())),
                    )
            })),
        );

        let recover_state: AnyElement = match &self.recover {
            Job::Idle => muted(
                &p,
                if dom {
                    "Restore tells the unit to boot DSM at its next start."
                } else {
                    "This unit has no internal boot flash, so there is no boot setting to restore from here: follow the steps above."
                },
            )
            .into_any_element(),
            Job::Running => muted(&p, "Restoring... the boot setting is being changed over SSH.").into_any_element(),
            Job::Done(Ok(s)) => banner(
                &p,
                Tone::Ok,
                "Boot setting restored",
                format!(
                    "{} The unit boots DSM at its next reboot. If DSM's system partition is gone, DSM starts in recovery mode.",
                    s.trim()
                ),
                vec![],
            )
            .into_any_element(),
            Job::Done(Err(e)) => {
                banner(&p, Tone::Err, "Could not restore", e.clone(), vec![]).id("recover-error").role(Role::Alert).into_any_element()
            }
        };

        let tail: Vec<_> = self.install.log.tail_info(14).into_iter().cloned().collect();
        let log_card = card(
            &p,
            Some("Last log lines".into()),
            None,
            None,
            code_box(&p, &tail.iter().map(crate::ui::logview::format_line).collect::<Vec<_>>(), cx),
        );

        let log_path = self.logger_path.clone().map(|p| p.display().to_string()).unwrap_or_else(|| "(no log file was written)".into());
        let body = v_flex()
            .w_full()
            .gap(px(space::S16))
            .child(status)
            .child(touched_card)
            .child(steps)
            .child(recover_state)
            .child(log_card)
            .child(
                h_flex()
                    .gap(px(space::S8))
                    .items_center()
                    .child(icon_sized("file-text", 14.0, p.fg_2))
                    .child(muted(&p, "Log file:"))
                    .child(mono(&p, log_path, cx).flex_1().min_w_0().truncate()),
            );

        let log_path2 = self.logger_path.clone();
        let log_dir = self.cfg_dir.join("logs");
        Page::new("The installation did not finish", body)
            .subtitle(format!("{name}. What happened, whether the disks were touched, and what to do next."))
            .note(muted(
                &p,
                if touched {
                    "Do not power off the NAS while it is reading its console."
                } else {
                    "No restore is needed: nothing was written."
                },
            ))
            .buttons(vec![
                btn_secondary("open-logs", "Show logs folder")
                    .icon(icon("folder-open"))
                    .on_click(move |_, _, cx| {
                        let target = log_path2.clone().unwrap_or_else(|| log_dir.clone());
                        cx.reveal_path(&target)
                    })
                    .into_any_element(),
                btn_secondary("restore-dsm", if touched { "Restore to DSM" } else { "Confirm DSM is back" })
                    .icon(icon("life-buoy"))
                    .loading(self.recover.running())
                    .disabled(!dom || self.recover.running())
                    .tooltip(if dom { "" } else { "Only units with an internal boot flash can be pointed back at DSM from here" })
                    .on_click(cx.listener(|this, _, _, cx| this.start_recover(cx)))
                    .into_any_element(),
                btn_primary("start-over", "Start over")
                    .icon(icon("undo-2"))
                    .on_click(cx.listener(|this, _, window, cx| this.start_over(window, cx)))
                    .into_any_element(),
            ])
    }
}
