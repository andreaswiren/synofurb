//! The desktop interface, built on GPUI Kit (GPUI + gpui-component). One codebase for Windows, macOS and Linux.
//!
//! Layout of this module (see docs/GUI.md for the architecture and how to add a screen):
//! * `theme`     - sizes and type styles; loads the designer's ThemeSet (`assets/theme/`) into GPUI Kit
//! * `tokens`    - generated colour tokens (`design/v2/tokens-v2.json`)
//! * `copy`      - every user-visible string, with word-budget tests
//! * `kit`       - small helpers on stock widgets (tags, labels with info icons, segmented choices)
//! * `disclosure`- info icons (tooltip / popover) and dialogs
//! * `icons`, `brand`, `assets` - Lucide roles, the logo, bundled fonts
//! * `state`     - pure state machine pieces (screens, risk level, confirm gate, install progress)
//! * `logview`   - bounded log buffer with level/text filtering
//! * `diagram`   - plan -> disk/partition/array/LVM diagram
//! * `worker`    - background threads (scan, inspect, install, updates) reporting through one channel
//! * `demo`      - built-in units and scripted events for `synofurb gui --demo`
//! * `app`       - the root view: state, message pump, actions
//! * `render`    - window chrome (step rail, safety bar, action bar)
//! * `screens`   - one module per screen

// The desktop UI is mid-rebuild (phase 1 done: fonts, icons, theme, scrollbars, copy, disclosure, help, about; phase 2
// screens await the owner's mockup approval), so some helpers and copy items are not wired in yet.
#![allow(dead_code)]

pub mod app;
pub mod assets;
pub mod brand;
pub mod cli_cmd;
pub mod copy;
pub mod demo;
pub mod diagram;
pub mod disclosure;
pub mod format;
pub mod icons;
pub mod kit;
pub mod logview;
mod overlays;
pub mod raid;
mod render;
pub mod screens;
pub mod settings;
pub mod state;
pub mod theme;
pub mod tokens;
pub mod worker;

use crate::cli::config_dir;
use gpui_kit::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use settings::UiSettings;
use state::{DEMO_SCREEN_NAMES, DemoTarget};

/// Start the UI. `demo` is the `--screen` name when `--demo` was given.
pub fn run(demo: Option<&str>) -> anyhow::Result<()> {
    let target = match demo {
        None => None,
        Some(name) => Some(DemoTarget::parse(name).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown demo screen '{name}'. Choose one of: {} (the old names layout and account also work)",
                DEMO_SCREEN_NAMES.join(", ")
            )
        })?),
    };
    let settings = if target.is_some() { UiSettings::for_demo() } else { UiSettings::load(&UiSettings::path(&config_dir())) };
    let scale = settings.text_scale;

    gpui_kit::application().with_assets(assets::UiAssets).run(move |cx| {
        gpui_kit::init(cx);
        assets::register_fonts(cx);
        render::bind_keys(cx);
        screens::logviewer::bind_keys(cx);
        let dark = settings.theme.is_dark(theme::os_is_dark(cx));
        theme::apply(dark, scale, cx);

        // SYNOFURB_WINDOW=960x640 opens at a given size (used to test the minimum size).
        let (w, h) = std::env::var("SYNOFURB_WINDOW")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?))))
            .unwrap_or((theme::layout::WINDOW_W, theme::layout::WINDOW_H));
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(theme::layout::WINDOW_MIN_W), px(theme::layout::WINDOW_MIN_H))),
            app_id: Some("synofurb".into()),
            ..gpui_kit::component::TitleBar::window_options()
        };
        let res = gpui_kit::open_window(options, cx, move |window, cx| {
            window.set_window_title("synofurb");
            let view = cx.new(|cx| app::App::new(target, window, cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            view
        });
        if let Err(e) = res {
            eprintln!("error: cannot open the window: {e:#}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}
