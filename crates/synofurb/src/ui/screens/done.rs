//! D10 Done: the logins, shown once. The password must be copied before Finish enables.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure::paragraphs;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::theme::Themed;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, IntoElement, Window, div, px};
use synofurb_core::model::Firmware;

/// Percent-encode for a URL query value.
pub fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

impl App {
    /// The pre-filled "compat report" issue: model, DSM version, synofurb version, result, boot method. No serial.
    pub fn report_url(&self) -> String {
        let (model, dsm, method) = self
            .facts
            .as_ref()
            .map(|f| (f.model.clone(), f.dsm_version.clone(), self.sys.boot.resolve(f).label().to_string()))
            .unwrap_or_default();
        let result = if matches!(self.result, Some(Ok(_))) { "installed" } else { "failed" };
        let body =
            format!("model: {model}\ndsm: {dsm}\nsynofurb: {}\nresult: {result}\nboot method: {method}\n", super::super::app::version());
        format!(
            "https://github.com/andreaswiren/synofurb/issues/new?template=compat-report.yml&title={}&body={}",
            pct(&format!("Compat report: {model}")),
            pct(&body)
        )
    }

    fn done_row(&self, cx: &mut Context<App>, id: &'static str, value: String, what: &'static str) -> AnyElement {
        h_flex()
            .gap(px(4.0))
            .items_center()
            .child(kit::mono(cx, value.clone()).flex_1().min_w_0())
            .child(Clipboard::new(id).value(value).tooltip("Copy").on_copied(move |_, _, _| {
                let _ = what;
            }))
            .into_any_element()
    }

    pub(crate) fn page_done(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let app = cx.entity();
        let Some(Ok(o)) = self.result.clone() else {
            return Page::new(copy::DONE_TITLE.short, kit::muted(cx, "-"));
        };
        let ip = o.ip.map(|i| i.to_string()).unwrap_or_else(|| "?".into());
        let ssh_cmd = match &self.sys.generated_key {
            Some((_, path)) => format!("ssh -i {} {}@{}", crate::ui::format::shorten_path(&path.display().to_string(), 40), o.username, ip),
            None => format!("ssh {}@{}", o.username, ip),
        };
        let label = |id: &'static str, it: &'static copy::Item| kit::labelled(id, it, &app);
        let mut list = DescriptionList::horizontal()
            .columns(1)
            .bordered(false)
            .child(DescriptionItem::new(copy::DONE_ADDRESS.short).value(self.done_row(cx, "c-addr", ip.clone(), "Address")))
            .child(DescriptionItem::new(copy::DONE_USER.short).value(kit::strong(o.username.clone()).into_any_element()))
            .child(
                DescriptionItem::new(label("d-pw", &copy::DONE_PASSWORD)).value(
                    h_flex()
                        .gap(px(4.0))
                        .items_center()
                        .child(div().w(px(260.0)).child(Input::new(&self.inp.new_password).readonly(true).mask_toggle()))
                        .child(Clipboard::new("c-pw").value(self.input_value(&self.inp.new_password, cx)).tooltip("Copy").on_copied({
                            let app = app.clone();
                            move |_, _, cx| {
                                app.update(cx, |a, cx| {
                                    a.password_copied = true;
                                    cx.notify();
                                })
                            }
                        }))
                        .into_any_element(),
                ),
            )
            .child(DescriptionItem::new(copy::DONE_SSH.short).value(self.done_row(cx, "c-ssh", ssh_cmd, "SSH command")));
        if let Some(fp) = &o.ssh_fingerprint {
            list = list.child(DescriptionItem::new(label("d-hk", &copy::DONE_HOST_KEY)).value(self.done_row(
                cx,
                "c-fp",
                fp.clone(),
                "Fingerprint",
            )));
        }
        if let Some(u) = &o.mgmt_url {
            let uu = u.clone();
            list = list.child(
                DescriptionItem::new(label("d-web", &copy::DONE_WEB_UI)).value(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::mono(cx, u.clone()))
                        .child(
                            Button::new("open-web")
                                .outline()
                                .small()
                                .icon(icons::icon(role::EXTERNAL))
                                .label(copy::DONE_OPEN.short)
                                .on_click(move |_, _, cx| cx.open_url(&uu)),
                        )
                        .into_any_element(),
                ),
            );
        }

        let mut next = vec!["Open the web UI.".to_string(), "Replace warned disks.".into()];
        next.push(if self.facts.as_ref().is_some_and(|f| f.firmware == Firmware::Dom) {
            "Check the BIOS boot order: the internal Synology flash may take precedence.".into()
        } else {
            "Check the BIOS boot order.".into()
        });
        let next_ui = v_flex().gap(px(2.0)).text_sm().children(next.into_iter().map(|l| div().child(l)));
        let acc = Accordion::new("done-acc")
            .bordered(true)
            .item(|i| i.title(h_flex().gap(px(2.0)).child(copy::DONE_NEXT_STEPS.short)).child(next_ui));

        let mut body = v_flex().w_full().max_w(px(680.0)).gap(px(12.0));
        if !o.mgmt_reachable {
            body = body.child(Alert::warning("web-wait", "Web UI not answering yet.").small().icon(icons::icon(role::WARN)));
        }
        body = body.child(list);
        if !self.install.uboot_lines.is_empty() {
            let lines = self.install.uboot_lines.clone();
            body = body.child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(Alert::warning("arm-action", copy::DONE_ARM_ACTION.short).small().icon(icons::icon(role::WARN)).flex_1())
                    .child(Button::new("arm-cmds").outline().label(copy::DONE_SHOW_COMMANDS.short).on_click(move |_, window, cx| {
                        let lines = lines.clone();
                        window.open_dialog(cx, move |d, _, cx| {
                            let text = lines.join("\n");
                            d.bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
                                .title("U-Boot commands")
                                .w(px(560.0))
                                .child(
                                    v_flex()
                                        .gap(px(8.0))
                                        .child(
                                            div()
                                                .p(px(8.0))
                                                .rounded(cx.theme_radius())
                                                .bg(cx.pal().log_bg)
                                                .text_color(cx.pal().log_fg)
                                                .font_family(gpui_kit::component::ActiveTheme::theme(cx).mono_font_family.clone())
                                                .text_xs()
                                                .children(lines.iter().map(|l| div().child(l.clone()))),
                                        )
                                        .child(Clipboard::new("c-uboot").value(text)),
                                )
                                .footer(
                                    DialogFooter::new().child(
                                        Button::new("ub-close")
                                            .primary()
                                            .label(copy::BTN_CLOSE.short)
                                            .on_click(|_, window, cx| window.close_dialog(cx)),
                                    ),
                                )
                        });
                    })),
            );
        }
        body = body.child(acc).child(h_flex().gap(px(8.0)).items_center().child(
            Checkbox::new("pw-copied").label(copy::DONE_COPIED.short).checked(self.password_copied).on_click(cx.listener(
                |this, v: &bool, _, cx| {
                    this.password_copied = *v;
                    cx.notify();
                },
            )),
        ));
        let copied = self.password_copied;
        Page::new(copy::DONE_TITLE.short, body)
            .subtitle(copy::DONE_SUB.short)
            .left(vec![
                Button::new("save-logins")
                    .ghost()
                    .icon(icons::icon(role::SAVE))
                    .label(copy::DONE_SAVE.short)
                    .tooltip(copy::DONE_SAVE.tip)
                    .on_click(cx.listener(|this, _, window, cx| this.save_logins(window, cx)))
                    .into_any_element(),
            ])
            .right(vec![
                Button::new("report")
                    .outline()
                    .label(copy::DONE_REPORT.short)
                    .tooltip(copy::DONE_REPORT.tip)
                    .on_click(cx.listener(|this, _, window, cx| this.open_report_dialog(window, cx)))
                    .into_any_element(),
                kit::with_reason(
                    "finish-why",
                    Button::new("finish")
                        .primary()
                        .label(copy::DONE_FINISH.short)
                        .on_click(cx.listener(|this, _, window, cx| this.start_over(window, cx))),
                    (!copied).then(|| copy::DONE_FINISH.tip.to_string()),
                ),
            ])
    }

    pub fn open_report_dialog(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let url = self.report_url();
        window.open_dialog(cx, move |d, _, cx| {
            let url = url.clone();
            d.bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
                .title(copy::DLG_REPORT.short)
                .w(px(crate::ui::theme::layout::DIALOG_W))
                .child(paragraphs(copy::DLG_REPORT.body, cx))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("rep-cancel")
                                .outline()
                                .label(copy::BTN_CANCEL.short)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("rep-open").primary().icon(icons::icon(role::EXTERNAL)).label(copy::BTN_OPEN_ISSUE.short).on_click(
                                move |_, window, cx| {
                                    cx.open_url(&url);
                                    window.close_dialog(cx);
                                },
                            ),
                        ),
                )
        });
    }
}

trait ThemeRadius {
    fn theme_radius(&self) -> gpui_kit::Pixels;
}
impl ThemeRadius for gpui_kit::App {
    fn theme_radius(&self) -> gpui_kit::Pixels {
        gpui_kit::component::ActiveTheme::theme(self).radius
    }
}

#[cfg(test)]
mod tests {
    use super::pct;

    #[test]
    fn percent_encoding_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(pct("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(pct("a b\n&="), "a%20b%0A%26%3D");
    }
}
