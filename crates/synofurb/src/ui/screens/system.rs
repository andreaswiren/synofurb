//! Step 6: settings of the new system: account, SSH key, time zone, mirror, serial console, management plane and
//! how the installer is started (Auto / boot flash / kexec / kit).

use crate::ui::app::App;
use crate::ui::render::Page;
use crate::ui::state::*;
use crate::ui::theme::{self, Themed, Tone, space};
use crate::ui::widgets::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, IntoElement, SharedString, Window, div, px};
use synofurb_core::config::{generate_password, validate_hostname, validate_ssh_public_key, validate_username};
use synofurb_core::model::Arch;

/// Field-level problems of the system form (empty = valid). `(field, message)`.
pub fn system_errors(
    hostname: &str,
    username: &str,
    password: &str,
    key_mode: KeyMode,
    pubkey: &str,
    serial_text: &str,
    serial_speed: &str,
) -> Vec<(&'static str, String)> {
    let mut v = vec![];
    if let Err(e) = validate_hostname(hostname.trim()) {
        v.push(("hostname", e.to_string()));
    }
    if let Err(e) = validate_username(username.trim()) {
        v.push(("username", e.to_string()));
    }
    if password.len() < 8 {
        v.push(("password", "Use at least 8 characters.".into()));
    }
    if key_mode == KeyMode::Paste
        && let Err(e) = validate_ssh_public_key(pubkey.trim())
    {
        v.push(("pubkey", e.to_string()));
    }
    if serial_from_input(serial_text, serial_speed).is_none() {
        v.push(("serial", "Not a valid console: use a name such as ttyS0, or a raw console such as uart8250,mmio32,0xfedc9000,115200n8, and a speed such as 115200.".into()));
    }
    v
}

impl App {
    pub(crate) fn system_form_errors(&self, cx: &gpui_kit::App) -> Vec<(&'static str, String)> {
        system_errors(
            &self.input_value(&self.inp.hostname, cx),
            &self.input_value(&self.inp.username, cx),
            &self.input_value(&self.inp.new_password, cx),
            self.sys.key_mode,
            &self.input_value(&self.inp.pubkey, cx),
            &self.input_value(&self.inp.serial_text, cx),
            &self.input_value(&self.inp.serial_speed, cx),
        )
    }

    pub(crate) fn page_system(&mut self, window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let Some(f) = self.facts.clone() else {
            return Page::new("The new system", muted(&p, "Inspect a unit first.")).buttons(vec![
                btn_secondary("back", "Back")
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Plan, window, cx)))
                    .into_any_element(),
            ]);
        };
        let errors = self.system_form_errors(cx);
        let err_for = |field: &str| errors.iter().find(|(k, _)| *k == field).map(|(_, m)| SharedString::from(m.clone()));
        let helper_or_err = |field: &str, helper: &str| -> Option<SharedString> {
            err_for(field).or_else(|| (!helper.is_empty()).then(|| SharedString::from(helper.to_string())))
        };

        // ---- account
        let show = self.sys.show_password;
        let password_row = h_flex()
            .gap(px(space::S8))
            .items_center()
            .child(div().flex_1().min_w_0().child(Input::new(&self.inp.new_password)))
            .child(Checkbox::new("show-pw").label("show").checked(show).on_click(cx.listener(|this, v: &bool, window, cx| {
                this.sys.show_password = *v;
                let st = this.inp.new_password.clone();
                let masked = !*v;
                st.update(cx, |s, cx| s.set_masked(masked, window, cx));
                cx.notify();
            })))
            .child(btn_secondary("gen-pw", "Generate").small().icon(icon("refresh-cw")).on_click(cx.listener(|this, _, window, cx| {
                let st = this.inp.new_password.clone();
                this.set_input(&st, &generate_password(20), window, cx);
            })))
            .child(btn_secondary("copy-pw", "Copy").small().icon(icon("copy")).on_click(cx.listener(|this, _, window, cx| {
                let pw = this.input_value(&this.inp.new_password, cx);
                this.copy_text(pw, "Password", window, cx);
            })));

        let key_choices = v_flex()
            .gap(px(space::S8))
            .child(choice_card(
                &p,
                "key-gen",
                self.sys.key_mode == KeyMode::Generate,
                false,
                "Generate a new ed25519 key pair",
                "The private key is saved on this computer (readable only by you) and shown once at the end.",
                cx.listener(|this, _, _, cx| {
                    this.sys.key_mode = KeyMode::Generate;
                    cx.notify();
                }),
            ))
            .child(choice_card(
                &p,
                "key-paste",
                self.sys.key_mode == KeyMode::Paste,
                false,
                "Use my existing public key",
                "Paste the line from your .pub file.",
                cx.listener(|this, _, _, cx| {
                    this.sys.key_mode = KeyMode::Paste;
                    cx.notify();
                }),
            ))
            .when(self.sys.key_mode == KeyMode::Paste, |d| d.child(Input::new(&self.inp.pubkey)))
            .child(choice_card(
                &p,
                "key-none",
                self.sys.key_mode == KeyMode::None,
                false,
                "No key (password login only)",
                "Less secure; you can add a key later.",
                cx.listener(|this, _, _, cx| {
                    this.sys.key_mode = KeyMode::None;
                    cx.notify();
                }),
            ));

        let account = v_flex()
            .gap(px(14.0))
            .child(form_row(
                &p,
                "Host name",
                helper_or_err("hostname", "Also reachable as <name>.local through mDNS."),
                Input::new(&self.inp.hostname),
            ))
            .child(form_row(
                &p,
                "Default user",
                helper_or_err("username", "Gets sudo. Root login over SSH is off."),
                Input::new(&self.inp.username),
            ))
            .child(form_row(
                &p,
                "Password",
                helper_or_err("password", "Shown once more at the end. synofurb does not keep it."),
                password_row,
            ))
            .child(form_row(
                &p,
                "SSH key",
                helper_or_err("pubkey", "SSH accepts both: key login is tried first and password login stays enabled."),
                key_choices,
            ));

        // ---- region and console
        let serial = f.serial_console();
        let detected = match &serial.raw {
            Some(raw) => format!(
                "Detected from DSM's kernel command line: raw console {raw}. AMD Ryzen/EPYC boards have no legacy UART, so this is configured as given."
            ),
            None => format!(
                "Detected from DSM's kernel command line: {} at {} baud. USB serial adapters get a login prompt automatically.",
                serial.tty, serial.speed
            ),
        };
        let mut region = v_flex()
            .gap(px(14.0))
            .child(form_row(&p, "Time zone", None, div().w(px(280.0)).child(Input::new(&self.inp.timezone))))
            .child(form_row(
                &p,
                "Debian mirror",
                Some("The NAS downloads packages from here during the install, so it needs Internet access (or a local mirror).".into()),
                Input::new(&self.inp.mirror),
            ))
            .child(form_row(
                &p,
                "Serial console",
                helper_or_err("serial", &detected),
                h_flex()
                    .gap(px(space::S8))
                    .child(div().flex_1().min_w_0().child(Input::new(&self.inp.serial_text)))
                    .child(div().w(px(110.0)).child(Input::new(&self.inp.serial_speed)))
                    .child(div().pt(px(8.0)).child(muted(&p, "baud"))),
            ));
        if f.arch == Arch::Armhf {
            region = region.child(form_row(
                &p,
                "Board device tree",
                Some("File name known to Debian for this board; needed on ARM U-Boot units.".into()),
                Input::new(&self.inp.dtb),
            ));
        }
        let region = region.child(form_row(
            &p,
            "Management plane",
            None,
            v_flex()
                .gap(px(space::S8))
                .child(
                    Switch::new("mgmt")
                        .label("Install the management plane (status, users, network, volumes, mounts) on https://<ip>:8443")
                        .checked(self.sys.enable_mgmt)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.sys.enable_mgmt = *v;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("fw")
                        .label("Include non-free firmware in the installer (needed by some network chips)")
                        .checked(self.sys.with_firmware)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.sys.with_firmware = *v;
                            cx.notify();
                        })),
                )
                .child(Checkbox::new("verbose").label("Verbose installer debug logging (recommended)").checked(self.sys.verbose).on_click(
                    cx.listener(|this, v: &bool, _, cx| {
                        this.sys.verbose = *v;
                        cx.notify();
                    }),
                )),
        ));

        // ---- boot method
        let resolved = self.sys.boot.resolve(&f);
        let mut methods = v_flex().gap(px(space::S8));
        for (i, m) in BootChoice::ALL.iter().enumerate() {
            let avail = m.available(&f);
            let m2 = *m;
            let title = if *m == BootChoice::Auto { format!("{}: {}", m.label(), resolved.label()) } else { m.label().to_string() };
            let detail = match avail {
                Ok(()) => m.explain().to_string(),
                Err(why) => format!("{why} {}", m.explain()),
            };
            methods = methods.child(choice_card(
                &p,
                ("boot", i),
                self.sys.boot == *m,
                avail.is_err(),
                title,
                detail,
                cx.listener(move |this, _, _, cx| {
                    this.sys.boot = m2;
                    cx.notify();
                }),
            ));
        }
        match resolved {
            BootChoice::Kexec => {
                methods = methods.child(form_row(
                    &p,
                    "kexec binary",
                    Some("Leave empty to use assets/kexec/<arch>/kexec.".into()),
                    h_flex().gap(px(space::S8)).child(div().flex_1().min_w_0().child(Input::new(&self.inp.kexec_path))).child(
                        btn_secondary("browse-kexec", "Browse...").on_click(cx.listener(|this, _, window, cx| {
                            let t = this.inp.kexec_path.clone();
                            this.browse_file(t, window, cx)
                        })),
                    ),
                ));
            }
            BootChoice::Kit => {
                methods = methods.child(form_row(
                    &p,
                    "Kit folder",
                    Some("Kernel, initrd and instructions are written here; you boot the NAS from them.".into()),
                    h_flex().gap(px(space::S8)).child(div().flex_1().min_w_0().child(Input::new(&self.inp.kit_dir))).child(
                        btn_secondary("browse-kit", "Browse...").on_click(cx.listener(|this, _, window, cx| {
                            let t = this.inp.kit_dir.clone();
                            this.browse_dir(t, window, cx)
                        })),
                    ),
                ));
            }
            BootChoice::Dom => {
                methods = methods.child(card(
                    &p,
                    Some("What the boot-flash method does".into()),
                    Some(badge(&p, Tone::Ok, "shield-check", "Backup first, DSM stays the fail-safe", false).into_any_element()),
                    None,
                    v_flex().gap(px(space::S6)).children(DOM_FLOW.iter().enumerate().map(|(i, s)| {
                        h_flex()
                            .gap(px(space::S8))
                            .child(text(&p, theme::CAPTION, format!("{}", i + 1)).text_color(p.accent_fg).w(px(14.0)).flex_none())
                            .child(muted(&p, *s).flex_1().min_w_0())
                    })),
                ));
            }
            _ => {}
        }

        let body = v_flex()
            .w_full()
            .gap(px(space::S16))
            .child(card(&p, Some("Account".into()), None, None, account))
            .child(card(&p, Some("Region, console and services".into()), None, None, region))
            .child(card(&p, Some("How the installer is started".into()), None, None, methods));

        let valid = errors.is_empty();
        let _ = window;
        Page::new("The new system", body)
            .subtitle("These settings end up on the NAS. Defaults are filled in from what was detected; nothing is written yet.")
            .note(if valid {
                muted(&p, "Settings look valid.").into_any_element()
            } else {
                h_flex()
                    .gap(px(space::S8))
                    .items_center()
                    .child(icon_sized("circle-x", 16.0, p.err_fg))
                    .child(
                        text(&p, theme::BODY_SM, format!("{} setting(s) need attention.", errors.len()))
                            .text_color(p.err_fg)
                            .font_weight(FontWeight(600.0)),
                    )
                    .into_any_element()
            })
            .buttons(vec![
                btn_secondary("back", "Back")
                    .icon(icon("arrow-left"))
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Plan, window, cx)))
                    .into_any_element(),
                btn_primary("next", "Next: review and confirm")
                    .icon(icon("arrow-right"))
                    .disabled(!valid)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Confirm, window, cx)))
                    .into_any_element(),
            ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Vec<(&'static str, String)> {
        system_errors("nas", "nasadmin", "a-strong-password", KeyMode::Generate, "", "ttyS0", "115200")
    }

    #[test]
    fn defaults_are_valid() {
        assert!(ok().is_empty());
    }

    #[test]
    fn bad_fields_are_reported_per_field() {
        let e = system_errors("Bad Host!", "root", "short", KeyMode::Paste, "not a key", "ttyS0;x", "115200");
        let fields: Vec<&str> = e.iter().map(|(k, _)| *k).collect();
        for f in ["hostname", "password", "pubkey", "serial"] {
            assert!(fields.contains(&f), "{f} in {fields:?}");
        }
        let amd = system_errors("nas", "nasadmin", "a-strong-password", KeyMode::None, "", "uart8250,mmio32,0xfedc9000,115200n8", "115200");
        assert!(amd.is_empty(), "raw AMD consoles are accepted: {amd:?}");
    }
}
