//! D1 Find NAS: filter, rescan, add by IP, the units as a `DataTable`, a second table for units already running Debian.

use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure::info_tip;
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::theme::Themed;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyHeader, EmptyMedia, EmptyTitle};
use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::table::{
    Column, DataTable, Table, TableBody, TableCell, TableDelegate, TableHead, TableHeader, TableRow, TableState,
};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, App as GpuiApp, Context, IntoElement, WeakEntity, Window, div, px};
use synofurb_core::compat::Status;
use synofurb_core::discovery::Found;

/// Column widths of the Find table for a table `avail` px wide (0 = unknown) at text factor `scale`: shares of the available width, so
/// the columns always add up to the table and never produce a horizontal scroll. The shares are weights of the content the columns
/// hold (model + Twin tag, address, DSM version, sources, status tag); the sum is clamped so it never exceeds `avail`.
pub fn column_widths(avail: f32, _scale: f32) -> [f32; 5] {
    const W: [f32; 5] = [2.0, 2.0, 1.4, 2.0, 2.4];
    let total: f32 = W.iter().sum();
    let avail = if avail > 0.0 { avail } else { 880.0 };
    W.map(|w| (avail * w / total).floor())
}

/// One row of the units table.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitRow {
    pub ip: String,
    pub model: String,
    pub dsm: String,
    pub found: String,
    pub status: Status,
    pub twin: bool,
}

#[derive(Default)]
pub struct InventoryTable {
    rows: Vec<UnitRow>,
    selected: Option<usize>,
    /// Text-size factor: the columns are in rem, so they grow with the text.
    scale: f32,
    /// Width of the table in px (the window minus the page padding), so the columns can share it.
    avail: f32,
    app: Option<WeakEntity<App>>,
}

impl InventoryTable {
    pub fn set_rows(&mut self, rows: Vec<UnitRow>) -> bool {
        let changed = self.rows != rows;
        self.rows = rows;
        changed
    }
    pub fn set_scale(&mut self, s: f32, avail: f32) -> bool {
        let changed = (self.scale - s).abs() > f32::EPSILON || (self.avail - avail).abs() > 0.5;
        self.scale = s;
        self.avail = avail;
        changed
    }
    pub fn set_selected(&mut self, ix: Option<usize>) {
        self.selected = ix;
    }
    pub fn set_app(&mut self, app: WeakEntity<App>) {
        self.app = Some(app);
    }
    pub fn ip_at(&self, ix: usize) -> Option<String> {
        self.rows.get(ix).map(|r| r.ip.clone())
    }
    pub fn index_of(&self, ip: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.ip == ip)
    }
}

impl TableDelegate for InventoryTable {
    fn columns_count(&self, _: &GpuiApp) -> usize {
        5
    }
    fn rows_count(&self, _: &GpuiApp) -> usize {
        self.rows.len()
    }
    fn column(&self, ix: usize, _: &GpuiApp) -> Column {
        // Zero cell padding: every cell pads itself, so the selected row can draw its accent border edge to edge.
        let w = column_widths(self.avail, self.scale)[ix.min(4)];
        let label = [&copy::INV_COL_MODEL, &copy::INV_COL_ADDR, &copy::INV_COL_DSM, &copy::INV_COL_FOUND, &copy::INV_COL_STATUS][ix.min(4)];
        Column::new(["model", "addr", "dsm", "found", "status"][ix.min(4)], label.short).width(px(w)).p_0()
    }
    fn render_th(&mut self, ix: usize, _: &mut Window, _cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let item = match ix {
            0 => &copy::INV_COL_MODEL,
            1 => &copy::INV_COL_ADDR,
            2 => &copy::INV_COL_DSM,
            3 => &copy::INV_COL_FOUND,
            _ => &copy::INV_COL_STATUS,
        };
        let app = self.app.as_ref().and_then(|a| a.upgrade());
        h_flex()
            .items_center()
            .gap(px(2.0))
            .px(px(8.0))
            .when(ix == 0, |d| d.pl(px(30.0)))
            .child(item.short)
            .when_some(app.filter(|_| item.has_more() || !item.tip.is_empty()), |d, app| d.child(info_tip(("th-info", ix), item, &app)))
    }
    fn render_td(&mut self, row: usize, col: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let p = cx.pal();
        let r = &self.rows[row];
        let selected = self.selected == Some(row);
        let edge = cx.theme().primary.opacity(0.5);
        // The selection is never colour alone: a Radio in the first cell and a 1 px accent border (50 %) around the row.
        let cell = h_flex().size_full().items_center().gap(px(6.0)).px(px(8.0)).when(selected, |d| {
            let d = d.border_t_1().border_b_1().border_color(edge);
            match col {
                0 => d.border_l_1(),
                4 => d.border_r_1(),
                _ => d,
            }
        });
        match col {
            0 => cell
                .child(Radio::new(("inv-radio", row)).checked(selected).tab_stop(false).accessibility_label(format!("Select {}", r.model)))
                .child(kit::strong(r.model.clone()))
                .when(r.twin, |d| d.child(kit::tag(&p, crate::ui::theme::Tone::Warn, Some(role::WARN), copy::INV_TWIN.short)))
                .into_any_element(),
            1 => cell.child(kit::mono(cx, r.ip.clone())).into_any_element(),
            2 => cell.child(div().text_sm().child(r.dsm.clone())).into_any_element(),
            3 => cell.child(div().text_xs().text_color(cx.theme().muted_foreground).truncate().child(r.found.clone())).into_any_element(),
            _ => cell.child(kit::compat_tag(&p, r.status)).into_any_element(),
        }
    }
    fn render_empty(&mut self, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        div()
    }
}

fn found_row(app: &App, d: &Found, twin: bool) -> UnitRow {
    let (status, _) = app.compat_for_found(d);
    UnitRow {
        ip: d.primary_ip().map(|i| i.to_string()).unwrap_or_default(),
        model: if d.model.is_empty() { "Synology".into() } else { d.model.clone() },
        dsm: if d.dsm_version.is_empty() { "-".into() } else { d.dsm_version.clone() },
        found: d.sources.join(" \u{00b7} "),
        status,
        twin,
    }
}

impl App {
    fn unit_rows(&self, cx: &GpuiApp) -> Vec<UnitRow> {
        let q = self.input_value(&self.inp.filter, cx).to_lowercase();
        let units: Vec<&Found> = self.inv.devices.iter().filter(|d| d.synology && !d.synofurb_debian).collect();
        units
            .iter()
            .filter_map(|d| {
                let twin = units.iter().filter(|o| !o.model.is_empty() && o.model == d.model).count() > 1;
                let r = found_row(self, d, twin);
                let hay = format!("{} {} {} {}", r.model, r.ip, r.dsm, r.found).to_lowercase();
                (q.is_empty() || hay.contains(&q)).then_some(r)
            })
            .collect()
    }

    pub(crate) fn page_inventory(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
        let p = cx.pal();
        let app = cx.entity();
        let scanning = self.scan.running;
        let rows = self.unit_rows(cx);
        let selected = self.selected_ip.clone();
        let sel_ix = selected.as_deref().and_then(|ip| rows.iter().position(|r| r.ip == ip));
        let n_rows = rows.len();
        let weak = cx.weak_entity();
        let scale = self.settings.text_scale.factor();
        let avail = f32::from(_window.viewport_size().width) - 2.0 * crate::ui::theme::layout::PAGE_PAD - 24.0;
        self.inv_table.update(cx, |t, cx| {
            t.delegate_mut().set_app(weak);
            t.delegate_mut().set_selected(sel_ix);
            let scaled = t.delegate_mut().set_scale(scale, avail);
            if t.delegate_mut().set_rows(rows) || scaled {
                t.refresh(cx);
            }
        });
        if let Some(ix) = sel_ix
            && self.inv_table.read(cx).selected_row() != Some(ix)
        {
            self.inv_table.update(cx, |t, cx| t.set_selected_row(ix, cx));
        }

        let methods: Vec<(String, bool)> = self.scan.methods.iter().map(|(n, r)| (n.to_string(), r.is_ok())).collect();
        let details = Popover::new("scan-details")
            .anchor(Anchor::TopLeft)
            .trigger(
                Button::new("scan-info")
                    .ghost()
                    .xsmall()
                    .icon(icons::icon(role::TIP))
                    .tooltip(copy::INV_RESCAN.tip)
                    .accessibility_label(copy::INV_RESCAN.tip),
            )
            .content({
                move |_, _, _cx| {
                    v_flex().w(px(300.0)).gap(px(6.0)).text_sm().child(kit::strong("Scan details")).child(copy::INV_RESCAN.body).children(
                        methods.iter().map(|(n, ok)| {
                            let (tone, icon) =
                                if *ok { (crate::ui::theme::Tone::Ok, role::OK) } else { (crate::ui::theme::Tone::Warn, role::WARN) };
                            kit::tag(&p, tone, Some(icon), n.clone())
                        }),
                    )
                }
            });
        let toolbar =
            h_flex()
                .gap(px(8.0))
                .items_center()
                .child(div().w(px(260.0)).child(
                    Input::new(&self.inp.filter).prefix(icons::icon(role::SEARCH)).cleanable(true).aria_label(copy::INV_FILTER.short),
                ))
                .child(
                    Button::new("rescan")
                        .ghost()
                        .icon(icons::icon(role::REFRESH))
                        .label(copy::INV_RESCAN.short)
                        .loading(scanning)
                        .disabled(scanning)
                        .on_click(cx.listener(|this, _, _, cx| this.start_scan(cx))),
                )
                .child(details)
                .child(
                    Button::new("add-ip")
                        .ghost()
                        .icon(icons::icon(gpui_kit::assets::IconName::Plus))
                        .label(copy::INV_ADD.short)
                        .tooltip(copy::INV_ADD.tip)
                        .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx))),
                )
                .when(scanning, |d| d.child(Spinner::new().small()));

        let converted: Vec<Found> = self.inv.devices.iter().filter(|d| d.synofurb_debian).cloned().collect();
        let units_ui =
            if n_rows == 0 && !scanning {
                Empty::new()
                    .header(
                        EmptyHeader::new()
                            .media(EmptyMedia::new().child(icons::icon_sized(role::SEARCH, 24.0, cx.theme().muted_foreground)))
                            .title(EmptyTitle::new().child(copy::INV_EMPTY.short)),
                    )
                    .content(
                        EmptyContent::new().child(
                            h_flex()
                                .gap(px(8.0))
                                .child(
                                    Button::new("scan-again")
                                        .outline()
                                        .label(copy::INV_SCAN_AGAIN.short)
                                        .on_click(cx.listener(|this, _, _, cx| this.start_scan(cx))),
                                )
                                .child(
                                    Button::new("add-ip-2")
                                        .outline()
                                        .label(copy::INV_ADD.short)
                                        .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx))),
                                )
                                .child(Button::new("empty-help").link().label("Help").on_click(
                                    cx.listener(|this, _, window, cx| this.open_help_topic(Some("troubleshooting"), window, cx)),
                                )),
                        ),
                    )
                    .into_any_element()
            } else if n_rows == 0 {
                v_flex().gap(px(6.0)).children((0..3).map(|_| Skeleton::new().h(px(28.0)).w_full())).into_any_element()
            } else {
                let h = 36.0 * (n_rows.min(6) as f32 + 1.0) + 8.0;
                div()
                    .w_full()
                    .h(px(h))
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .child(DataTable::new(&self.inv_table).stripe(false))
                    .into_any_element()
            };

        let mut body = v_flex().w_full().gap(px(12.0)).child(units_ui);
        if !converted.is_empty() {
            body = body.child(kit::heading(copy::INV_RUNNING.short)).child(
                Table::new()
                    .child(
                        TableHeader::new().child(
                            TableRow::new()
                                .child(TableHead::new().child(copy::INV_COL_MODEL.short))
                                .child(TableHead::new().child(copy::INV_COL_ADDR.short))
                                .child(TableHead::new().text_right().child("")),
                        ),
                    )
                    .child(TableBody::new().children(converted.iter().enumerate().map(|(i, d)| {
                        let ip = d.primary_ip().map(|i| i.to_string()).unwrap_or_default();
                        let d2 = d.clone();
                        TableRow::new()
                            .child(TableCell::new().child(kit::strong(if d.model.is_empty() { d.title() } else { d.model.clone() })))
                            .child(TableCell::new().child(kit::mono(cx, ip)))
                            .child(
                                TableCell::new().text_right().child(
                                    Button::new(("manage", i))
                                        .ghost()
                                        .small()
                                        .label(copy::INV_MANAGE.short)
                                        .tooltip(copy::INV_MANAGE.tip)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_manage(d2.clone(), window, cx);
                                        })),
                                ),
                            )
                    }))),
            );
        }
        let none = selected.is_none();
        let _ = app;
        Page::new(copy::INV_TITLE.short, body).subtitle(copy::INV_SUB.short).header_right(toolbar).right(vec![kit::with_reason(
            "connect-why",
            Button::new("connect").primary().label(copy::INV_CONNECT.short).icon(icons::icon(role::NEXT)).on_click(cx.listener(
                |this, _, window, cx| {
                    if let Some(ip) = this.selected_ip.clone() {
                        this.select_host(&ip, window, cx);
                    }
                },
            )),
            none.then(|| copy::INV_CONNECT.tip.to_string()),
        )])
    }

    /// Used by the other screens to name the selected unit.
    pub(crate) fn unit_name(&self) -> String {
        self.facts
            .as_ref()
            .map(|f| if f.model.is_empty() { f.hostname.clone() } else { f.model.clone() })
            .unwrap_or_else(|| "the NAS".into())
    }
}

#[cfg(test)]
mod tests {
    use super::column_widths;

    #[test]
    fn the_find_columns_always_fit_the_table() {
        for avail in [560.0, 700.0, 890.0, 1130.0] {
            for scale in [1.0, 1.15, 1.3] {
                let w = column_widths(avail, scale);
                assert!(w.iter().sum::<f32>() <= avail, "{avail} at {scale}: {w:?}");
                assert!(w.iter().all(|c| *c > 60.0), "{w:?}");
            }
        }
        assert!(column_widths(0.0, 1.0).iter().sum::<f32>() <= 880.0, "unknown width falls back to a safe 880");
    }
}
