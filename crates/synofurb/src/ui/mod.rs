//! The desktop interface, built on GPUI Kit (GPUI + gpui-component). One codebase for Windows, macOS and Linux.
//!
//! Layout of this module (see docs/GUI.md for the architecture and how to add a screen):
//! * `theme`     - every colour, size, radius and type style; maps our palette onto GPUI Kit's theme
//! * `widgets`   - badges, banners, cards, form rows, tables built from the theme
//! * `state`     - pure state machine pieces (screens, risk level, confirm gate, install progress)
//! * `logview`   - bounded log buffer with level/text filtering
//! * `diagram`   - plan -> disk/partition/array/LVM diagram
//! * `worker`    - background threads (scan, inspect, install, updates) reporting through one channel
//! * `demo`      - built-in units and scripted events for `synofurb gui --demo`
//! * `app`       - the root view: state, message pump, actions
//! * `render`    - window chrome (step rail, safety bar, action bar)
//! * `screens`   - one module per screen

pub mod app;
pub mod assets;
pub mod demo;
pub mod diagram;
pub mod format;
pub mod logview;
mod render;
pub mod screens;
pub mod settings;
pub mod state;
pub mod theme;
pub mod widgets;
pub mod worker;

use crate::cli::config_dir;
use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
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
        render::bind_keys(cx);
        screens::logviewer::bind_keys(cx);
        let dark = settings.theme.is_dark(theme::os_is_dark(cx));
        theme::apply(theme::Palette::for_mode(dark), cx);

        // SYNOFURB_WINDOW=960x640 opens at a given size (used to test the minimum size).
        let (w, h) = std::env::var("SYNOFURB_WINDOW")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?))))
            .unwrap_or((theme::layout::WINDOW_W, theme::layout::WINDOW_H));
        let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(theme::layout::WINDOW_MIN_W), px(theme::layout::WINDOW_MIN_H))),
            titlebar: Some(TitlebarOptions { title: Some("synofurb".into()), ..Default::default() }),
            app_id: Some("synofurb".into()),
            ..Default::default()
        };
        let res = gpui_kit::open_window(options, cx, move |window, cx| {
            window.set_rem_size(px(scale.rem_px()));
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
