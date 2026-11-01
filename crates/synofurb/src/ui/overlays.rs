//! Dialogs and the Help sheet that are opened from several screens, and the "Copy command" action.

use super::app::App;
use super::cli_cmd::CliCommand;
use super::copy::{self, Item};
use super::disclosure::{dialog, paragraphs};
use super::icons::{self, role};
use super::screens::help::HelpView;
use super::state::{BootChoice, KeyMode, Screen};
use super::theme::layout;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::Input;
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, ClipboardItem, Context, IntoElement, Window, px};
use synofurb_core::plan::StorageMode;

impl App {
    /// The Help sheet, optionally opened on a topic. Pressing F1 while it is open does nothing.
    pub fn open_help_topic(&mut self, topic: Option<&'static str>, window: &mut Window, cx: &mut Context<App>) {
        if window.has_active_sheet(cx) {
            // already open: just show the requested topic
            if let Some(v) = self.help_view.as_ref().and_then(|v| v.upgrade()) {
                v.update(cx, |v, cx| v.show(topic, cx));
                return;
            }
        }
        let topic = topic.or_else(|| self.default_help_topic());
        let app = cx.entity();
        let view = cx.new(|cx| HelpView::new(app, topic, window, cx));
        self.help_view = Some(view.downgrade());
        window.open_sheet(cx, move |sheet, _, _| sheet.title(copy::HELP_TITLE.short).size(px(layout::SHEET_W_WIDE)).child(view.clone()));
    }

    /// The topic F1 opens on the current screen.
    pub fn default_help_topic(&self) -> Option<&'static str> {
        Some(match self.screen {
            Screen::Confirm | Screen::Install | Screen::Failed | Screen::Recover => "recovery",
            Screen::BootMgr => "boot-manager",
            Screen::Inventory | Screen::Connect => "first-install",
            Screen::System => "first-install",
            Screen::Done => "first-install",
            _ => return None,
        })
    }

    pub fn open_keep_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let keep = self.form.req.mode == StorageMode::KeepVolumes;
        window.open_dialog(cx, move |d, _, cx| {
            let app = app.clone();
            d.title(copy::PLAN_KEEP_DIALOG.short).w(px(layout::DIALOG_W)).child(paragraphs(copy::PLAN_KEEP_DIALOG.body, cx)).footer(
                DialogFooter::new()
                    .child(
                        Button::new("keep-cancel")
                            .outline()
                            .label(copy::BTN_CANCEL.short)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("keep-use")
                            .primary()
                            .label(if keep { copy::PLAN_USE_WIPE.short } else { copy::PLAN_USE_KEEP.short })
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                app.update(cx, |a, cx| {
                                    a.set_storage_mode(if keep { StorageMode::WipeAll } else { StorageMode::KeepVolumes }, cx)
                                });
                            }),
                    ),
            )
        });
    }

    /// "What happens next" with the variant that matches what happened.
    pub fn open_next_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let item: &'static Item = if !self.install.past_no_return {
            &copy::DLG_NEXT_PRE
        } else if self.form.req.mode == StorageMode::KeepVolumes {
            &copy::DLG_NEXT_KEEP
        } else {
            &copy::DLG_NEXT_WIPE
        };
        let app = cx.entity();
        dialog(window, cx, item, &app);
    }

    /// "Before you start": once, on the first launch.
    pub fn open_first_launch(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        window.open_dialog(cx, move |d, _, cx| {
            let app = app.clone();
            d.title(copy::FIRST_LAUNCH.short).w(px(layout::DIALOG_W)).child(paragraphs(copy::FIRST_LAUNCH.body, cx)).footer(
                DialogFooter::new()
                    .child(Button::new("fl-help").ghost().icon(icons::icon(role::HELP)).label(copy::BTN_OPEN_HELP.short).on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            app.update(cx, |a, cx| a.open_help_topic(Some("cli"), window, cx));
                        }
                    }))
                    .child(Button::new("fl-ok").primary().label(copy::BTN_GOT_IT.short).on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        app.update(cx, |a, _| {
                            a.settings.first_launch_done = true;
                            a.save_settings();
                        });
                    })),
            )
        });
    }

    /// "Add NAS": probe an address that discovery did not find.
    pub fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let input = self.inp.add_ip.clone();
        window.open_dialog(cx, move |d, _, _| {
            let (app, input2) = (app.clone(), input.clone());
            d.title(copy::INV_ADD_TITLE.short)
                .w(px(layout::DIALOG_W))
                .child(v_flex().gap(px(8.0)).child(copy::INV_ADD_TITLE.body).child(Input::new(&input).cleanable(true)))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("add-cancel")
                                .outline()
                                .label(copy::BTN_CANCEL.short)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("add-ok").primary().label(copy::INV_ADD_BTN.short).on_click(move |_, window, cx| {
                            let ip = input2.read(cx).value().trim().to_string();
                            if !ip.is_empty() {
                                window.close_dialog(cx);
                                app.update(cx, |a, cx| a.select_host(&ip, window, cx));
                            }
                        })),
                )
        });
    }

    /// The command line for the current choices (None until a unit has been inspected).
    pub fn cli_spec(&self, cx: &gpui_kit::App) -> Option<CliCommand> {
        let f = self.facts.as_ref()?;
        let plan = &self.form;
        let key_file = self.use_key.then(|| self.input_value(&self.inp.key_path, cx)).filter(|s| !s.trim().is_empty());
        let authorize_key = match self.sys.key_mode {
            KeyMode::Generate => "generate".to_string(),
            KeyMode::None => "none".to_string(),
            KeyMode::Paste => "<public-key-file>".to_string(),
        };
        let dtb = self.input_value(&self.inp.dtb, cx);
        let method = self.sys.boot.resolve(f);
        let serial = self.input_value(&self.inp.serial_text, cx).trim().to_string();
        let kit = self.input_value(&self.inp.kit_dir, cx).trim().to_string();
        let kexec = self.input_value(&self.inp.kexec_path, cx).trim().to_string();
        Some(CliCommand {
            host: super::format::display_host(&self.input_value(&self.inp.host, cx)),
            port: self.input_value(&self.inp.port, cx).trim().parse().unwrap_or(22),
            user: self.input_value(&self.inp.user, cx).trim().to_string(),
            key_file,
            keep_mode: plan.req.mode == StorageMode::KeepVolumes,
            keep_arrays: plan.keep.iter().cloned().collect(),
            system_gib: plan.system_gib.max(0.0) as u64,
            copies: (!plan.copies_auto).then_some(plan.copies),
            raid: plan.req.data_raid,
            free_percent: plan.req.data_free_percent,
            kernel: plan.kernel,
            hostname: self.input_value(&self.inp.hostname, cx).trim().to_string(),
            username: self.input_value(&self.inp.username, cx).trim().to_string(),
            authorize_key,
            timezone: self.input_value(&self.inp.timezone, cx).trim().to_string(),
            mirror: self.input_value(&self.inp.mirror, cx).trim().to_string(),
            serial,
            serial_speed: self.input_value(&self.inp.serial_speed, cx).trim().parse().unwrap_or(115_200),
            dtb: Some(dtb.trim().to_string()).filter(|d| !d.is_empty()),
            method,
            kit_dir: (method == BootChoice::Kit).then_some(kit).filter(|s| !s.is_empty()),
            kexec: (method == BootChoice::Kexec).then_some(kexec).filter(|s| !s.is_empty()),
            bundle: self.source_offline.then(|| self.input_value(&self.inp.bundle, cx).trim().to_string()).filter(|s| !s.is_empty()),
            no_firmware: !self.sys.with_firmware,
            no_verbose: !self.sys.verbose,
        })
    }

    /// The command to copy on this screen: the read-only `plan` form on Plan and before the confirmation is satisfied,
    /// the `install` form with the three confirmation flags only after it is.
    pub fn cli_text(&self, cx: &gpui_kit::App) -> Option<String> {
        let spec = self.cli_spec(cx)?;
        Some(match self.screen {
            Screen::Plan => spec.plan_command(),
            Screen::System => spec.install_command(None),
            Screen::Confirm => {
                let typed = self.input_value(&self.inp.typed, cx);
                let phrase = self.phrase();
                if self.gate(&typed, &phrase).is_open() { spec.install_command(Some(&phrase)) } else { spec.plan_command() }
            }
            _ => spec.install_command(None),
        })
    }

    /// Ctrl+Shift+C and the "Copy command" buttons.
    pub fn copy_cli_command(&mut self, window: &mut Window, cx: &mut Context<App>) {
        if !matches!(self.screen, Screen::Plan | Screen::System | Screen::Confirm) {
            return;
        }
        if let Some(text) = self.cli_text(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.toast(window, cx, "Command copied", true);
        }
    }

    /// The icon button ("Copy command", terminal icon, tooltip only).
    pub fn copy_command_button(&self, cx: &mut Context<App>) -> gpui_kit::AnyElement {
        let item = if self.screen == Screen::Confirm { &copy::BTN_COPY_CMD_CONFIRM } else { &copy::BTN_COPY_CMD };
        Button::new("copy-cmd")
            .ghost()
            .icon(icons::icon(role::TERMINAL))
            .tooltip(item.tip)
            .accessibility_label(item.short)
            .on_click(cx.listener(|this, _, window, cx| this.copy_cli_command(window, cx)))
            .into_any_element()
    }

    /// The app (not the NAS) could not reach the Internet.
    pub fn offline_app(&self) -> bool {
        self.app_offline || matches!(self.update, super::app::UpdateState::CheckFailed(_))
    }

    pub fn set_storage_mode(&mut self, mode: StorageMode, cx: &mut Context<App>) {
        self.form.req.mode = mode;
        self.rebuild_plan();
        cx.notify();
    }
}
