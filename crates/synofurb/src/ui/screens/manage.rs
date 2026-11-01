//! Manage a converted unit: health (read over SSH with the engine's `manage` module), the management-app update with
//! rollback, and a read-only Storage tab with the Hybrid layout and the hand-off to the web UI. There are NO RAID
//! mutations on the desktop; every array change runs in the web UI.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::mgmt::{self, UpdateGate};
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::empty::{Empty, EmptyHeader, EmptyTitle};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, IntoElement, Window, div, px};
use synofurb_core::manage::{Overall, Smart};

/// The management-app update row's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MgmtState {
    #[default]
    Idle,
    /// 0..=100
    Updating(u8),
    Updated,
    /// Rolled back after a failed health check.
    Failed,
}

const UNIT_VERSION: &str = "0.9.1";
const APP_VERSION: &str = "0.9.2";

fn tone_of(o: Overall) -> Tone {
    match o {
        Overall::Ok => Tone::Ok,
        Overall::Warn => Tone::Warn,
        Overall::Crit => Tone::Err,
        Overall::Unknown => Tone::Neutral,
    }
}

impl App {
    fn health_card(
        &self,
        cx: &mut Context<App>,
        id: &'static str,
        icon: gpui_kit::assets::IconName,
        title: &'static str,
        body: AnyElement,
    ) -> impl IntoElement {
        GroupBox::new()
            .outline()
            .id(id)
            .title(h_flex().gap(px(6.0)).items_center().child(icons::icon(icon)).child(kit::strong(title)))
            .child(if self.manage_report.is_none() && !self.manage_loading {
                v_flex()
                    .gap(px(6.0))
                    .child(Skeleton::new().h(px(16.0)).w(px(160.0)))
                    .child(Skeleton::new().h(px(16.0)).w(px(100.0)))
                    .into_any_element()
            } else {
                body
            })
            .when(false, |d| d.child(cx.theme().foreground.to_string()))
    }

    fn mgmt_row(&self, cx: &mut Context<App>, url: String) -> AnyElement {
        let p = cx.pal();
        let h = self.manage_report.as_ref();
        let state = self.mgmt;
        let demo = self.demo.is_some();
        // Demo: scripted constants. Real runs: the unit's version comes from its health report, the app's from the
        // management plane bundled in this program; nothing is shown that was not read.
        let app_version: String = if demo { APP_VERSION.into() } else { synofurb_core::manage::bundled_mgmt_version().to_string() };
        let cur: String = if demo {
            (if state == MgmtState::Updated { APP_VERSION } else { UNIT_VERSION }).into()
        } else {
            h.map(|h| h.version.clone()).unwrap_or_default()
        };
        let known = !cur.is_empty() && !app_version.is_empty();
        let gate = mgmt::update_gate(h, matches!(state, MgmtState::Updating(_)), known && cur == app_version);
        let (tag_tone, tag_icon, tag_text) = match state {
            _ if !known => (Tone::Neutral, role::TIP, "Version unknown"),
            MgmtState::Updating(_) => (Tone::Info, role::RUNNING, "Updating"),
            MgmtState::Failed => (Tone::Warn, role::WARN, "Not updated"),
            MgmtState::Updated | MgmtState::Idle if cur == app_version => (Tone::Ok, role::OK, "Up to date"),
            _ => (Tone::Info, role::DOWNLOAD, "Update available"),
        };
        let reason = match &gate {
            UpdateGate::Blocked(r) => Some((*r).to_string()),
            _ => None,
        };
        let update = Button::new("mgmt-update")
            .icon(icons::icon(role::DOWNLOAD))
            .label("Update")
            .on_click(cx.listener(|this, _, window, cx| this.start_mgmt_update(window, cx)));
        let update =
            if matches!(gate, UpdateGate::Allowed | UpdateGate::AllowedWithWarning(_)) && h.is_some_and(|h| h.overall == Overall::Ok) {
                update.primary()
            } else {
                update.outline()
            };
        let open = Button::new("open-web")
            .outline()
            .icon(icons::icon(role::EXTERNAL))
            .label(copy::RAID_OPEN_WEB.short)
            .on_click(move |_, _, cx| cx.open_url(&url));
        let mut col = v_flex().gap(px(8.0)).child(
            h_flex()
                .gap(px(12.0))
                .items_center()
                .flex_wrap()
                .child(
                    v_flex()
                        .flex_1()
                        .child(h_flex().gap(px(8.0)).items_center().child(kit::strong("Management app")).child(kit::tag(
                            &p,
                            tag_tone,
                            Some(tag_icon),
                            tag_text,
                        )))
                        .child(
                            h_flex()
                                .gap(px(8.0))
                                .child(kit::caption(cx, "On unit"))
                                .child(kit::mono_sm(cx, if cur.is_empty() { "-".to_string() } else { cur.clone() }))
                                .child(kit::caption(cx, "In this app"))
                                .child(kit::mono_sm(cx, if app_version.is_empty() { "-".to_string() } else { app_version.clone() })),
                        ),
                )
                .children(
                    (state == MgmtState::Updated && self.mgmt_note.as_deref().is_some_and(|n| n.contains("updated"))
                        || demo && state == MgmtState::Updated)
                        .then(|| {
                            Button::new("mgmt-rollback")
                                .ghost()
                                .icon(icons::icon(role::ROTATE_BACK))
                                .label("Roll back")
                                .on_click(cx.listener(|this, _, window, cx| this.confirm_mgmt_rollback(window, cx)))
                                .into_any_element()
                        }),
                )
                .child(kit::with_reason("mgmt-why", update, reason))
                .child(kit::with_reason("web-why", open, h.is_none().then(|| "Unit unreachable.".to_string()))),
        );
        if let UpdateGate::AllowedWithWarning(w) = gate {
            col = col.child(Alert::warning("mgmt-warn", w).small().icon(icons::icon(role::WARN)));
        }
        if let (false, Some(n)) = (state == MgmtState::Failed, self.mgmt_note.as_ref().filter(|_| !demo)) {
            col = col.child(Alert::info("mgmt-note", n.clone()).small().icon(icons::icon(role::INFO)));
        }
        if let MgmtState::Updating(pct) = state {
            col = col.child(Progress::new("mgmt-progress").value(pct as f32)).child(h_flex().gap(px(16.0)).children(
                ["Upload", "Install", "Restart", "Health check"].iter().enumerate().map(|(i, s)| {
                    let k = (pct as usize / 26).min(3);
                    h_flex()
                        .gap(px(2.0))
                        .items_center()
                        .text_xs()
                        .when(i == k, |d| d.font_weight(gpui_kit::FontWeight::SEMIBOLD))
                        .when(i < k, |d| d.child(icons::icon_sized(role::CHECK, 12.0, p.ok_fg)))
                        .child(*s)
                }),
            ));
        }
        if state == MgmtState::Failed {
            col = col.child(
                Alert::warning(
                    "mgmt-failed",
                    self.mgmt_note.clone().unwrap_or_else(|| "Health check timed out. Rolled back to 0.9.1.".into()),
                )
                .title("Update failed.")
                .small()
                .icon(icons::icon(role::ROTATE_BACK)),
            );
        }
        GroupBox::new().outline().id("mgmt").child(col).into_any_element()
    }

    /// "Sign in to read health": the user and secret of THIS unit (the address is the unit's, shown beside it).
    fn manage_signin_form(&self, cx: &mut Context<App>) -> AnyElement {
        use gpui_kit::component::input::Input;
        let app = cx.entity();
        let ip = self.manage_unit.as_ref().and_then(|u| u.primary_ip()).map(|i| i.to_string()).unwrap_or_default();
        let secret: AnyElement = if self.mg_use_key {
            Input::new(&self.inp.mg_key_path).cleanable(true).into_any_element()
        } else {
            Input::new(&self.inp.mg_password).mask_toggle().into_any_element()
        };
        let mode = kit::segmented("mg-mode", &[copy::CONN_PASSWORD.short, copy::CONN_KEY.short], usize::from(self.mg_use_key), {
            let app = app.clone();
            move |ix, _, cx| {
                app.update(cx, |a, cx| {
                    a.mg_use_key = ix == 1;
                    cx.notify();
                })
            }
        });
        let ready = if self.mg_use_key {
            !self.input_value(&self.inp.mg_key_path, cx).trim().is_empty()
        } else {
            !self.input_value(&self.inp.mg_password, cx).is_empty()
        } && !self.input_value(&self.inp.mg_user, cx).trim().is_empty();
        v_flex()
            .gap(px(12.0))
            .max_w(px(480.0))
            .child(
                Alert::info("m-signin", format!("{ip}: {}", copy::MAN_SIGNIN.tip))
                    .title(copy::MAN_SIGNIN.short)
                    .icon(icons::icon(role::LOCK)),
            )
            .child(kit::field_plain(copy::CONN_USER.short, Input::new(&self.inp.mg_user)))
            .child(kit::field_plain(copy::CONN_SIGNIN.short, mode))
            .child(kit::field_plain(if self.mg_use_key { copy::CONN_KEY_FILE.short } else { copy::CONN_PASSWORD.short }, secret))
            .child(
                h_flex().child(kit::with_reason(
                    "m-signin-why",
                    Button::new("m-signin-go")
                        .primary()
                        .icon(icons::icon(role::NEXT))
                        .label(copy::MAN_READ_HEALTH.short)
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in_manage(cx))),
                    (!ready).then(|| copy::MAN_SIGNIN_WHY.tip.to_string()),
                )),
            )
            .into_any_element()
    }

    fn overview_tab(&self, cx: &mut Context<App>, url: String) -> AnyElement {
        let p = cx.pal();
        let mut col = v_flex().gap(px(12.0));
        let Some(h) = self.manage_report.clone() else {
            if self.manage_loading {
                return v_flex()
                    .gap(px(8.0))
                    .child(Skeleton::new().h(px(24.0)).w(px(240.0)))
                    .child(Skeleton::new().h(px(16.0)).w(px(160.0)))
                    .into_any_element();
            }
            if !self.manage_signed_in {
                return self.manage_signin_form(cx);
            }
            // signed in, but the read failed: show the unit's own error, never a guess about power or cables
            let why = self.manage_err.clone().unwrap_or_default();
            return col
                .child(Alert::error("unreach", why).title(copy::MAN_READ_FAILED.short).icon(icons::icon(role::OFFLINE)))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .child(
                            Button::new("retry")
                                .outline()
                                .icon(icons::icon(role::REFRESH))
                                .label(copy::INSTALL_RETRY.short)
                                .on_click(cx.listener(|this, _, _, cx| this.read_manage_health(cx))),
                        )
                        .child(Button::new("change-signin").ghost().label(copy::MAN_CHANGE_SIGNIN.short).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.manage_signed_in = false;
                                this.manage_err = None;
                                cx.notify();
                            },
                        )))
                        .child(
                            Button::new("rescan")
                                .ghost()
                                .label(copy::INV_SCAN_AGAIN.short)
                                .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Inventory, window, cx))),
                        ),
                )
                .into_any_element();
        };
        let s = h.summary();
        let (ov_text, ov_icon) = match h.overall {
            Overall::Ok => ("Healthy", role::OK),
            Overall::Warn => ("Needs attention", role::WARN),
            Overall::Crit => ("Critical", role::STOP),
            Overall::Unknown => ("Unknown", role::OFFLINE),
        };
        let top = h_flex()
            .gap(px(8.0))
            .items_center()
            .flex_wrap()
            .child(kit::tag(&p, tone_of(h.overall), Some(ov_icon), ov_text))
            .children(h.warnings.iter().map(|w| kit::tag(&p, tone_of(h.overall), Some(role::WARN), w.clone())))
            .child(div().flex_1())
            .child(
                Button::new("recheck")
                    .ghost()
                    .icon(icons::icon(role::REFRESH))
                    .tooltip("Check again.")
                    .accessibility_label("Check again")
                    .on_click(cx.listener(|this, _, _, cx| this.read_manage_health(cx))),
            );
        col = col.child(GroupBox::new().outline().id("overall").child(top)).child(self.mgmt_row(cx, url));

        let disks = v_flex().gap(px(4.0)).children(h.disks.iter().map(|d| {
            let (tone, icon, text) = match d.smart {
                Smart::Passed => (Tone::Ok, role::OK, "OK"),
                Smart::Failed => (Tone::Err, role::ERR, "Failed"),
                Smart::Unknown => (Tone::Neutral, role::TIP, "Unknown"),
            };
            h_flex()
                .gap(px(8.0))
                .items_center()
                .child(kit::mono(cx, d.dev.trim_start_matches("/dev/").to_string()).w(px(48.0)))
                .child(kit::mono_sm(cx, d.temp_c.map(|t| format!("{t:.0} \u{b0}C")).unwrap_or_else(|| "-".into())))
                .child(div().flex_1())
                .child(kit::tag(&p, tone, Some(icon), text))
        }));
        let arrays = v_flex().gap(px(6.0)).children(h.arrays.iter().map(|a| {
            let (tone, icon, text) = if a.degraded {
                (Tone::Err, role::ERR, "Degraded")
            } else if a.rebuilding() {
                (Tone::Info, role::REFRESH, "Rebuilding")
            } else {
                (Tone::Ok, role::OK, "Clean")
            };
            v_flex()
                .gap(px(4.0))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::mono(cx, a.name.clone()))
                        .child(kit::caption(cx, a.level.to_uppercase()))
                        .child(div().flex_1())
                        .child(kit::tag(&p, tone, Some(icon), text)),
                )
                .children(a.sync_pct.filter(|_| a.rebuilding()).map(|pc| {
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(div().flex_1().child(Progress::new(("rb", a.name.len())).value(pc as f32)))
                        .child(kit::mono_sm(cx, format!("{pc:.0}%")))
                }))
        }));
        let volumes = v_flex().gap(px(6.0)).children(h.volumes.iter().map(|v| {
            let pc = v.used_pct.unwrap_or(0.0) as f32;
            h_flex()
                .gap(px(8.0))
                .items_center()
                .child(kit::mono(cx, v.mount.clone()).w(px(96.0)))
                .child(div().flex_1().child(Progress::new(("vol", v.mount.len())).value(pc)))
                .child(kit::mono_sm(cx, format!("{pc:.0}%")))
        }));
        let services = h_flex().gap(px(6.0)).flex_wrap().children(h.services.iter().enumerate().map(|(i, sv)| {
            let up = sv.active != Some(false) || self.svc_fixed;
            h_flex()
                .gap(px(4.0))
                .items_center()
                .child(kit::tag(&p, if up { Tone::Ok } else { Tone::Err }, Some(if up { role::OK } else { role::ERR }), sv.name.clone()))
                .children((!up).then(|| {
                    Button::new(("svc", i)).outline().xsmall().label("Restart").on_click(cx.listener(|this, _, _, cx| {
                        this.svc_fixed = true;
                        cx.notify();
                    }))
                }))
        }));
        let upd = v_flex().gap(px(4.0)).child(kit::mono(cx, format!("{} pending", h.updates.apt_pending.unwrap_or(0)))).children(
            h.updates.security_pending.filter(|n| *n > 0).map(|n| kit::tag(&p, Tone::Warn, Some(role::WARN), format!("{n} security"))),
        );
        let boot = v_flex()
            .gap(px(2.0))
            .child(div().text_sm().child(format!("{} \u{b7} {}", if h.os.is_empty() { "Debian 13" } else { &h.os }, h.kernel)))
            .child(kit::caption(cx, "DSM loader entry remains"));
        let grid = v_flex()
            .gap(px(12.0))
            .child(
                h_flex()
                    .gap(px(12.0))
                    .items_start()
                    .child(div().flex_1().child(self.health_card(cx, "c-disks", role::DISK, "Disks", disks.into_any_element())))
                    .child(div().flex_1().child(self.health_card(cx, "c-arrays", role::LAYERS, "Arrays", arrays.into_any_element()))),
            )
            .child(
                h_flex()
                    .gap(px(12.0))
                    .items_start()
                    .child(div().flex_1().child(self.health_card(cx, "c-vols", role::DATABASE, "Volumes", volumes.into_any_element())))
                    .child(div().flex_1().child(self.health_card(cx, "c-svc", role::HEARTBEAT, "Services", services.into_any_element()))),
            )
            .child(
                h_flex()
                    .gap(px(12.0))
                    .items_start()
                    .child(div().flex_1().child(self.health_card(cx, "c-upd", role::PACKAGE, "Updates", upd.into_any_element())))
                    .child(div().flex_1().child(self.health_card(cx, "c-boot", role::POWER, "Boot default", boot.into_any_element()))),
            );
        let _ = s;
        col.child(grid).into_any_element()
    }

    /// Storage: arrays and disks, read-only; every change runs in the web UI.
    fn storage_tab(&self, cx: &mut Context<App>, url: String) -> AnyElement {
        let p = cx.pal();
        let Some(h) = self.manage_report.clone() else {
            return Alert::error("st-unreach", "Check power, cable and network.")
                .title(copy::RAID_UNREACHABLE.short)
                .icon(icons::icon(role::OFFLINE))
                .into_any_element();
        };
        let degraded = h.arrays.iter().any(|a| a.degraded);
        let arrays = Table::new()
            .child(
                TableHeader::new().child(
                    TableRow::new()
                        .child(TableHead::new().child(copy::RAID_ARRAY.short))
                        .child(TableHead::new().child(copy::RAID_LEVEL.short))
                        .child(TableHead::new().child(copy::RAID_STATE.short))
                        .child(TableHead::new().text_right().child(copy::RAID_PROGRESS.short)),
                ),
            )
            .child(TableBody::new().children(h.arrays.iter().map(|a| {
                let (tone, icon, text) = if a.degraded {
                    (Tone::Err, role::ERR, "Degraded")
                } else if a.rebuilding() {
                    (Tone::Info, role::REFRESH, "Rebuilding")
                } else {
                    (Tone::Ok, role::OK, "Clean")
                };
                TableRow::new()
                    .child(TableCell::new().child(kit::mono(cx, a.name.clone())))
                    .child(TableCell::new().child(a.level.to_uppercase()))
                    .child(TableCell::new().child(kit::tag(&p, tone, Some(icon), text)))
                    .child(TableCell::new().text_right().child(kit::mono(
                        cx,
                        a.sync_pct.filter(|_| a.rebuilding()).map(|p| format!("{p:.0}%")).unwrap_or_else(|| "-".into()),
                    )))
            })));
        let disks = Table::new()
            .child(
                TableHeader::new().child(
                    TableRow::new()
                        .child(TableHead::new().child(copy::INSPECT_DISK.short))
                        .child(TableHead::new().child("In arrays"))
                        .child(TableHead::new().child("SMART")),
                ),
            )
            .child(TableBody::new().children(h.disks.iter().map(|d| {
                let (tone, icon, text) = match d.smart {
                    Smart::Passed => (Tone::Ok, role::OK, "OK"),
                    Smart::Failed => (Tone::Err, role::ERR, "Failed"),
                    Smart::Unknown => (Tone::Neutral, role::TIP, "Unknown"),
                };
                TableRow::new()
                    .child(TableCell::new().child(kit::mono(cx, d.dev.trim_start_matches("/dev/").to_string())))
                    .child(TableCell::new().child(kit::mono_sm(cx, h.arrays.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(" "))))
                    .child(TableCell::new().child(kit::tag(&p, tone, Some(icon), text)))
            })));
        let mut col = v_flex().gap(px(12.0));
        if degraded {
            col = col.child(
                Alert::warning("st-degraded", "Replace the disk in the web UI.")
                    .title(copy::RAID_DEGRADED_ALERT.short)
                    .icon(icons::icon(role::WARN)),
            );
        }
        col = col.child(arrays).child(disks);
        if self.manage_hybrid {
            col = col.child(
                GroupBox::new().outline().id("hyb").child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::tag(&p, Tone::Neutral, Some(role::LAYERS), "Hybrid 1-disk"))
                        .child(kit::mono_sm(cx, "LVM vg_data \u{b7} /srv/data \u{b7} 8.00 TiB"))
                        .child(crate::ui::disclosure::info_tip("hyb-info", &copy::PLAN_HYBRID_TIP, &cx.entity())),
                ),
            );
        }
        col.child(
            h_flex().gap(px(8.0)).items_center().child(kit::caption(cx, copy::RAID_ACTIONS.tip)).child(
                Button::new("st-web")
                    .primary()
                    .icon(icons::icon(role::EXTERNAL))
                    .label(copy::RAID_OPEN_WEB.short)
                    .tooltip(copy::RAID_OPEN_WEB.tip)
                    .on_click(move |_, _, cx| cx.open_url(&format!("{url}/#raid"))),
            ),
        )
        .into_any_element()
    }

    pub(crate) fn page_manage(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let unit = self.manage_unit.clone();
        let title = unit.as_ref().map(|d| if d.model.is_empty() { d.title() } else { d.model.clone() }).unwrap_or_else(|| "NAS".into());
        let ip = unit.as_ref().and_then(|d| d.primary_ip()).map(|i| i.to_string()).unwrap_or_default();
        let url = synofurb_core::manage::web_url(&ip);
        let tab = self.manage_tab;
        let content: AnyElement = match tab {
            0 => self.overview_tab(cx, url.clone()),
            1 => self.storage_tab(cx, url.trim_end_matches('/').to_string()),
            2 => v_flex()
                .gap(px(8.0))
                .child(
                    h_flex()
                        .gap(px(4.0))
                        .items_center()
                        .child(kit::labelled("m-ssh", &copy::MAN_SSH_CMD, &app))
                        .child(kit::mono(cx, format!("ssh <user>@{ip}")))
                        .child(Clipboard::new("m-copy").value(format!("ssh <user>@{ip}"))),
                )
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::labelled("m-web", &copy::MAN_WEB_UI, &app))
                        .child(kit::mono(cx, url.clone())),
                )
                .child(
                    h_flex()
                        .gap(px(2.0))
                        .child(kit::tag(&p, Tone::Neutral, Some(role::LOCK), copy::MAN_PW_NOT_KEPT.short))
                        .child(crate::ui::disclosure::info_tip("m-pw", &copy::MAN_PW_NOT_KEPT, &app)),
                )
                .into_any_element(),
            3 => Button::new("m-rec")
                .outline()
                .icon(icons::icon(role::HELP))
                .label(copy::MAN_RECOVERY.short)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.recover_for_manage = true;
                    this.go(Screen::Recover, window, cx)
                }))
                .into_any_element(),
            _ => Empty::new().header(EmptyHeader::new().title(EmptyTitle::new().child(copy::MAN_NO_HISTORY.short))).into_any_element(),
        };
        let tabs = TabBar::new("manage-tabs")
            .underline()
            .selected_index(tab)
            .children(
                [
                    copy::MAN_OVERVIEW.short,
                    copy::MAN_STORAGE.short,
                    copy::MAN_LOGINS.short,
                    copy::MAN_RECOVERY.short,
                    copy::MAN_HISTORY.short,
                ]
                .map(|l| Tab::new().label(l)),
            )
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.manage_tab = *ix;
                cx.notify();
            }));
        let body = v_flex().w_full().gap(px(16.0)).child(tabs).child(content);
        Page::new(copy::MAN_TITLE.short, body)
            .subtitle(format!("{title} \u{b7} {}", copy::MAN_SUB.short))
            .left(vec![
                Button::new("m-diag")
                    .ghost()
                    .icon(icons::icon(role::COPY))
                    .label(copy::MAN_DIAG.short)
                    .tooltip(copy::MAN_DIAG.tip)
                    .on_click(cx.listener(|this, _, window, cx| this.copy_diagnostics(window, cx)))
                    .into_any_element(),
            ])
            .right(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(this.return_to, window, cx)))
                    .into_any_element(),
            ])
    }

    /// Read the health of the unit on screen (demo: the sample report; real: over SSH with the sign-in typed on this page,
    /// never the Connect form). A missing credential is the sign-in state, not an error and not "unreachable".
    pub fn read_manage_health(&mut self, cx: &mut Context<App>) {
        if self.demo.is_some() {
            self.manage_report = Some(mgmt::demo_health(mgmt::DemoHealth::Ok));
            self.manage_err = None;
            cx.notify();
            return;
        }
        match self.target(cx) {
            Ok(t) => {
                self.manage_loading = true;
                self.manage_err = None;
                crate::ui::worker::spawn_manage_health(self.tx.clone(), t);
            }
            Err(e) => {
                self.manage_report = None;
                self.manage_signed_in = false;
                self.manage_err = Some(e);
            }
        }
        cx.notify();
    }

    /// "Read health": accept the typed sign-in for the unit on screen and read it.
    pub fn sign_in_manage(&mut self, cx: &mut Context<App>) {
        self.manage_signed_in = true;
        self.manage_err = None;
        self.read_manage_health(cx);
    }

    /// Demo: a scripted update. A real run calls the engine's `push_update` on a worker thread (it rolls the unit back by
    /// itself when the new version does not answer) and shows the engine's own report.
    pub fn start_mgmt_update(&mut self, window: &mut Window, cx: &mut Context<App>) {
        use std::time::Duration;
        if matches!(self.mgmt, MgmtState::Updating(_)) {
            return;
        }
        self.mgmt_note = None;
        if self.demo.is_none() {
            match self.target(cx) {
                Ok(t) => {
                    self.mgmt = MgmtState::Updating(4);
                    crate::ui::worker::spawn_mgmt_update(self.tx.clone(), t, false);
                }
                Err(e) => {
                    self.mgmt = MgmtState::Failed;
                    self.mgmt_note = Some(e);
                }
            }
            cx.notify();
            return;
        }
        self.mgmt = MgmtState::Updating(4);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            for pct in [18u8, 40, 62, 85, 100] {
                cx.background_executor().timer(Duration::from_millis(700)).await;
                if this
                    .update(cx, |t, cx| {
                        t.mgmt = if pct == 100 { MgmtState::Updated } else { MgmtState::Updating(pct) };
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// "Roll back?": asks, then (demo) resets the scripted state or (real) calls `push_rollback`.
    pub fn confirm_mgmt_rollback(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let app = cx.entity();
        let app2 = app.clone();
        crate::ui::disclosure::dialog_with(window, cx, &copy::DLG_MGMT_ROLLBACK, &app, copy::MGMT_ROLLBACK_OK.short, move |cx| {
            app2.update(cx, |a, cx| {
                a.mgmt_note = None;
                if a.demo.is_some() {
                    a.mgmt = MgmtState::Idle;
                } else {
                    match a.target(cx) {
                        Ok(t) => {
                            a.mgmt = MgmtState::Updating(4);
                            crate::ui::worker::spawn_mgmt_update(a.tx.clone(), t, true);
                        }
                        Err(e) => {
                            a.mgmt = MgmtState::Failed;
                            a.mgmt_note = Some(e);
                        }
                    }
                }
                cx.notify();
            })
        });
    }
}
