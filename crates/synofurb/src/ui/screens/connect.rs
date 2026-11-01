//! Step 2: connect to DSM over SSH. Says first what it needs and exactly what it reads.

use crate::ui::app::App;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Window, div, px};

impl App {
    pub(crate) fn page_connect(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let inspecting = self.inspecting;

        let login = RadioGroup::horizontal("login-with")
            .selected_index(Some(if self.use_key { 1 } else { 0 }))
            .child(Radio::new("lw-pass").label("Password"))
            .child(Radio::new("lw-key").label("Private key file"))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.use_key = *ix == 1;
                cx.notify();
            }));

        let auth_row = if self.use_key {
            form_row(
                &p,
                "Key file",
                Some("An OpenSSH private key. It is read on this computer and used for this connection only.".into()),
                h_flex().gap(px(space::S8)).child(div().flex_1().min_w_0().child(Input::new(&self.inp.key_path))).child(
                    btn_secondary("browse-key", "Browse...").on_click(cx.listener(|this, _, window, cx| {
                        let t = this.inp.key_path.clone();
                        this.browse_file(t, window, cx)
                    })),
                ),
            )
        } else {
            form_row(&p, "Password", None, self.password_input())
        };

        let form = v_flex()
            .w_full()
            .gap(px(14.0))
            .child(form_row(&p, "Address", None, Input::new(&self.inp.host)))
            .child(form_row(&p, "SSH port", None, div().w(px(120.0)).child(Input::new(&self.inp.port))))
            .child(form_row(
                &p,
                "User",
                Some("An administrator account. DSM 7 admins get root through sudo.".into()),
                Input::new(&self.inp.user),
            ))
            .child(form_row(&p, "Log in with", None, login))
            .child(auth_row)
            .child(form_row(
                &p,
                "sudo",
                None,
                v_flex()
                    .gap(px(space::S8))
                    .child(Checkbox::new("same-sudo").label("Use the same password for sudo").checked(self.same_sudo).on_click(
                        cx.listener(|this, v: &bool, _, cx| {
                            this.same_sudo = *v;
                            cx.notify();
                        }),
                    ))
                    .when(!self.same_sudo, |d| d.child(Input::new(&self.inp.sudo).mask_toggle())),
            ));

        let reads = v_flex()
            .gap(px(space::S6))
            .child(muted(&p, "synofurb reads, read-only:"))
            .children(
                [
                    "Model, DSM version, CPU, RAM and kernel command line",
                    "Disks, partition tables and RAID/LVM layout",
                    "Mounted volumes and network cards",
                    "Whether the kernel supports kexec, and the internal boot flash if present",
                ]
                .map(|l| {
                    h_flex().gap(px(space::S8)).child(icon_sized("check", 14.0, p.ok_fg).mt(px(3.0))).child(muted(&p, l).flex_1().min_w_0())
                }),
            )
            .child(divider(&p))
            .child(muted(&p, "Nothing is written to the NAS until you confirm the data-loss gate at step 7."));

        let enable_ssh = banner(
            &p,
            Tone::Info,
            "SSH must be on in DSM",
            "Control Panel > Terminal & SNMP > Enable SSH service. Use an administrator account.",
            vec![],
        );

        let mut body = v_flex().w_full().gap(px(space::S16)).child(enable_ssh);
        if let Some(e) = &self.error {
            body =
                body.child(banner(&p, Tone::Err, "Could not connect", e.clone(), vec![]).id("connect-error").role(gpui_kit::Role::Alert));
        }
        body = body.child(
            h_flex()
                .w_full()
                .items_start()
                .gap(px(space::S24))
                .child(div().flex_1().min_w_0().child(card(&p, Some("Connection".into()), None, None, form)))
                .child(div().w(px(300.0)).flex_none().child(card(&p, Some("What we read".into()), None, None, reads))),
        );

        let host_empty = self.input_value(&self.inp.host, cx).trim().is_empty();
        Page::new("Connect to DSM", body)
            .subtitle("synofurb needs SSH access to read the disk layout (read-only) and, later, to start the installer.")
            .note(muted(
                &p,
                if inspecting {
                    "Connecting and reading the unit. Nothing is being changed."
                } else {
                    "Nothing on the NAS has been changed."
                },
            ))
            .buttons(vec![
                btn_secondary("back-inv", "Back")
                    .icon(icon("arrow-left"))
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Inventory, window, cx)))
                    .into_any_element(),
                Button::new("inspect")
                    .primary()
                    .label(if inspecting { "Inspecting..." } else { "Connect and inspect" })
                    .loading(inspecting)
                    .disabled(inspecting || host_empty)
                    .on_click(cx.listener(|this, _, window, cx| this.start_inspect(window, cx)))
                    .into_any_element(),
            ])
    }
}
