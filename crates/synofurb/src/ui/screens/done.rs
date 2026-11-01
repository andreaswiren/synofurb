//! Final step: where the new system is and how to log in. The password and the private key are shown here once.

use crate::ui::app::App;
use crate::ui::format::{mask, shorten_path};
use crate::ui::render::Page;
use crate::ui::theme::{self, Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FontWeight, IntoElement, Window, px};
use synofurb_core::model::Firmware;

impl App {
    /// Value + small copy button on one line.
    fn copy_row(&self, p: &theme::Palette, id: &str, value: String, what: &'static str, cx: &mut Context<App>) -> AnyElement {
        let v = value.clone();
        let id = id.to_string();
        h_flex()
            .gap(px(space::S8))
            .items_center()
            .child(mono(p, value, cx).flex_1().min_w_0())
            .child(
                btn_ghost(gpui_kit::SharedString::from(format!("copy-{id}")), "Copy")
                    .small()
                    .icon(icon("copy"))
                    .on_click(cx.listener(move |this, _, window, cx| this.copy_text(v.clone(), what, window, cx))),
            )
            .into_any_element()
    }

    pub(crate) fn page_done(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let Some(Ok(o)) = self.result.clone() else {
            return Page::new("Debian is installed", muted(&p, "Waiting for the result...")).buttons(vec![]);
        };
        let name = self.unit_name();
        let ip = o.ip.map(|i| i.to_string()).unwrap_or_else(|| "see your DHCP server".into());
        let password = self.input_value(&self.inp.new_password, cx);
        let shown = if self.reveal_password { password.clone() } else { mask(&password) };
        let ssh_cmd = format!("ssh {}@{}", o.username, ip);
        let serial = {
            let t = self.input_value(&self.inp.serial_text, cx);
            let s = self.input_value(&self.inp.serial_speed, cx);
            format!(
                "{t} at {s} baud, 8N1 (built-in port and any USB serial adapter). VGA/HDMI and a USB keyboard work on the local console."
            )
        };

        let reach = if o.mgmt_reachable {
            banner(
                &p,
                Tone::Ok,
                format!("Debian is installed on {name}"),
                "The new system answers on SSH. Write the logins below down: the password and the private key are not kept anywhere else.",
                vec![],
            )
        } else {
            banner(
                &p,
                Tone::Warn,
                format!("Debian is installed on {name}, but the management page did not answer yet"),
                "Give it a minute. If SSH works, the management plane may still be starting.",
                vec![],
            )
        };

        let reveal = h_flex()
            .gap(px(space::S8))
            .items_center()
            .child(mono(&p, shown, cx).flex_1().min_w_0())
            .child(
                btn_ghost("reveal-pw", if self.reveal_password { "Hide" } else { "Reveal" })
                    .small()
                    .icon(icon(if self.reveal_password { "eye-off" } else { "eye" }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reveal_password = !this.reveal_password;
                        cx.notify();
                    })),
            )
            .child(btn_ghost("copy-pw", "Copy").small().icon(icon("copy")).on_click(cx.listener(|this, _, window, cx| {
                let pw = this.input_value(&this.inp.new_password, cx);
                this.copy_text(pw, "Password", window, cx);
            })));

        let mut logins = v_flex()
            .child(kv(&p, "Host name", text(&p, theme::BODY_SM, format!("{}  (also {}.local through mDNS)", o.hostname, o.hostname))))
            .child(kv(&p, "Address", self.copy_row(&p, "addr", ip.clone(), "Address", cx)))
            .child(kv(&p, "User", text(&p, theme::BODY_SM, o.username.clone()).font_weight(FontWeight(600.0))))
            .child(kv(&p, "Password", reveal))
            .child(kv(&p, "SSH command", self.copy_row(&p, "ssh", ssh_cmd.clone(), "SSH command", cx)));
        if let Some(fp) = &o.ssh_fingerprint {
            logins = logins.child(kv(&p, "SSH host key", self.copy_row(&p, "fp", fp.clone(), "Fingerprint", cx)));
        }
        if let Some((kp, path)) = &self.sys.generated_key {
            let pstr = path.display().to_string();
            logins = logins.child(kv(
                &p,
                "SSH private key",
                v_flex().gap(px(space::S2)).child(self.copy_row(&p, "key", pstr.clone(), "Key path", cx)).child(
                    muted(&p, format!("{}  -  use: ssh -i {} {}@{}", kp.fingerprint, shorten_path(&pstr, 48), o.username, ip))
                        .text_size(theme::CAPTION.size()),
                ),
            ));
        }
        if let Some(u) = &o.mgmt_url {
            let uu = u.clone();
            logins = logins.child(kv(
                &p,
                "Management plane",
                v_flex()
                    .gap(px(space::S2))
                    .child(h_flex().gap(px(space::S8)).items_center().child(mono(&p, u.clone(), cx)).child(
                        btn_ghost("open-mgmt", "Open").small().icon(icon("external-link")).on_click(move |_, _, cx| cx.open_url(&uu)),
                    ))
                    .child(
                        muted(&p, "Self-signed certificate: your browser will warn once. Log in with the user above.")
                            .text_size(theme::CAPTION.size()),
                    ),
            ));
        }
        logins = logins.child(kv(&p, "Serial console", muted(&p, serial)));

        let mut next = vec![
            "Root login is disabled. Use the user above with sudo.".to_string(),
            "RAID arrays synchronise in the background after the first boot; large disks take hours. The NAS is usable meanwhile."
                .to_string(),
        ];
        if self.facts.as_ref().is_some_and(|f| f.firmware == Firmware::Dom) {
            next.push("If the NAS does not boot from its disks, check the BIOS/UEFI boot order: the internal Synology flash module may take precedence.".into());
        } else {
            next.push("If the NAS boots into the old system, check the BIOS/UEFI boot order.".into());
        }
        if let Some(path) = &o.log_path {
            next.push(format!("The full debug log is saved at {}.", path.display()));
        }
        let next_card = card(
            &p,
            Some("Next steps".into()),
            None,
            None,
            v_flex().gap(px(space::S6)).children(next.into_iter().map(|l| {
                h_flex()
                    .gap(px(space::S8))
                    .child(icon_sized("chevron-right", 12.0, p.placeholder).mt(px(5.0)))
                    .child(muted(&p, l).flex_1().min_w_0())
            })),
        );

        let mut body = v_flex().w_full().gap(px(space::S16)).child(reach).child(card(
            &p,
            Some("Logins".into()),
            Some(badge(&p, Tone::Warn, "key-round", "Shown once", false).into_any_element()),
            None,
            logins,
        ));
        if let Some(b) = self.uboot_box(&p, cx) {
            body = body.child(b);
        }
        body = body.child(next_card);

        Page::new(format!("Debian is installed on {name}"), body)
            .subtitle("Everything you need to log in. Store the password and the key somewhere safe.")
            .note(muted(&p, "Root login is disabled."))
            .buttons(vec![
                btn_secondary("save-logins", "Save logins to a file...")
                    .icon(icon("save"))
                    .on_click(cx.listener(|this, _, window, cx| this.save_logins(window, cx)))
                    .into_any_element(),
                btn_secondary("copy-ssh", "Copy SSH command")
                    .icon(icon("terminal"))
                    .on_click(cx.listener(move |this, _, window, cx| this.copy_text(ssh_cmd.clone(), "SSH command", window, cx)))
                    .into_any_element(),
                btn_primary("start-over", "Start over")
                    .icon(icon("undo-2"))
                    .on_click(cx.listener(|this, _, window, cx| this.start_over(window, cx)))
                    .into_any_element(),
            ])
    }
}
