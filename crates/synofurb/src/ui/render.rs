//! The window shell around every screen (UX spec 1.1): TitleBar (mark, wordmark, Help / Settings / menu), the
//! horizontal Stepper, the scrolling page, a pinned footer with the actions, and the StatusBar (risk tag, unit,
//! "Way back"). Screens return a [`Page`]; this file lays it out. Every element here is a stock gpui-component widget.

use super::app::{App, UpdateState};
use super::brand;
use super::copy::{self, Item};
use super::disclosure::info_tip;
use super::facts::{self, Risk, UnitFacts, Where};
use super::icons::{self, role};
use super::kit;
use super::state::*;
use super::theme::{self, Themed, Tone, layout};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, AnyElement, Context, FontWeight, IntoElement, KeyBinding, Render, Role, SharedString, Window, actions, div, px};

actions!(synofurb, [GoBack, PrimaryAction, OpenSettings, OpenHelp, OpenAbout, CopyCommand, CheckUpdates]);

/// `cmd-<k>` on macOS, `ctrl-<k>` elsewhere (a `&'static str`, as `KeyBinding::new` needs).
macro_rules! modkey {
    ($k:literal) => {
        if cfg!(target_os = "macos") { concat!("cmd-", $k) } else { concat!("ctrl-", $k) }
    };
}
pub(crate) use modkey;

pub const KEY_CONTEXT: &str = "SynoApp";
/// The same, but not while a text field has focus (so typing `?` in a field types a question mark).
const KEY_CONTEXT_NOT_TEXT: &str = "SynoApp && !Input";

pub fn bind_keys(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("alt-left", GoBack, Some(KEY_CONTEXT)),
        KeyBinding::new(modkey!(","), OpenSettings, Some(KEY_CONTEXT)),
        KeyBinding::new("f1", OpenHelp, Some(KEY_CONTEXT)),
        KeyBinding::new("?", OpenHelp, Some(KEY_CONTEXT_NOT_TEXT)),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-shift-c" } else { "ctrl-shift-c" }, CopyCommand, Some(KEY_CONTEXT)),
    ]);
}

/// What a screen hands to the shell.
pub struct Page {
    pub title: SharedString,
    /// The title's info icon (popover with a paragraph and a link to a Help topic).
    pub title_info: Option<&'static Item>,
    pub subtitle: Option<SharedString>,
    /// The subtitle's tooltip.
    pub subtitle_tip: Option<&'static Item>,
    pub header_right: Option<AnyElement>,
    pub body: AnyElement,
    /// Left side of the footer: Back, Copy command.
    pub footer_left: Vec<AnyElement>,
    /// Right side of the footer: secondary actions, then the primary action rightmost.
    pub footer_right: Vec<AnyElement>,
    /// The body scrolls inside the page. Screens with their own scrolling region (the install log) turn this off.
    pub scroll: bool,
}

impl Page {
    pub fn new(title: impl Into<SharedString>, body: impl IntoElement) -> Page {
        Page {
            title: title.into(),
            title_info: None,
            subtitle: None,
            subtitle_tip: None,
            header_right: None,
            body: body.into_any_element(),
            footer_left: vec![],
            footer_right: vec![],
            scroll: true,
        }
    }
    pub fn subtitle(mut self, s: impl Into<SharedString>) -> Page {
        self.subtitle = Some(s.into());
        self
    }
    pub fn subtitle_tip(mut self, i: &'static Item) -> Page {
        self.subtitle_tip = Some(i);
        self
    }
    pub fn title_info(mut self, i: &'static Item) -> Page {
        self.title_info = Some(i);
        self
    }
    pub fn header_right(mut self, e: impl IntoElement) -> Page {
        self.header_right = Some(e.into_any_element());
        self
    }
    pub fn left(mut self, b: Vec<AnyElement>) -> Page {
        self.footer_left = b;
        self
    }
    pub fn right(mut self, b: Vec<AnyElement>) -> Page {
        self.footer_right = b;
        self
    }
    pub fn no_scroll(mut self) -> Page {
        self.scroll = false;
        self
    }
}

impl App {
    fn page_for(&mut self, window: &mut Window, cx: &mut Context<App>) -> Page {
        match self.screen {
            Screen::Inventory => self.page_inventory(window, cx),
            Screen::Connect => self.page_connect(window, cx),
            Screen::Inspect => self.page_inspect(window, cx),
            Screen::Goal => self.page_goal(window, cx),
            Screen::Plan => self.page_plan(window, cx),
            Screen::System => self.page_system(window, cx),
            Screen::Confirm => self.page_confirm(window, cx),
            Screen::Install => self.page_install(window, cx),
            Screen::Failed => self.page_failed(window, cx),
            Screen::Done => self.page_done(window, cx),
            Screen::Settings => self.page_settings(window, cx),
            Screen::BootMgr => self.page_bootmgr(window, cx),
            Screen::Recover => self.page_recover(window, cx),
            Screen::Manage => self.page_manage(window, cx),
            Screen::About => self.page_about(window, cx),
        }
    }

    /// Back target for Alt+Left and the Back button.
    pub fn back_target(&self) -> Option<Screen> {
        Some(match self.screen {
            Screen::Connect => Screen::Inventory,
            Screen::Inspect => Screen::Connect,
            Screen::Goal => Screen::Inspect,
            Screen::Plan | Screen::BootMgr | Screen::Recover => Screen::Goal,
            Screen::System => Screen::Plan,
            Screen::Confirm => Screen::System,
            Screen::Settings | Screen::About | Screen::Manage => self.return_to,
            _ => return None,
        })
    }

    /// What the app knows has been written to the unit (the risk Tag and Recovery read this, never the screen name).
    pub fn unit_facts(&self) -> UnitFacts {
        let dom = self.facts.as_ref().is_some_and(|f| self.sys.boot.resolve(f) == BootChoice::Dom);
        let debian = matches!(self.result, Some(Ok(_))) || self.manage_bound();
        let agent = matches!(self.bootmgr, super::app::Job::Running | super::app::Job::Done(Ok(())));
        let fail = self.screen == Screen::Failed || matches!(self.result, Some(Err(_)));
        // The unit's own boot state, when it was read over SSH, beats this session's memory.
        let read = self.unit_state.as_ref().filter(|s| s.read == synofurb_core::recovery::ReadOutcome::Read);
        let session_flash = (agent || (self.install_started && dom) || self.agent_unknown) && !self.flash_untouched;
        match read {
            Some(s) => UnitFacts {
                flash: !s.our_entries.is_empty(),
                disks: self.install.past_no_return,
                debian: debian || s.running == synofurb_core::recovery::RunningSystem::Debian,
                fail,
                agent_unknown: false,
                agent_running: s.running == synofurb_core::recovery::RunningSystem::Agent,
                default_not_dsm: !s.default_entry.is_dsm(),
            },
            None => UnitFacts {
                flash: session_flash,
                disks: self.install.past_no_return,
                debian,
                fail,
                agent_unknown: self.agent_unknown,
                agent_running: false,
                default_not_dsm: debian || self.agent_unknown,
            },
        }
    }

    pub fn risk_where(&self) -> Where {
        match self.screen {
            Screen::Settings | Screen::About => Where::Neutral,
            Screen::Manage => Where::Manage(super::mgmt::risk_health(self.manage_report.as_ref(), self.svc_fixed), self.svc_fixed),
            Screen::Plan | Screen::System | Screen::Confirm => Where::Planning,
            _ => Where::Wizard,
        }
    }

    /// The risk Tag state; `None` on Settings and About.
    pub fn risk(&self) -> Option<Risk> {
        facts::risk(&self.unit_facts(), self.risk_where())
    }

    fn render_title_bar(&self, cx: &mut Context<App>) -> AnyElement {
        let app = cx.entity();
        let dot = matches!(self.update, UpdateState::Available(_));
        let right = h_flex()
            .gap(px(4.0))
            .items_center()
            .child(
                Button::new("tb-help")
                    .ghost()
                    .small()
                    .icon(icons::icon(role::TIP))
                    .tooltip(copy::MENU_HELP.tip)
                    .accessibility_label(copy::MENU_HELP.short)
                    .on_click(cx.listener(|this, _, window, cx| this.open_help_topic(None, window, cx))),
            )
            .child(
                Badge::new().dot().count(usize::from(dot)).child(
                    Button::new("tb-settings")
                        .ghost()
                        .small()
                        .icon(icons::icon(role::SETTINGS))
                        .tooltip(copy::MENU_SETTINGS.tip)
                        .accessibility_label(copy::MENU_SETTINGS.short)
                        .disabled(self.install_running())
                        .on_click(cx.listener(|this, _, window, cx| this.go(Screen::Settings, window, cx))),
                ),
            )
            .child(
                Button::new("tb-menu")
                    .ghost()
                    .small()
                    .icon(icons::icon(gpui_kit::assets::IconName::Menu))
                    .accessibility_label("Menu")
                    .dropdown_menu(move |menu, _, _| {
                        let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
                        menu.item(
                            PopupMenuItem::new(copy::MENU_ABOUT.short)
                                .icon(icons::icon(role::ABOUT))
                                .on_click(move |_, window, cx| a1.update(cx, |t, cx| t.go(Screen::About, window, cx))),
                        )
                        .item(
                            PopupMenuItem::new(copy::MENU_UPDATES.short)
                                .icon(icons::icon(role::REFRESH))
                                .on_click(move |_, _, cx| a2.update(cx, |t, cx| t.check_updates_now(cx))),
                        )
                        .item(
                            PopupMenuItem::new(copy::WELCOME_MENU.short)
                                .icon(icons::icon(role::SPARKLES))
                                .on_click(move |_, window, cx| a3.update(cx, |t, cx| t.open_welcome(0, window, cx))),
                        )
                    }),
            );
        TitleBar::new()
            .child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    // the two-tone logotype (mark + outlined "synofurb"), not plain text
                    .child(brand::logo(cx, 22.0)),
            )
            .child(right)
            .into_any_element()
    }

    fn render_stepper(&self, cx: &mut Context<App>) -> AnyElement {
        let cur = self.current_step();
        let running = self.install_running();
        let p_err = cx.pal().err_fg;
        let items = Step::ALL.iter().map(|s| {
            let state = step_state(cur, *s);
            let clickable = can_jump(cur, *s, self.install_started) && !running;
            let label = match s {
                Step::Find => &copy::STEP_FIND,
                Step::Connect => &copy::STEP_CONNECT,
                Step::Goal => &copy::STEP_GOAL,
                Step::Setup => &copy::STEP_SETUP,
                Step::Confirm => &copy::STEP_CONFIRM,
                Step::Install => &copy::STEP_INSTALL,
            };
            let failed = self.screen == Screen::Failed && *s == Step::Install;
            let done = state == StepState::Done || (self.screen == Screen::Done && *s == Step::Install);
            let upcoming = !done && !failed && state != StepState::Current;
            let item = StepperItem::new().disabled(!clickable && state != StepState::Current).child(
                div()
                    .text_sm()
                    .when(state == StepState::Current, |d| d.font_weight(FontWeight::BOLD))
                    .when(upcoming, |d| d.text_color(cx.theme().muted_foreground))
                    .when(failed, |d| d.text_color(p_err))
                    .child(label.short),
            );
            if failed {
                item.icon(icons::icon(role::ERR))
            } else if done {
                item.icon(icons::icon(role::CHECK))
            } else {
                item
            }
        });
        let at = if self.screen == Screen::Done { Step::ALL.len() } else { cur.number() - 1 };
        div()
            .w_full()
            .flex_none()
            .px(px(layout::PAGE_PAD))
            .py(px(8.0))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Stepper::new("steps").selected_index(at).items(items).on_click(cx.listener(|this, ix: &usize, window, cx| {
                let target = Step::ALL[*ix];
                if can_jump(this.current_step(), target, this.install_started) && !this.install_running() {
                    this.go(target.screen(), window, cx);
                }
            })))
            .into_any_element()
    }

    /// The unit strip under the stepper: tile, model, compatibility (or Debian) and Twin tags, address, serial, version,
    /// and the facts on the right. Not shown on Find, Settings or About (nothing is selected there).
    fn render_unit_header(&self, cx: &mut Context<App>) -> Option<AnyElement> {
        if matches!(self.screen, Screen::Inventory | Screen::Settings | Screen::About) {
            return None;
        }
        let p = cx.pal();
        let (mut model, mut serial, status) = self.selected_unit_model();
        let mut host = super::format::display_host(&self.input_value(&self.inp.host, cx));
        if self.manage_bound()
            && let Some(u) = &self.manage_unit
        {
            model = if u.model.is_empty() { u.title() } else { u.model.clone() };
            serial = u.serial.clone();
            host = u.primary_ip().map(|i| i.to_string()).unwrap_or_default();
        }
        let debian = self.unit_facts().debian;
        let twin = self.twin_of_selected(cx);
        let (disks, cpu, ram, dsm) = match self.facts.as_ref().filter(|_| !self.manage_bound()) {
            Some(f) => (
                format!(
                    "{} disks \u{00b7} {}",
                    f.candidate_disks().len(),
                    super::format::size_2dp(f.candidate_disks().iter().map(|d| d.size_bytes()).sum())
                ),
                super::format::short_cpu(&f.cpu),
                format!("{:.0} GiB", f.mem_gib()),
                f.dsm_version.clone(),
            ),
            None => (String::new(), String::new(), String::new(), String::new()),
        };
        Some(
            h_flex()
                .w_full()
                .flex_none()
                .items_center()
                .gap(px(12.0))
                .px(px(layout::PAGE_PAD))
                .py(px(8.0))
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .size(px(40.0))
                        .rounded(cx.theme().radius)
                        .bg(p.accent_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icons::icon_sized(role::SERVER, 20.0, p.accent_fg)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            h_flex()
                                .gap(px(8.0))
                                .items_center()
                                .child(div().text_base().font_weight(FontWeight::BOLD).child(model))
                                .child(if debian {
                                    kit::tag(&p, Tone::Ok, Some(role::OK), "Debian 13")
                                } else {
                                    kit::compat_tag(&p, status)
                                })
                                .when_some(twin, |d, _| d.child(kit::tag(&p, Tone::Warn, Some(role::WARN), copy::INV_TWIN.short))),
                        )
                        .child(
                            h_flex()
                                .gap(px(12.0))
                                .child(kit::mono_sm(cx, host))
                                .child(kit::mono_sm(cx, if serial.is_empty() { String::new() } else { format!("SN {serial}") }))
                                .child(kit::caption(cx, if dsm.is_empty() { String::new() } else { format!("DSM {dsm}") })),
                        ),
                )
                .child(h_flex().gap(px(12.0)).child(kit::caption(cx, disks)).child(kit::caption(cx, cpu)).child(kit::caption(cx, ram)))
                .into_any_element(),
        )
    }

    fn render_status_bar(&self, cx: &mut Context<App>) -> AnyElement {
        let p = cx.pal();
        let at = self.risk_where();
        let risk = self.risk();
        let risk_tag = risk.map(|r| {
            let (tone, icon) = match r {
                Risk::ReadOnly => (Tone::Neutral, role::LOCK),
                Risk::PlanOnly => (Tone::Neutral, role::EDIT),
                Risk::FlashChanged | Risk::DisksChanged | Risk::NeedsAttention => (Tone::Warn, role::WARN),
                Risk::FailedSafely => (Tone::Warn, role::SAFE),
                Risk::BootUnconfirmed => (Tone::Warn, role::TIP),
                Risk::DebianRunning => (Tone::Ok, role::OK),
                Risk::Critical => (Tone::Err, role::STOP),
                Risk::Unreachable => (Tone::Neutral, role::OFFLINE),
            };
            div().id("risk-tag").role(Role::Status).aria_label(format!("{}. {}", r.label(), r.way_back(at))).child(kit::tag(
                &p,
                tone,
                Some(icon),
                r.label(),
            ))
        });
        let way_back = risk.map(|r| {
            let text = r.way_back(at);
            Popover::new("way-back")
                .anchor(Anchor::BottomRight)
                .trigger(Button::new("way-back-btn").link().small().label(copy::WAY_BACK.short))
                .content(move |_, _, _| div().w(px(280.0)).text_sm().child(text))
        });
        let offline = self.offline_app();
        StatusBar::new()
            .left(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .children(risk_tag)
                    .when(self.demo.is_some(), |d| d.child(kit::tag(&p, Tone::Info, None, copy::TAG_DEMO.short)))
                    .when(offline, |d| d.child(kit::tag(&p, Tone::Warn, Some(role::OFFLINE), copy::TAG_OFFLINE.short))),
            )
            .right(h_flex().children(way_back))
            .into_any_element()
    }

    fn render_update_banner(&self, cx: &mut Context<App>) -> Option<AnyElement> {
        if self.install_started && self.result.is_none() {
            return None; // never while a conversion runs
        }
        if self.screen.is_side_page() {
            return None;
        }
        let UpdateState::Available(info) = &self.update else { return None };
        let p = cx.pal();
        Some(
            h_flex()
                .w_full()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .px(px(layout::PAGE_PAD))
                .pt(px(8.0))
                .child(Alert::info("update-banner", copy::UPD_BANNER.short).small().flex_1().min_w_0())
                .child(kit::tag(&p, Tone::Accent, None, info.latest.clone()))
                .child(
                    Button::new("upd-details")
                        .ghost()
                        .small()
                        .label(copy::UPD_DETAILS.short)
                        .on_click(cx.listener(|this, _, window, cx| this.open_update_dialog(window, cx))),
                )
                .child(
                    Button::new("upd-later")
                        .ghost()
                        .small()
                        .label(copy::UPD_LATER.short)
                        .on_click(cx.listener(|this, _, _, cx| this.remind_tomorrow(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_header(&self, page: &Page, cx: &mut Context<App>) -> AnyElement {
        let app = cx.entity();
        h_flex()
            .w_full()
            .items_start()
            .justify_between()
            .gap(px(16.0))
            .child(
                v_flex()
                    .gap(px(2.0))
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap(px(4.0))
                            .items_center()
                            .child(
                                div()
                                    .id("page-title")
                                    .role(Role::Heading)
                                    .aria_label(page.title.clone())
                                    .text_size(theme::TITLE.size())
                                    .line_height(theme::TITLE.line())
                                    .font_weight(FontWeight(theme::TITLE.weight))
                                    .child(page.title.clone()),
                            )
                            .when_some(page.title_info, |d, i| d.child(info_tip("title-info", i, &app))),
                    )
                    .when_some(page.subtitle.clone(), |d, s| {
                        d.child(
                            h_flex()
                                .gap(px(2.0))
                                .items_center()
                                .child(kit::muted(cx, s))
                                .when_some(page.subtitle_tip, |d, i| d.child(info_tip("sub-info", i, &app))),
                        )
                    }),
            )
            .when_some(page.header_right.is_some().then_some(()), |d, _| d)
            .into_any_element()
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut page = self.page_for(window, cx);
        let title_bar = self.render_title_bar(cx);
        let stepper = (!self.screen.is_side_page()).then(|| self.render_stepper(cx));
        let unit_header = self.render_unit_header(cx);
        let status = self.render_status_bar(cx);
        let update = self.render_update_banner(cx);
        let header = self.render_header(&page, cx);
        let header_right = page.header_right.take();

        let top = h_flex()
            .w_full()
            .items_start()
            .gap(px(16.0))
            .child(div().flex_1().min_w_0().child(header))
            .when_some(header_right, |d, r| d.child(r));
        let content = v_flex().w_full().gap(px(16.0)).p(px(layout::PAGE_PAD)).child(top).child(page.body);
        let content_area = if page.scroll {
            div()
                .id("page")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .track_scroll(&self.page_scroll)
                .vertical_scrollbar(&self.page_scroll)
                .child(content)
                .into_any_element()
        } else {
            div().flex_1().min_h_0().w_full().flex().flex_col().child(content.h_full().min_h_0()).into_any_element()
        };

        let footer = h_flex()
            .w_full()
            .flex_none()
            .px(px(layout::PAGE_PAD))
            .py(px(12.0))
            .gap(px(8.0))
            .items_center()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().tokens.background)
            .child(h_flex().gap(px(8.0)).items_center().children(page.footer_left))
            .child(div().flex_1())
            .child(h_flex().gap(px(8.0)).items_center().children(page.footer_right));

        div()
            .id("synofurb")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &GoBack, window, cx| {
                if !this.install_running()
                    && let Some(t) = this.back_target()
                {
                    this.go(t, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                if !this.install_running() && this.screen != Screen::Settings {
                    this.go(Screen::Settings, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenHelp, window, cx| this.open_help_topic(None, window, cx)))
            .on_action(cx.listener(|this, _: &OpenAbout, window, cx| this.go(Screen::About, window, cx)))
            .on_action(cx.listener(|this, _: &CheckUpdates, _, cx| this.check_updates_now(cx)))
            .on_action(cx.listener(|this, _: &CopyCommand, window, cx| this.copy_cli_command(window, cx)))
            .on_action(cx.listener(|this, _: &PrimaryAction, window, cx| this.primary_action(window, cx)))
            // Enter runs the screen's main action only while no control has focus (the page itself is focused). A focused
            // button, link or field keeps Enter for itself (a bound action would swallow the button's own Enter), and the
            // Erase button swallows it on purpose (see `page_confirm`).
            .on_key_down(cx.listener(|this, ev: &gpui_kit::KeyDownEvent, window, cx| {
                if ev.keystroke.key == "enter" && ev.keystroke.modifiers == gpui_kit::Modifiers::default() && this.focus.is_focused(window)
                {
                    this.primary_action(window, cx);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().tokens.background)
            .text_color(cx.theme().foreground)
            .text_size(theme::BODY.size())
            .child(title_bar)
            .children(stepper)
            .children(unit_header)
            .children(update)
            .child(content_area)
            .child(footer)
            .child(status)
    }
}

impl App {
    /// Enter outside a text field: the primary action of the screen. Never the destructive action.
    pub fn primary_action(&mut self, window: &mut Window, cx: &mut Context<App>) {
        match self.screen {
            Screen::Inventory => {
                if let Some(ip) = self.selected_ip.clone() {
                    self.select_host(&ip, window, cx);
                }
            }
            Screen::Connect => self.start_inspect(window, cx),
            Screen::Inspect => {
                if !self.facts.as_ref().is_some_and(|f| self.compat_for_facts(f).blocking) {
                    self.go(Screen::Goal, window, cx)
                }
            }
            Screen::Goal => self.continue_from_goal(window, cx),
            Screen::Plan => {
                if self.plan_ok() {
                    self.go(Screen::System, window, cx)
                }
            }
            Screen::System if self.system_form_errors(cx).is_empty() => self.go(Screen::Confirm, window, cx),
            _ => {}
        }
    }
}
