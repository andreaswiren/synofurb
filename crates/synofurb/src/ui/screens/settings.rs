//! D12 Settings: the stock `Settings` widget (pages General, Updates, Data, Logs, Privacy). Changes apply at once.
//! Rows are `SettingItem::render` rows so every label can carry its info icon instead of a paragraph of help.

use crate::ui::app::{App, UpdateState, version};
use crate::ui::copy::{self, Item};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::theme::{TextScale, ThemeChoice};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Entity, IntoElement, SharedString, Window, div, px};
use synofurb_core::compat::DbOrigin;

fn row(
    app: &Entity<App>,
    id: &'static str,
    item: &'static Item,
    control: impl Fn(&Entity<App>, &mut Window, &mut gpui_kit::App) -> AnyElement + 'static,
) -> SettingItem {
    let app = app.clone();
    SettingItem::render(move |_, window, cx| {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .child(kit::labelled(id, item, &app))
            .child(control(&app, window, cx))
            .into_any_element()
    })
}

impl App {
    pub(crate) fn page_settings(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let app = cx.entity();
        let theme_ix = ThemeChoice::ALL.iter().position(|t| *t == self.settings.theme).unwrap_or(0);
        let scale_ix = TextScale::ALL.iter().position(|t| *t == self.settings.text_scale).unwrap_or(0);
        let accent_ix = crate::ui::theme::Accent::ALL.iter().position(|a| *a == self.settings.accent).unwrap_or(0);
        let beta = self.settings.update_channel == "prerelease";
        let reduce = self.settings.reduce_motion;
        let check = self.settings.check_updates;
        let send = self.settings.send_results;
        let list_note = format!("{} \u{00b7} {} models", self.compat.updated(), self.compat.len());
        let origin = match self.compat.origin() {
            DbOrigin::Embedded => "built in",
            DbOrigin::Cache => "cached",
        };
        let refreshing = self.compat_refreshing;
        let offline = self.offline_app();
        let (upd_text, upd_available) = match &self.update {
            UpdateState::Available(i) => (format!("{} available", i.latest), true),
            UpdateState::UpToDate => (format!("{} is current", version()), false),
            UpdateState::Checking => ("checking".into(), false),
            UpdateState::CheckFailed(_) => ("not checked".into(), false),
            _ => (version().to_string(), false),
        };
        let logs = self.cfg_dir.join("logs");
        let logs_s = logs.display().to_string();
        let bundle_input = self.inp.bundle.clone();
        let auto_state = self.settings_auto.clone();

        let general = SettingPage::new(copy::SET_GENERAL.short).icon(icons::icon(role::SETTINGS)).group(
            SettingGroup::new()
                .item(row(&app, "s-theme", &copy::SET_THEME, move |app, _, _| {
                    let app = app.clone();
                    kit::segmented(
                        "theme",
                        &[copy::SET_SYSTEM.short, copy::SET_LIGHT.short, copy::SET_DARK.short],
                        theme_ix,
                        move |ix, _, cx| app.update(cx, |a, cx| a.set_theme(ThemeChoice::ALL[ix], cx)),
                    )
                    .into_any_element()
                }))
                .item(row(&app, "s-size", &copy::SET_TEXT_SIZE, move |app, _, _| {
                    let app = app.clone();
                    kit::segmented(
                        "text-size",
                        &[copy::SET_DEFAULT.short, copy::SET_LARGE.short, copy::SET_LARGEST.short],
                        scale_ix,
                        move |ix, window, cx| {
                            app.update(cx, |a, cx| {
                                a.settings.text_scale = TextScale::ALL[ix];
                                a.save_settings();
                                let dark = a.settings.theme.is_dark(crate::ui::theme::os_is_dark(cx));
                                crate::ui::theme::apply(dark, a.settings.text_scale, a.settings.accent, cx);
                                let _ = &window;
                                cx.notify();
                            })
                        },
                    )
                    .into_any_element()
                }))
                .item(row(&app, "s-accent", &copy::SET_ACCENT, move |app, _, _| {
                    let app = app.clone();
                    kit::segmented("accent", &["Purple", "Blue", "Green", "Amber", "Rose"], accent_ix, move |ix, _, cx| {
                        app.update(cx, |a, cx| {
                            a.settings.accent = crate::ui::theme::Accent::ALL[ix];
                            a.save_settings();
                            let dark = a.settings.theme.is_dark(crate::ui::theme::os_is_dark(cx));
                            crate::ui::theme::apply(dark, a.settings.text_scale, a.settings.accent, cx);
                            cx.notify();
                        })
                    })
                    .into_any_element()
                }))
                .item(row(&app, "s-motion", &copy::SET_MOTION, move |app, _, _| {
                    let app = app.clone();
                    Switch::new("reduce-motion")
                        .checked(reduce)
                        .on_click(move |v: &bool, _, cx| {
                            let v = *v;
                            app.update(cx, |a, cx| {
                                a.settings.reduce_motion = v;
                                a.save_settings();
                                cx.notify();
                            })
                        })
                        .into_any_element()
                })),
        );

        let updates = SettingPage::new(copy::SET_UPDATES.short).icon(icons::icon(role::REFRESH)).group(
            SettingGroup::new()
                .item(row(&app, "s-channel", &copy::SET_CHANNEL, move |app, _, _| {
                    let app = app.clone();
                    kit::segmented("channel", &[copy::SET_STABLE.short, copy::SET_BETA.short], usize::from(beta), move |ix, _, cx| {
                        app.update(cx, |a, cx| {
                            a.settings.update_channel = if ix == 1 { "prerelease".into() } else { "stable".into() };
                            a.save_settings();
                            cx.notify();
                        })
                    })
                    .into_any_element()
                }))
                .item(row(&app, "s-auto", &copy::SET_AUTO_UPDATE, move |_, _, _| match &auto_state {
                    Some(st) => {
                        div().w(px(180.0)).child(Select::new(st).accessibility_label(copy::SET_AUTO_UPDATE.short)).into_any_element()
                    }
                    None => div().into_any_element(),
                }))
                .item(row(&app, "s-list", &copy::SET_COMPAT_LIST, move |app, _, cx| {
                    let app = app.clone();
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(kit::caption(cx, format!("{list_note} \u{00b7} {origin}")))
                        .child(
                            Button::new("refresh-compat")
                                .outline()
                                .small()
                                .icon(icons::icon(role::REFRESH))
                                .label(copy::SET_REFRESH.short)
                                .loading(refreshing)
                                .disabled(refreshing || offline)
                                .tooltip(if offline { copy::TAG_OFFLINE.short } else { "" })
                                .on_click(move |_, _, cx| app.update(cx, |a, cx| a.refresh_compat(cx))),
                        )
                        .into_any_element()
                }))
                .item(SettingItem::render({
                    let app = app.clone();
                    let text = upd_text.clone();
                    move |_, _, cx| {
                        let app2 = app.clone();
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(kit::muted(cx, text.clone()))
                            .child(
                                Button::new("check-now")
                                    .outline()
                                    .small()
                                    .icon(icons::icon(role::REFRESH))
                                    .label(copy::MENU_UPDATES.short)
                                    .on_click(move |_, _, cx| app2.update(cx, |a, cx| a.check_updates_now(cx)))
                                    .when(upd_available, |b| b.primary()),
                            )
                            .into_any_element()
                    }
                })),
        );
        let _ = check;

        let data = SettingPage::new(copy::SET_DATA.short).icon(icons::icon(role::DATABASE)).group(SettingGroup::new().item(row(
            &app,
            "s-bundle",
            &copy::SET_BUNDLE,
            move |app, _, _| {
                let app = app.clone();
                let input = bundle_input.clone();
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(div().w(px(260.0)).child(Input::new(&input)))
                    .child(
                        Button::new("bundle-browse")
                            .outline()
                            .small()
                            .icon(icons::icon(role::FOLDER))
                            .accessibility_label(copy::CONN_BROWSE.short)
                            .on_click(move |_, window, cx| {
                                let input = input.clone();
                                app.update(cx, |a, cx| a.browse_dir(input, window, cx))
                            }),
                    )
                    .child(
                        Button::new("bundle-dl")
                            .ghost()
                            .small()
                            .label(copy::SET_BUNDLE_DOWNLOAD.short)
                            .disabled(true)
                            .tooltip("Run `synofurb-cli bundle` to build or download one."),
                    )
                    .into_any_element()
            },
        )));

        let logs_page = SettingPage::new(copy::SET_LOGS.short).icon(icons::icon(role::LOG)).group(SettingGroup::new().item(row(
            &app,
            "s-logs",
            &copy::SET_LOG_FOLDER,
            move |_, _, cx| {
                let logs = logs.clone();
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(kit::mono_sm(cx, logs_s.clone()).max_w(px(320.0)).truncate())
                    .child(Button::new("open-logs").outline().small().icon(icons::icon(role::FOLDER)).label(copy::SET_OPEN.short).on_click(
                        move |_, _, cx| {
                            let _ = std::fs::create_dir_all(&logs);
                            cx.reveal_path(&logs)
                        },
                    ))
                    .into_any_element()
            },
        )));

        let privacy = SettingPage::new(copy::SET_PRIVACY.short).icon(icons::icon(role::LOCK)).group(SettingGroup::new().item(row(
            &app,
            "s-send",
            &copy::SET_SEND_RESULTS,
            move |app, _, _| {
                let app = app.clone();
                Switch::new("send-results")
                    .checked(send)
                    .on_click(move |v: &bool, _, cx| {
                        let v = *v;
                        app.update(cx, |a, cx| {
                            a.settings.send_results = v;
                            a.save_settings();
                            cx.notify();
                        })
                    })
                    .into_any_element()
            },
        )));

        let body = div()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(Settings::new("settings").sidebar_width(px(180.0)).pages([general, updates, data, logs_page, privacy]));
        Page::new(copy::SET_TITLE.short, body).no_scroll().left(vec![
            Button::new("back")
                .ghost()
                .icon(icons::icon(role::BACK))
                .label(copy::BTN_BACK.short)
                .on_click(cx.listener(|this, _, window, cx| this.go(this.return_to, window, cx)))
                .into_any_element(),
        ])
    }

    /// The Auto-update select ("Check on start" / "Never"), created once.
    pub(crate) fn make_auto_select(window: &mut Window, cx: &mut Context<App>) -> Entity<SelectState<Vec<SharedString>>> {
        use gpui_kit::component::IndexPath;
        use gpui_kit::component::select::SelectEvent;
        let st = cx.new(|cx| {
            SelectState::new(
                vec![SharedString::from("Check on start"), SharedString::from("Never")],
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        cx.subscribe(&st, |this: &mut App, _, ev: &SelectEvent<Vec<SharedString>>, cx| {
            if let SelectEvent::Confirm(Some(v)) = ev {
                this.settings.check_updates = v.as_ref() == "Check on start";
                this.save_settings();
                cx.notify();
            }
        })
        .detach();
        st
    }
}
