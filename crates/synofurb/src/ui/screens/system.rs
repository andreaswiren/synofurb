//! D6 System: name, login, kernel, source and boot method. One `Form`, ToggleGroups for the small choices, a collapsed
//! Accordion for the options most people never touch.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::*;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::form::Form;
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div, px};
use synofurb_core::config::{KernelChannel, generate_password, validate_hostname, validate_ssh_public_key, validate_username};
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
        let mut v = system_errors(
            &self.input_value(&self.inp.hostname, cx),
            &self.input_value(&self.inp.username, cx),
            &self.input_value(&self.inp.new_password, cx),
            self.sys.key_mode,
            &self.input_value(&self.inp.pubkey, cx),
            &self.input_value(&self.inp.serial_text, cx),
            &self.input_value(&self.inp.serial_speed, cx),
        );
        if self.source_offline && !std::path::Path::new(self.input_value(&self.inp.bundle, cx).trim()).is_dir() {
            v.push(("bundle", copy::SYS_NO_BUNDLE.tip.into()));
        }
        v
    }

    pub(crate) fn page_system(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let Some(f) = self.facts.clone() else {
            return Page::new(copy::SYS_TITLE.short, kit::muted(cx, "-")).left(vec![
                Button::new("back")
                    .ghost()
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Plan, window, cx)))
                    .into_any_element(),
            ]);
        };
        let errors = self.system_form_errors(cx);
        let err_for = |field: &str| errors.iter().find(|(k, _)| *k == field).map(|(_, m)| m.clone());

        let password = h_flex()
            .gap(px(4.0))
            .items_center()
            .child(div().flex_1().min_w_0().child(Input::new(&self.inp.new_password).mask_toggle()))
            .child(Clipboard::new("copy-pw").value(self.input_value(&self.inp.new_password, cx)).tooltip("Copy"))
            .child(
                Button::new("gen-pw")
                    .ghost()
                    .icon(icons::icon(role::REFRESH))
                    .tooltip("Generate")
                    .accessibility_label("Generate")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let st = this.inp.new_password.clone();
                        this.set_input(&st, &generate_password(20), window, cx);
                    })),
            );
        let key_ix = match self.sys.key_mode {
            KeyMode::Generate => 0,
            KeyMode::Paste => 1,
            KeyMode::None => 2,
        };
        let key = v_flex()
            .gap(px(8.0))
            .child(kit::segmented(
                "ssh-key",
                &[copy::SYS_KEY_GENERATE.short, copy::SYS_KEY_PASTE.short, copy::SYS_KEY_NONE.short],
                key_ix,
                {
                    let app = app.clone();
                    move |ix, _, cx| {
                        app.update(cx, |a, cx| {
                            a.sys.key_mode = [KeyMode::Generate, KeyMode::Paste, KeyMode::None][ix];
                            cx.notify();
                        })
                    }
                },
            ))
            .when(self.sys.key_mode == KeyMode::Paste, |d| d.child(Input::new(&self.inp.pubkey)));
        let kernel = kit::segmented(
            "kernel",
            &[copy::SYS_STABLE.short, copy::SYS_NEWEST.short],
            usize::from(self.form.kernel == KernelChannel::Newest),
            {
                let app = app.clone();
                move |ix, _, cx| {
                    app.update(cx, |a, cx| {
                        a.form.kernel = if ix == 1 { KernelChannel::Newest } else { KernelChannel::Stable };
                        cx.notify();
                    })
                }
            },
        );
        let source = kit::segmented("source", &[copy::SYS_ONLINE.short, copy::SYS_OFFLINE.short], usize::from(self.source_offline), {
            let app = app.clone();
            move |ix, _, cx| {
                app.update(cx, |a, cx| {
                    a.source_offline = ix == 1;
                    cx.notify();
                })
            }
        });
        let resolved = self.sys.boot.resolve(&f);
        let via = match resolved {
            BootChoice::Dom => copy::SYS_VIA_FLASH.short,
            BootChoice::Kexec => copy::SYS_VIA_KEXEC.short,
            _ => copy::SYS_VIA_KIT.short,
        };

        let mut form = Form::vertical()
            .columns(2)
            .child(kit::field("f-host", &copy::SYS_HOSTNAME, &app, Input::new(&self.inp.hostname)))
            .child(kit::field("f-user", &copy::SYS_USER, &app, Input::new(&self.inp.username)))
            .child(kit::field("f-pw", &copy::SYS_PASSWORD, &app, password))
            .child(kit::field("f-key", &copy::SYS_SSH_KEY, &app, key))
            .child(kit::field_plain(copy::SYS_TIMEZONE.short, div().w(px(280.0)).child(Input::new(&self.inp.timezone))))
            .child(kit::field(
                "f-console",
                &copy::SYS_CONSOLE,
                &app,
                h_flex()
                    .gap(px(8.0))
                    .child(div().flex_1().min_w_0().child(Input::new(&self.inp.serial_text)))
                    .child(div().w(px(110.0)).child(Input::new(&self.inp.serial_speed))),
            ))
            .child(kit::field(
                "f-web",
                &copy::SYS_WEB_UI,
                &app,
                Switch::new("mgmt").checked(self.sys.enable_mgmt).on_click(cx.listener(|this, v: &bool, _, cx| {
                    this.sys.enable_mgmt = *v;
                    cx.notify();
                })),
            ))
            .child(kit::field("f-kernel", &copy::SYS_KERNEL, &app, kernel))
            .child(kit::field("f-source", &copy::SYS_SOURCE, &app, source));
        if self.source_offline {
            let missing = err_for("bundle");
            form = form.child(kit::field(
                "f-bundle",
                &copy::SYS_BUNDLE,
                &app,
                v_flex()
                    .gap(px(6.0))
                    .child(
                        h_flex()
                            .gap(px(8.0))
                            .child(div().flex_1().min_w_0().child(Input::new(&self.inp.bundle)))
                            .child(
                                Button::new("browse-bundle")
                                    .outline()
                                    .icon(icons::icon(role::FOLDER))
                                    .label(copy::CONN_BROWSE.short)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let t = this.inp.bundle.clone();
                                        this.browse_dir(t, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("dl-bundle")
                                    .ghost()
                                    .label(copy::SYS_DOWNLOAD.short)
                                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Settings, window, cx))),
                            ),
                    )
                    .when_some(missing, |d, _| {
                        d.child(
                            Alert::error("no-bundle", copy::SYS_NO_BUNDLE.tip)
                                .title(copy::SYS_NO_BUNDLE.short)
                                .small()
                                .icon(icons::icon(role::ERR)),
                        )
                    }),
            ));
        }
        let layout_text = self.form.plan.as_ref().map(|pl| crate::ui::raid::raid_label(pl.request.data_raid)).unwrap_or("Auto");
        form = form.child(kit::field("f-layout", &copy::SYS_DATA_LAYOUT, &app, kit::chip(&p, layout_text)));
        form = form.child(kit::field("f-via", &copy::SYS_BOOT_VIA, &app, kit::tag(&p, Tone::Info, Some(role::CABLE), via)));

        // options most people never touch: collapsed
        let more = v_flex()
            .gap(px(12.0))
            .child(
                Form::vertical()
                    .child(kit::field("f-mirror", &copy::SYS_MIRROR, &app, Input::new(&self.inp.mirror)))
                    .when(f.arch == Arch::Armhf, |fm| fm.child(kit::field("f-dtb", &copy::SYS_DTB, &app, Input::new(&self.inp.dtb)))),
            )
            .when(resolved == BootChoice::Kit, |d| {
                d.child(
                    h_flex()
                        .gap(px(8.0))
                        .child(div().flex_1().child(kit::field("f-kit", &copy::SYS_KIT_DIR, &app, Input::new(&self.inp.kit_dir))))
                        .child(Button::new("browse-kit").outline().icon(icons::icon(role::FOLDER)).on_click(cx.listener(
                            |this, _, window, cx| {
                                let t = this.inp.kit_dir.clone();
                                this.browse_dir(t, window, cx)
                            },
                        ))),
                )
            })
            .when(resolved == BootChoice::Kexec, |d| {
                d.child(h_flex().gap(px(8.0)).child(div().flex_1().child(Input::new(&self.inp.kexec_path))).child(
                    Button::new("browse-kexec").outline().icon(icons::icon(role::FOLDER)).on_click(cx.listener(|this, _, window, cx| {
                        let t = this.inp.kexec_path.clone();
                        this.browse_file(t, window, cx)
                    })),
                ))
            })
            .child(Checkbox::new("fw").label(copy::SYS_FIRMWARE.short).checked(self.sys.with_firmware).on_click(cx.listener(
                |this, v: &bool, _, cx| {
                    this.sys.with_firmware = *v;
                    cx.notify();
                },
            )))
            .child(Checkbox::new("verbose").label(copy::SYS_VERBOSE.short).checked(self.sys.verbose).on_click(cx.listener(
                |this, v: &bool, _, cx| {
                    this.sys.verbose = *v;
                    cx.notify();
                },
            )));
        let body = v_flex()
            .w_full()
            .max_w(px(900.0))
            .gap(px(16.0))
            .child(self.setup_tabs(cx))
            .child(form)
            .child(Accordion::new("sys-more").bordered(true).item(|i| i.title(copy::SYS_MORE.short).child(more)));

        let valid = errors.is_empty();
        Page::new(copy::STEP_SETUP.short, body)
            .subtitle(copy::SYS_SUB.short)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Plan, window, cx)))
                    .into_any_element(),
                self.copy_command_button(cx),
            ])
            .right(vec![kit::with_reason(
                "next-why",
                Button::new("next")
                    .primary()
                    .icon(icons::icon(role::NEXT))
                    .label(copy::SYS_NEXT.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Confirm, window, cx))),
                (!valid).then(|| errors.iter().map(|(_, m)| m.clone()).collect::<Vec<_>>().join(" ")),
            )])
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
