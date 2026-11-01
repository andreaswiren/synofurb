//! The window shell around every screen (UX spec 1.1): TitleBar (mark, wordmark, Help / Settings / menu), the
//! horizontal Stepper, the scrolling page, a pinned footer with the actions, and the StatusBar (risk tag, unit,
//! "Way back"). Screens return a [`Page`]; this file lays it out. Every element here is a stock gpui-component widget.

use super::app::{App, UpdateState};
use super::brand;
use super::copy::{self, Item};
use super::disclosure::info_tip;
use super::icons::{self, role};
use super::kit;
use super::state::*;
use super::theme::{self, Themed, Tone, layout};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::tag::Tag;
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
        KeyBinding::new("enter", PrimaryAction, Some(KEY_CONTEXT)),
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

    /// The risk tag state, from what has been written.
    pub fn risk(&self) -> Risk {
        let dom = self.facts.as_ref().is_some_and(|f| self.sys.boot.resolve(f) == BootChoice::Dom);
        risk_for(Written {
            screen: Some(if self.screen.is_side_page() { self.return_to } else { self.screen }),
            install_started: self.install_started,
            past_no_return: self.install.past_no_return,
            failed: self.screen == Screen::Failed || matches!(self.result, Some(Err(_))),
            succeeded: matches!(self.result, Some(Ok(_))),
            dom,
            agent: matches!(self.bootmgr, super::app::Job::Running | super::app::Job::Done(Ok(()))),
        })
    }

    fn risk_item(r: Risk) -> &'static Item {
        match r {
            Risk::ReadOnly => &copy::RISK_READ_ONLY,
            Risk::PlanOnly => &copy::RISK_PLAN_ONLY,
            Risk::FlashChanged => &copy::RISK_FLASH_CHANGED,
            Risk::DisksChanged => &copy::RISK_DISKS_CHANGED,
            Risk::FailedSafely => &copy::RISK_FAILED_SAFELY,
            Risk::DebianRunning => &copy::RISK_DEBIAN_RUNNING,
        }
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
                        let (a1, a2) = (app.clone(), app.clone());
                        menu.item(PopupMenuItem::new(copy::MENU_ABOUT.short).icon(icons::icon(role::ABOUT)).on_click(
                            move |_, window, cx| a1.update(cx, |t, cx| t.go(Screen::About, window, cx)),
                        ))
                        .item(PopupMenuItem::new(copy::MENU_UPDATES.short).icon(icons::icon(role::REFRESH)).on_click(
                            move |_, _, cx| a2.update(cx, |t, cx| t.check_updates_now(cx)),
                        ))
                    }),
            );
        TitleBar::new()
            .child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(brand::mark(cx, 20.0))
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(copy::APP_NAME.short)),
            )
            .child(right)
            .into_any_element()
    }

    fn render_stepper(&self, cx: &mut Context<App>) -> AnyElement {
        let cur = self.current_step();
        let running = self.install_running();
        let items = Step::ALL.iter().map(|s| {
            let state = step_state(cur, *s);
            let clickable = can_jump(cur, *s, self.install_started) && !running;
            let label = match s {
                Step::Find => &copy::STEP_FIND,
                Step::Connect => &copy::STEP_CONNECT,
                Step::Inspect => &copy::STEP_INSPECT,
                Step::Goal => &copy::STEP_GOAL,
                Step::Plan => &copy::STEP_PLAN,
                Step::System => &copy::STEP_SYSTEM,
                Step::Confirm => &copy::STEP_CONFIRM,
                Step::Install => &copy::STEP_INSTALL,
                Step::Done => &copy::STEP_DONE,
            };
            StepperItem::new().disabled(!clickable && state != StepState::Current).child(div().text_xs().child(label.short))
        });
        div()
            .w_full()
            .flex_none()
            .px(px(layout::PAGE_PAD))
            .py(px(8.0))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Stepper::new("steps")
                    .small()
                    .selected_index(cur.number() - 1)
                    .items(items)
                    .on_click(cx.listener(|this, ix: &usize, window, cx| {
                        let target = Step::ALL[*ix];
                        if can_jump(this.current_step(), target, this.install_started) && !this.install_running() {
                            this.go(target.screen(), window, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &mut Context<App>) -> AnyElement {
        let p = cx.pal();
        let risk = self.risk();
        let item = Self::risk_item(risk);
        let (tone, icon) = match risk {
            Risk::ReadOnly | Risk::PlanOnly => (Tone::Ok, role::SAFE),
            Risk::FlashChanged => (Tone::Warn, role::WARN),
            Risk::DisksChanged => (Tone::Err, role::DANGER),
            Risk::FailedSafely => (Tone::Warn, role::WARN),
            Risk::DebianRunning => (Tone::Ok, role::OK),
        };
        let risk_tag = HoverCard::new("risk-tag")
            .trigger(div().id("risk-tag").role(Role::Status).aria_label(format!("{}. {}", item.short, item.tip)).child(kit::tag(&p, tone, Some(icon), item.short)))
            .content({
                let tip = item.tip;
                move |_, _, _| div().p(px(8.0)).max_w(px(280.0)).text_sm().child(tip)
            });
        let identity = self.facts.as_ref().map(|f| {
            let model = if f.model.is_empty() { f.hostname.clone() } else { f.model.clone() };
            format!("{} \u{00b7} {}", model, super::format::display_host(&self.input_value(&self.inp.host, cx)))
        });
        let way_back = Popover::new("way-back").anchor(Anchor::BottomRight).trigger(Button::new("way-back-btn").link().small().label(copy::WAY_BACK.short)).content({
            let tip = item.tip;
            move |_, _, _| div().w(px(280.0)).text_sm().child(tip)
        });
        let offline = self.offline_app();
        StatusBar::new()
            .left(h_flex().gap(px(8.0)).items_center().child(risk_tag).when(self.demo.is_some(), |d| {
                d.child(kit::tag(&p, Tone::Info, None, copy::TAG_DEMO.short))
            }).when(offline, |d| d.child(kit::tag(&p, Tone::Warn, Some(role::OFFLINE), copy::TAG_OFFLINE.short))))
            .child(match identity {
                Some(i) => kit::mono_sm(cx, i).into_any_element(),
                None => div().into_any_element(),
            })
            .right(way_back)
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
                .child(Button::new("upd-details").ghost().small().label(copy::UPD_DETAILS.short).on_click(cx.listener(|this, _, window, cx| this.open_update_dialog(window, cx))))
                .child(Button::new("upd-later").ghost().small().label(copy::UPD_LATER.short).on_click(cx.listener(|this, _, _, cx| this.remind_tomorrow(cx))))
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
                        d.child(h_flex().gap(px(2.0)).items_center().child(kit::muted(cx, s)).when_some(page.subtitle_tip, |d, i| d.child(info_tip("sub-info", i, &app))))
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
        let status = self.render_status_bar(cx);
        let update = self.render_update_banner(cx);
        let header = self.render_header(&page, cx);
        let header_right = page.header_right.take();

        let top = h_flex().w_full().items_start().gap(px(16.0)).child(div().flex_1().min_w_0().child(header)).when_some(header_right, |d, r| d.child(r));
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
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().tokens.background)
            .text_color(cx.theme().foreground)
            .text_size(theme::BODY.size())
            .child(title_bar)
            .children(stepper)
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
            Screen::System => {
                if self.system_form_errors(cx).is_empty() {
                    self.go(Screen::Confirm, window, cx)
                }
            }
            _ => {}
        }
    }
}
