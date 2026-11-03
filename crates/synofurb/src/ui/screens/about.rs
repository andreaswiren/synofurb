//! D17 About: identity, version, diagnostics to copy, links, credits (collapsed) with the bundled licences.

use crate::ui::app::{App, version};
use crate::ui::assets;
use crate::ui::brand;
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::state::Screen;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div, px};

const REPO: &str = "https://github.com/andreaswiren/synofurb";

/// `(name, role, licence label, licence text, link)` of every credit.
const CREDITS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "Debian",
        "the installed system, trixie",
        "DFSG licences",
        "Debian is made of many free-software packages, each with its own licence; see /usr/share/doc/<package>/copyright on the installed system.",
        "https://www.debian.org",
    ),
    (
        "Lucide",
        "icons",
        "ISC",
        "ISC License. Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT). All other copyright (c) for Lucide are held by Lucide Contributors 2022. Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.",
        "https://lucide.dev",
    ),
    ("Inter", "interface font", "OFL 1.1", assets::OFL_INTER, "https://rsms.me/inter/"),
    ("JetBrains Mono", "code font", "OFL 1.1", assets::OFL_JETBRAINS_MONO, "https://www.jetbrains.com/lp/mono/"),
    (
        "GPUI Kit, gpui-component",
        "UI framework",
        "Apache-2.0",
        "Licensed under the Apache License, Version 2.0. See http://www.apache.org/licenses/LICENSE-2.0 for the full text.",
        "https://gpui-kit.com",
    ),
    (
        "synofurb",
        "this program",
        "MIT",
        "MIT License. Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files, to deal in the Software without restriction, subject to the notice being included in all copies.",
        REPO,
    ),
];

impl App {
    /// Version, OS, window and theme settings, compatibility list date, log folder. Never secrets.
    pub fn diagnostics_text(&self, cx: &gpui_kit::App) -> String {
        use gpui_kit::component::ActiveTheme;
        format!(
            "synofurb {} ({})\nos: {} {}\nrenderer: GPUI (wgpu/native)\ntheme: {:?} ({}), text size {}\ncompatibility list: {} models, dated {}\nlog folder: {}\ndemo: {}\n",
            version(),
            crate::ui::updates::build_label(env!("SYNOFURB_GIT_DESCRIBE_BUILD"), version()),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.settings.theme,
            if cx.theme().is_dark() { "dark" } else { "light" },
            self.settings.text_scale.label(),
            self.compat.len(),
            self.compat.updated(),
            self.cfg_dir.join("logs").display(),
            self.demo.is_some(),
        )
    }

    pub fn copy_diagnostics(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let t = self.diagnostics_text(cx);
        self.copy_text(t, "Diagnostics", window, cx);
    }

    pub(crate) fn page_about(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let header = v_flex().gap(px(12.0)).child(brand::logo(cx, 56.0)).child(kit::muted(cx, copy::ABOUT_TAGLINE.short)).child(
            h_flex()
                .gap(px(8.0))
                .items_center()
                .child(kit::tag(
                    &p,
                    Tone::Neutral,
                    None,
                    format!("{} \u{00b7} {}", version(), crate::ui::updates::build_label(env!("SYNOFURB_GIT_DESCRIBE_BUILD"), version())),
                ))
                .when(self.demo.is_some(), |d| d.child(kit::tag(&p, Tone::Info, None, copy::TAG_DEMO.short))),
        );
        let links = h_flex()
            .gap(px(4.0))
            .child(
                Button::new("l-docs")
                    .link()
                    .icon(icons::icon(role::BOOK))
                    .label(copy::ABOUT_DOCS.short)
                    .on_click(|_, _, cx| cx.open_url(&format!("{REPO}/tree/main/docs"))),
            )
            .child(
                Button::new("l-src")
                    .link()
                    .icon(icons::icon(role::BRANCH))
                    .label(copy::ABOUT_SOURCE.short)
                    .on_click(|_, _, cx| cx.open_url(REPO)),
            )
            .child(
                Button::new("l-issue")
                    .link()
                    .icon(icons::icon(role::BUG))
                    .label(copy::ABOUT_REPORT.short)
                    .on_click(|_, _, cx| cx.open_url(&format!("{REPO}/issues/new"))),
            );
        let credits = v_flex().gap(px(2.0)).children(CREDITS.iter().enumerate().map(|(i, (name, what, lic, text, url))| {
            let (text, name_s, lic_s) = (*text, *name, *lic);
            let _ = app.clone();
            h_flex()
                .gap(px(8.0))
                .items_center()
                .py(px(4.0))
                .child(div().w(px(150.0)).text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(*name))
                .child(div().flex_1().text_sm().text_color(cx.theme().muted_foreground).child(*what))
                .child(Button::new(("lic", i)).link().small().label(*lic).tooltip(copy::ABOUT_LICENCE.short).on_click(
                    move |_, window, cx| {
                        window.open_dialog(cx, move |d, _, cx| {
                            d.bg(gpui_kit::component::ActiveTheme::theme(&*cx).popover)
                                .title(format!("{name_s} \u{00b7} {lic_s}"))
                                .w(px(560.0))
                                .child(
                                    div()
                                        .id("lic-scroll")
                                        .max_h(px(320.0))
                                        .overflow_y_scroll()
                                        .text_xs()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(text),
                                )
                                .footer(
                                    DialogFooter::new().child(
                                        Button::new("lic-close")
                                            .primary()
                                            .label(copy::BTN_CLOSE.short)
                                            .on_click(|_, window, cx| window.close_dialog(cx)),
                                    ),
                                )
                        });
                    },
                ))
                .child(Button::new(("lnk", i)).ghost().xsmall().icon(icons::icon(role::EXTERNAL)).accessibility_label(*name).on_click({
                    let url = url.to_string();
                    move |_, _, cx| cx.open_url(&url)
                }))
        }));
        let synodeb = Popover::new("synodeb")
            .trigger(Button::new("synodeb-btn").link().small().label(copy::ABOUT_SYNODEB.short))
            .content(|_, _, _| div().w(px(300.0)).text_sm().child(copy::ABOUT_SYNODEB.body));
        let acc = Accordion::new("credits")
            .bordered(true)
            .item(|i| i.title(copy::ABOUT_CREDITS.short).child(v_flex().gap(px(6.0)).child(credits).child(h_flex().child(synodeb))));
        let update_line = crate::ui::screens::update_status::status_row(&app, "a-upd", cx);
        let pending = crate::ui::screens::update_status::update_pending(&self.update);
        let body = v_flex().w_full().max_w(px(640.0)).gap(px(16.0)).child(header).child(update_line).child(links).child(acc);
        Page::new(copy::ABOUT_TITLE.short, body)
            .left(vec![
                Button::new("back")
                    .ghost()
                    .icon(icons::icon(role::BACK))
                    .label(copy::BTN_BACK.short)
                    .on_click(cx.listener(|this, _, window, cx| this.go(this.return_to, window, cx)))
                    .into_any_element(),
            ])
            .right(vec![
                Button::new("copy-diag")
                    // one primary per screen: a waiting update ("Details") takes it
                    .when(!pending, |b| b.primary())
                    .when(pending, |b| b.outline())
                    .icon(icons::icon(role::COPY))
                    .label(copy::ABOUT_COPY_DIAG.short)
                    .tooltip(copy::ABOUT_COPY_DIAG.tip)
                    .on_click(cx.listener(|this, _, window, cx| this.copy_diagnostics(window, cx)))
                    .into_any_element(),
            ])
    }
}

#[allow(dead_code)]
fn _s(_: Screen) {}
