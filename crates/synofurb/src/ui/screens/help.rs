//! D16 Help centre: a `Sheet` from the right with a search field, Topics / Shortcuts tabs, a list of ten topics and a
//! topic view rendered by `TextView::markdown`. Bundled, so it works offline; only "More" needs the Internet.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::empty::{Empty, EmptyHeader, EmptyTitle};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, Context, Entity, IntoElement, Keystroke, Render, SharedString, Subscription, Window, div, px};

const DOCS_BASE: &str = "https://github.com/andreaswiren/synofurb/blob/main/";

pub struct HelpView {
    app: Entity<App>,
    search: Entity<InputState>,
    tab: usize,
    topic: Option<&'static str>,
    _sub: Subscription,
}

impl HelpView {
    pub fn new(app: Entity<App>, topic: Option<&'static str>, window: &mut Window, cx: &mut Context<HelpView>) -> HelpView {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(copy::HELP_SEARCH.short));
        let sub = cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        HelpView { app, search, tab: 0, topic, _sub: sub }
    }

    /// Show a topic (or the list when `None`).
    pub fn show(&mut self, topic: Option<&'static str>, cx: &mut Context<HelpView>) {
        self.topic = topic;
        self.tab = 0;
        cx.notify();
    }
}

impl Render for HelpView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_string();
        let this = cx.entity();

        if let Some((id, (item, docs))) = self.topic.and_then(|t| copy::topic(t).map(|x| (t, x))) {
            let url = format!("{DOCS_BASE}{docs}");
            return v_flex()
                .size_full()
                .gap(px(12.0))
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(Button::new("help-back").ghost().small().icon(icons::icon(role::BACK)).label(copy::BTN_BACK.short).on_click(
                            {
                                let this = this.clone();
                                move |_, _, cx| this.update(cx, |v, cx| v.show(None, cx))
                            },
                        ))
                        .child(
                            Button::new("help-more")
                                .link()
                                .small()
                                .icon(icons::icon(role::EXTERNAL))
                                .label(copy::HELP_MORE.short)
                                .tooltip(copy::HELP_MORE.tip)
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        ),
                )
                .child(kit::heading(item.short))
                .child(
                    div()
                        .id("topic-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(TextView::markdown(SharedString::from(format!("topic-{id}")), item.body)),
                )
                .into_any_element();
        }

        let topics = copy::search_topics(&query);
        // the Welcome guide is the first row of the list (a dialog, not a topic, so it has no chevron)
        let welcome = query.trim().is_empty() || copy::WELCOME_MENU.matches(&query);
        let list: gpui_kit::AnyElement = if self.tab == 1 {
            v_flex()
                .gap(px(6.0))
                .children(copy::SHORTCUTS.iter().map(|(keys, what)| {
                    let key = Keystroke::parse(keys).ok();
                    let kbd: gpui_kit::AnyElement = match key {
                        Some(k) => Kbd::new(k).into_any_element(),
                        None => kit::mono_sm(cx, *keys).into_any_element(),
                    };
                    h_flex().gap(px(12.0)).items_center().child(div().w(px(120.0)).child(kbd)).child(div().text_sm().child(*what))
                }))
                .into_any_element()
        } else if topics.is_empty() && !welcome {
            Empty::new().header(EmptyHeader::new().title(EmptyTitle::new().child(copy::HELP_NO_TOPICS.short))).into_any_element()
        } else {
            v_flex()
                .gap(px(2.0))
                .when(welcome, |d| {
                    let app = self.app.clone();
                    d.child(
                        ListItem::new("help-welcome")
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(icons::icon(role::SPARKLES))
                                    .child(div().text_sm().child(copy::WELCOME_MENU.short)),
                            )
                            .on_click(move |_, window, cx| {
                                window.close_sheet(cx);
                                app.update(cx, |a, cx| a.open_welcome(0, window, cx));
                            }),
                    )
                })
                .children(topics.iter().enumerate().map(|(i, (id, item, _))| {
                    let (this, id) = (this.clone(), *id);
                    ListItem::new(("topic", i))
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .child(div().text_sm().child(item.short))
                                .child(icons::icon(role::COLLAPSE)),
                        )
                        .on_click(move |_, _, cx| this.update(cx, |v, cx| v.show(Some(id), cx)))
                }))
                .into_any_element()
        };
        let tabs = TabBar::new("help-tabs")
            .underline()
            .selected_index(self.tab)
            .children([Tab::new().label(copy::HELP_TOPICS.short), Tab::new().label(copy::HELP_SHORTCUTS.short)])
            .on_click(cx.listener(|v, ix: &usize, _, cx| {
                v.tab = *ix;
                cx.notify();
            }));
        let docs = Button::new("help-docs")
            .link()
            .small()
            .icon(icons::icon(role::EXTERNAL))
            .label(copy::HELP_DOCS.short)
            .tooltip(copy::HELP_DOCS.tip)
            .on_click(|_, _, cx| cx.open_url("https://github.com/andreaswiren/synofurb/tree/main/docs"));
        v_flex()
            .size_full()
            .gap(px(12.0))
            .child(Input::new(&self.search).prefix(icons::icon(role::SEARCH)).cleanable(true).aria_label(copy::HELP_SEARCH.short))
            .child(tabs)
            .child(div().id("help-scroll").flex_1().min_h_0().overflow_y_scroll().child(list))
            .child(h_flex().child(docs))
            .into_any_element()
    }
}
