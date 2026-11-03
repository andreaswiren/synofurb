//! D1 Find NAS: filter, rescan, add by IP, the units as a `DataTable`, a second table for units already running Debian.

use crate::ui::addhost::Outcome;
use crate::ui::app::App;
use crate::ui::copy;
use crate::ui::disclosure::{info_lines, info_tip};
use crate::ui::icons::{self, role};
use crate::ui::kit;
use crate::ui::render::Page;
use crate::ui::scanview::LineState;
use crate::ui::theme::{Themed, Tone};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle};
use gpui_kit::component::input::Input;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::table::{
    Column, DataTable, Table, TableBody, TableCell, TableDelegate, TableHead, TableHeader, TableRow, TableState,
};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App as GpuiApp, Context, IntoElement, WeakEntity, Window, div, px};
use synofurb_core::compat::Status;
use synofurb_core::discovery::Found;

/// Narrowest each data column may be, in rem (so it grows with the text size): model + radio (+ Twin), address, DSM version, status
/// tag. "Found by" is the one flexible column: it takes what is left and ends in an ellipsis when it must.
const MIN_REM: [f32; 4] = [9.0, 8.0, 5.6, 9.8];
/// "Found by" also holds the "Added by you" chip, which cannot shrink.
const FOUND_MIN_REM: f32 = 7.0;
/// Width of the actions column (Check again, Remove: two 24 px icon buttons, a 2 px gap and 4 px padding). It exists only while a row has actions.
pub const ACTIONS_PX: f32 = 60.0;

/// Number of columns of the Find table: the row actions get a column only while an added address is in the list.
pub fn column_count(has_actions: bool) -> usize {
    if has_actions { 6 } else { 5 }
}

/// Column widths of the Find table for a table `avail` px wide (0 = unknown) at text factor `scale`: model, address, DSM, found by,
/// status and, when `has_actions`, the row actions. Every data column gets at least what its content needs (rem based, so it grows with
/// the text), the actions column is a fixed 60 px, "Found by" absorbs the rest, and the sum never exceeds `avail`: no horizontal scroll,
/// no column that cuts its text mid-glyph (UI review 0.2.3 B1: the actions column used to take 135 px from the data).
pub fn column_widths(avail: f32, scale: f32, has_actions: bool) -> Vec<f32> {
    let avail = if avail > 0.0 { avail } else { 880.0 };
    let rem = 16.0 * scale.max(0.5);
    let actions = if has_actions { ACTIONS_PX } else { 0.0 };
    let [model, addr, dsm, status] = MIN_REM.map(|r| (r * rem).ceil());
    let found = (FOUND_MIN_REM * rem).ceil();
    let mins = [model, addr, dsm, found, status];
    let need: f32 = mins.iter().sum::<f32>() + actions;
    let mut w: Vec<f32> = if need >= avail {
        // narrower than the contents: shrink every data column alike (the text then ends in an ellipsis)
        let k = ((avail - actions) / mins.iter().sum::<f32>()).max(0.1);
        mins.iter().map(|m| (m * k).floor()).collect()
    } else {
        // spare room goes to the columns by weight (the flexible one gets the most); the floor keeps the sum under `avail`
        const SPARE: [f32; 5] = [2.0, 1.5, 1.0, 4.0, 1.5];
        let extra = avail - need;
        let total: f32 = SPARE.iter().sum();
        mins.iter().zip(SPARE).map(|(m, sp)| (m + extra * sp / total).floor()).collect()
    };
    if has_actions {
        w.push(actions);
    }
    w
}

/// What a row is: a unit found by a scan, or an address the user added and what the check made of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Scanned,
    Synology,
    Unknown,
    Unreachable,
}

impl UnitRow {
    /// The Address cell: the address, and `:port` when an added address has an SSH port other than 22.
    pub fn address_text(&self) -> String {
        match self.port {
            Some(p) if p != 22 => format!("{}:{p}", self.ip),
            _ => self.ip.clone(),
        }
    }
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
    pub kind: RowKind,
    /// Added by the user (it can be checked again and removed).
    pub added: bool,
    /// The SSH port typed with an added address, when it is not the default.
    pub port: Option<u16>,
    /// What was checked, for the info popover of an unknown or unreachable address.
    pub detail: Vec<String>,
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
    /// Do any of the rows have actions (an address the user added)?
    pub fn has_actions(&self) -> bool {
        self.rows.iter().any(|r| r.added)
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
        column_count(self.has_actions())
    }
    fn rows_count(&self, _: &GpuiApp) -> usize {
        self.rows.len()
    }
    fn column(&self, ix: usize, _: &GpuiApp) -> Column {
        // Zero cell padding: every cell pads itself, so the selected row can draw its accent border edge to edge.
        let w = column_widths(self.avail, self.scale, self.has_actions())[ix.min(column_count(self.has_actions()) - 1)];
        let names = [&copy::INV_COL_MODEL, &copy::INV_COL_ADDR, &copy::INV_COL_DSM, &copy::INV_COL_FOUND, &copy::INV_COL_STATUS];
        let text = names.get(ix).map(|i| i.short).unwrap_or("");
        Column::new(["model", "addr", "dsm", "found", "status", "actions"][ix.min(5)], text).width(px(w)).p_0()
    }
    fn render_th(&mut self, ix: usize, _: &mut Window, _cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let item = match ix {
            0 => &copy::INV_COL_MODEL,
            1 => &copy::INV_COL_ADDR,
            2 => &copy::INV_COL_DSM,
            3 => &copy::INV_COL_FOUND,
            4 => &copy::INV_COL_STATUS,
            _ => return div().into_any_element(),
        };
        let app = self.app.as_ref().and_then(|a| a.upgrade());
        h_flex()
            .items_center()
            .gap(px(2.0))
            .px(px(8.0))
            .when(ix == 0, |d| d.pl(px(32.0)))
            .child(item.short)
            .when_some(app.filter(|_| item.has_more() || !item.tip.is_empty()), |d, app| d.child(info_tip(("th-info", ix), item, &app)))
            .into_any_element()
    }
    fn render_td(&mut self, row: usize, col: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let p = cx.pal();
        let r = self.rows[row].clone();
        let selected = self.selected == Some(row);
        let edge = cx.theme().primary.opacity(0.5);
        // The selection is never colour alone: a Radio in the first cell and a 1 px accent border (50 %) around the row.
        let last = column_count(self.has_actions()) - 1;
        let cell = h_flex().size_full().items_center().gap(px(6.0)).px(px(8.0)).overflow_hidden().when(selected, |d| {
            let d = d.border_t_1().border_b_1().border_color(edge);
            match col {
                0 => d.border_l_1(),
                c if c == last => d.border_r_1(),
                _ => d,
            }
        });
        match col {
            0 => cell
                .child(Radio::new(("inv-radio", row)).checked(selected).tab_stop(false).accessibility_label(format!("Select {}", r.model)))
                .child(kit::strong(r.model.clone()).min_w_0().truncate())
                .when(r.twin, |d| d.child(kit::tag(&p, Tone::Warn, Some(role::WARN), copy::INV_TWIN.short)))
                .into_any_element(),
            1 => cell.child(kit::mono(cx, r.address_text()).min_w_0().truncate()).into_any_element(),
            2 => cell.child(div().text_sm().min_w_0().truncate().child(r.dsm.clone())).into_any_element(),
            3 if r.added => cell.child(kit::chip(&p, copy::INV_ADDED.short)).into_any_element(),
            3 => cell
                .child(div().min_w_0().text_xs().text_color(cx.theme().muted_foreground).truncate().child(r.found.clone()))
                .into_any_element(),
            4 => match r.kind {
                RowKind::Scanned | RowKind::Synology => cell.child(kit::compat_tag(&p, r.status)).into_any_element(),
                RowKind::Unknown => cell
                    .gap(px(2.0))
                    .child(kit::tag(&p, Tone::Warn, Some(role::WARN), copy::INV_UNKNOWN_TAG.short))
                    .child(info_lines(("row-info", row), copy::INV_CHECKED.short, r.detail.clone()))
                    .into_any_element(),
                RowKind::Unreachable => cell
                    .gap(px(2.0))
                    .child(kit::tag(&p, Tone::Err, Some(role::ERR), copy::INV_UNREACH_TAG.short))
                    .child(info_lines(("row-info", row), copy::INV_CHECKED.short, r.detail.clone()))
                    .into_any_element(),
            },
            _ if !r.added => cell.into_any_element(),
            _ => {
                let key = r.ip.clone();
                let (w1, w2) = (self.app.clone(), self.app.clone());
                let key2 = key.clone();
                // two icon buttons for every kind of row (same width on every row, so the column can be fixed); the words are the
                // tooltip and the accessible name
                let again = Button::new(("recheck", row))
                    .ghost()
                    .xsmall()
                    .icon(icons::icon(role::REFRESH))
                    .tooltip(copy::INV_CHECK_AGAIN.tip)
                    .accessibility_label(format!("{} {}", copy::INV_CHECK_AGAIN.short, key));
                cell.justify_end()
                    .gap(px(2.0))
                    .px(px(4.0))
                    .child(again.on_click(move |_, window, cx| {
                        if let Some(a) = w1.as_ref() {
                            let k = key.clone();
                            let _ = a.update(cx, |a, cx| a.recheck_added(&k, window, cx));
                        }
                    }))
                    .child(
                        Button::new(("remove", row))
                            .ghost()
                            .xsmall()
                            .icon(icons::icon(role::DELETE))
                            .tooltip(copy::INV_REMOVE.tip)
                            .accessibility_label(format!("{} {}", copy::INV_REMOVE.short, key2))
                            .on_click(move |_, _, cx| {
                                if let Some(a) = w2.as_ref() {
                                    let k = key2.clone();
                                    let _ = a.update(cx, |a, cx| a.remove_added(&k, cx));
                                }
                            }),
                    )
                    .into_any_element()
            }
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
        kind: RowKind::Scanned,
        added: false,
        port: None,
        detail: vec![],
    }
}

/// The rows of the addresses the user added: a Synology unit, an unknown device that answered, or an address nothing answered at.
fn added_row(app: &App, h: &crate::ui::addhost::AddedHost) -> UnitRow {
    let key = h.target.host.clone();
    match (h.outcome(), h.report.found.as_ref()) {
        (Outcome::Synology, Some(f)) => {
            let mut r = found_row(app, f, false);
            r.ip = key;
            r.kind = RowKind::Synology;
            r.added = true;
            r.port = h.target.port;
            r.found = copy::INV_ADDED.short.into();
            r
        }
        (o, _) => UnitRow {
            ip: key,
            model: if o == Outcome::Unknown { copy::INV_UNKNOWN.short } else { copy::INV_UNREACH.short }.into(),
            dsm: "-".into(),
            found: copy::INV_ADDED.short.into(),
            status: Status::Untested,
            twin: false,
            kind: if o == Outcome::Unknown { RowKind::Unknown } else { RowKind::Unreachable },
            added: true,
            port: h.target.port,
            detail: crate::ui::addhost::detail_lines(&h.report),
        },
    }
}

impl App {
    fn unit_rows(&self, cx: &GpuiApp) -> Vec<UnitRow> {
        let q = self.input_value(&self.inp.filter, cx).to_lowercase();
        // a scanned unit the user also added by hand is shown once, as the added row
        let added_synology: Vec<&str> =
            self.added.hosts.iter().filter(|h| h.outcome() == Outcome::Synology).map(|h| h.target.host.as_str()).collect();
        let units: Vec<&Found> = self
            .inv
            .devices
            .iter()
            .filter(|d| d.synology && !d.synofurb_debian)
            .filter(|d| !d.ips.iter().any(|i| added_synology.contains(&i.to_string().as_str())))
            .collect();
        let scanned = units.iter().map(|d| {
            let twin = units.iter().filter(|o| !o.model.is_empty() && o.model == d.model).count() > 1;
            found_row(self, d, twin)
        });
        let added = self.added.hosts.iter().map(|h| added_row(self, h));
        scanned
            .chain(added)
            .filter(|r| {
                let hay = format!("{} {} {} {}", r.model, r.ip, r.dsm, r.found).to_lowercase();
                q.is_empty() || hay.contains(&q)
            })
            .collect()
    }

    pub(crate) fn page_inventory(&mut self, _window: &mut Window, cx: &mut Context<App>) -> Page {
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

        let (sel_n, sel_total) = self.settings.scan_networks.counts(&self.nets);
        let chip_label = copy::NET_CHIP.fmt(&[&sel_n.to_string(), &sel_total.to_string()]);
        let toolbar =
            h_flex()
                .gap(px(8.0))
                .items_center()
                .flex_wrap()
                .child(div().w(px(240.0)).child(
                    Input::new(&self.inp.filter).prefix(icons::icon(role::SEARCH)).cleanable(true).aria_label(copy::INV_FILTER.short),
                ))
                .child(
                    Button::new("networks")
                        .ghost()
                        .icon(icons::icon(role::NETWORK))
                        .label(chip_label.clone())
                        .tooltip(copy::NET_CHIP.tip)
                        .accessibility_label(format!("{chip_label}. {}", copy::NET_CHIP.tip))
                        .on_click(cx.listener(|this, _, window, cx| this.open_networks_dialog(window, cx))),
                )
                .child(
                    Button::new("rescan")
                        .ghost()
                        .icon(icons::icon(role::REFRESH))
                        .label(copy::INV_RESCAN.short)
                        .disabled(scanning)
                        .on_click(cx.listener(|this, _, _, cx| this.start_scan(cx))),
                )
                .child(
                    Button::new("add-ip")
                        .ghost()
                        .icon(icons::icon(gpui_kit::assets::IconName::Plus))
                        .label(copy::INV_ADD.short)
                        .tooltip(copy::INV_ADD.tip)
                        .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(None, None, window, cx))),
                );

        let converted: Vec<Found> = self.inv.devices.iter().filter(|d| d.synofurb_debian).cloned().collect();
        let table = |cx: &mut Context<App>| -> gpui_kit::AnyElement {
            let h = 36.0 * (n_rows.min(10) as f32 + 1.0) + 8.0;
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
        let units_ui = if n_rows == 0 && !scanning {
            self.empty_state(cx)
        } else if n_rows == 0 {
            div().into_any_element()
        } else {
            table(cx)
        };
        // an empty result is explained by the empty state below (what was scanned, and the ways forward)
        let scan_ui = if n_rows == 0 && !scanning && self.scan.error.is_none() { div().into_any_element() } else { self.scan_section(cx) };

        let mut body = v_flex().w_full().gap(px(12.0)).child(scan_ui).child(units_ui);
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
        let unreachable = selected.as_deref().and_then(|k| self.added.get(k)).is_some_and(|h| h.outcome() == Outcome::Unreachable);
        let none = selected.is_none();
        let _ = app;
        Page::new(copy::INV_TITLE.short, body).subtitle(copy::INV_SUB.short).header_right(toolbar).right(vec![kit::with_reason(
            "connect-why",
            Button::new("connect").primary().disabled(unreachable).label(copy::INV_CONNECT.short).icon(icons::icon(role::NEXT)).on_click(
                cx.listener(|this, _, window, cx| {
                    if let Some(ip) = this.selected_ip.clone() {
                        this.select_host(&ip, window, cx);
                    }
                }),
            ),
            if none { Some(copy::INV_CONNECT.tip.to_string()) } else { unreachable.then(|| copy::INV_UNREACH_WHY.short.to_string()) },
        )])
    }

    /// The empty state of Find after a scan: what was scanned, and the three ways forward.
    fn empty_state(&self, cx: &mut Context<App>) -> gpui_kit::AnyElement {
        let what = self.scan.empty_line();
        Empty::new()
            .header(
                EmptyHeader::new()
                    .media(EmptyMedia::new().child(icons::icon_sized(role::SEARCH, 24.0, cx.theme().muted_foreground)))
                    .title(EmptyTitle::new().child(copy::INV_EMPTY.short))
                    .when(!what.is_empty(), |h| {
                        h.description(EmptyDescription::new().child(
                            v_flex().gap(px(2.0)).items_center().child(what.clone()).child(copy::INV_EMPTY_WHY.short),
                        ))
                    }),
            )
            .content(
                EmptyContent::new().child(
                    h_flex()
                        .gap(px(8.0))
                        .flex_wrap()
                        .justify_center()
                        .child(
                            Button::new("add-ip-2")
                                .outline()
                                .label(copy::INV_ADD.short)
                                .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(None, None, window, cx))),
                        )
                        .child(
                            Button::new("choose-nets")
                                .outline()
                                .label(copy::SCAN_CHOOSE.short)
                                .on_click(cx.listener(|this, _, window, cx| this.open_networks_dialog(window, cx))),
                        )
                        .child(
                            Button::new("scan-again")
                                .outline()
                                .label(copy::INV_SCAN_AGAIN.short)
                                .on_click(cx.listener(|this, _, _, cx| this.start_scan(cx))),
                        )
                        .child(
                            Button::new("empty-help")
                                .link()
                                .label(copy::HELP_LINK.short)
                                .on_click(cx.listener(|this, _, window, cx| this.open_help_topic(Some("troubleshooting"), window, cx))),
                        ),
                ),
            )
            .into_any_element()
    }

    /// The progress panel while a scan runs (what is being scanned, one line per method and network, Stop), the summary after
    /// it, or the error when it could not start. Nothing at all before the first scan.
    fn scan_section(&self, cx: &mut Context<App>) -> gpui_kit::AnyElement {
        let p = cx.pal();
        let muted = cx.theme().muted_foreground;
        let sv = &self.scan;
        if let Some(err) = &sv.error {
            return v_flex()
                .gap(px(8.0))
                .child(Alert::error("scan-error", err.clone()).title(copy::SCAN_NOT_STARTED.short).icon(icons::icon(role::ERR)))
                .child(
                    h_flex().gap(px(8.0)).child(
                        Button::new("scan-choose")
                            .outline()
                            .label(copy::SCAN_CHOOSE.short)
                            .on_click(cx.listener(|this, _, window, cx| this.open_networks_dialog(window, cx))),
                    ),
                )
                .into_any_element();
        }
        if sv.running {
            let reduce = self.settings.reduce_motion;
            let headline = if sv.stopping {
                copy::SCAN_STOPPING.short.to_string()
            } else if sv.lines.is_empty() {
                "Starting the scan".to_string()
            } else {
                sv.headline()
            };
            let pct = sv.fraction() * 100.0;
            let mut lines: Vec<gpui_kit::AnyElement> = vec![];
            let mut last_net: Option<&str> = None;
            for (i, l) in sv.lines.iter().enumerate() {
                // the network is named once, above its lines, and only when there is more than one: one network is in the headline
                if sv.show_network_captions() && last_net != Some(l.network.as_str()) {
                    last_net = Some(l.network.as_str());
                    lines.push(kit::caption(cx, sv.network_text(&l.network)).into_any_element());
                }
                let (lead, state_text): (gpui_kit::AnyElement, String) = match &l.state {
                    LineState::Waiting => {
                        (icons::icon_sized(role::PENDING, 14.0, muted).into_any_element(), copy::SCAN_WAITING.short.into())
                    }
                    LineState::Running => {
                        let lead = if reduce {
                            icons::icon_sized(role::RUNNING, 14.0, muted).into_any_element()
                        } else {
                            Spinner::new().xsmall().into_any_element()
                        };
                        let t = if l.is_sweep() && l.total > 0 {
                            format!("{} \u{00b7} {} of {}", copy::SCAN_RUNNING.short, l.done, l.total)
                        } else if l.total > 0 {
                            format!("{} \u{00b7} {}%", copy::SCAN_RUNNING.short, (l.fraction() * 100.0).round() as u32)
                        } else {
                            copy::SCAN_RUNNING.short.into()
                        };
                        (lead, t)
                    }
                    LineState::Done => (
                        icons::icon_sized(role::OK, 14.0, p.tone(Tone::Ok).fg).into_any_element(),
                        if l.found == 0 {
                            copy::SCAN_DONE.short.to_string()
                        } else {
                            format!("{} \u{00b7} {} found", copy::SCAN_DONE.short, l.found)
                        },
                    ),
                    LineState::Failed(_) => {
                        (icons::icon_sized(role::WARN, 14.0, p.tone(Tone::Warn).fg).into_any_element(), copy::SCAN_FAILED.short.into())
                    }
                    LineState::Skipped(_) => {
                        (icons::icon_sized(role::INFO, 14.0, muted).into_any_element(), copy::SCAN_SKIPPED.short.into())
                    }
                    LineState::Stopped => (icons::icon_sized(role::INFO, 14.0, muted).into_any_element(), copy::SCAN_STOPPED.short.into()),
                };
                let why = match &l.state {
                    LineState::Failed(w) | LineState::Skipped(w) => Some(w.clone()),
                    _ => None,
                };
                lines.push(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .text_sm()
                        .child(div().w(px(16.0)).flex_none().child(lead))
                        .child(div().w(px(220.0)).flex_none().truncate().child(crate::ui::scanview::method_name(l.method)))
                        .child(div().flex_1().min_w_0().truncate().text_xs().child(state_text))
                        .when_some(why, |d, w| d.child(info_lines(("scan-why", i), copy::SCAN_WHY.short, vec![w])))
                        .into_any_element(),
                );
            }
            return v_flex()
                .id("scan-panel")
                .gap(px(8.0))
                .p(px(12.0))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .gap(px(12.0))
                        .items_center()
                        .child(div().flex_1().min_w_0().text_sm().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(headline))
                        .child(
                            Button::new("scan-stop")
                                .outline()
                                .small()
                                .icon(icons::icon(role::STOP))
                                .label(copy::SCAN_STOP.short)
                                .disabled(sv.stopping)
                                .on_click(cx.listener(|this, _, _, cx| this.stop_scan(cx))),
                        ),
                )
                .child(Progress::new("scan-progress").value(pct).accessibility_label(copy::SCAN_PROGRESS.short))
                .child(v_flex().id("scan-lines").gap(px(4.0)).max_h(px(196.0)).overflow_y_scroll().children(lines))
                .into_any_element();
        }
        // after the scan: one line that says what happened, and what each method did behind an info button
        let Some(summary) = sv.summary() else { return div().into_any_element() };
        let rows: Vec<String> = sv
            .lines
            .iter()
            .map(|l| {
                let st = match &l.state {
                    LineState::Done => format!("done, {} found", l.found),
                    LineState::Failed(w) => format!("failed: {w}"),
                    LineState::Skipped(w) => format!("skipped: {w}"),
                    _ => "stopped".to_string(),
                };
                format!("{} on {}: {st}", crate::ui::scanview::method_name(l.method), sv.network_text(&l.network))
            })
            .collect();
        let failed = sv.failed_lines().len();
        h_flex()
            .gap(px(6.0))
            .items_center()
            .flex_wrap()
            .child(kit::caption(cx, summary))
            .when(failed > 0, |d| {
                d.child(kit::tag(
                    &p,
                    Tone::Warn,
                    Some(role::WARN),
                    format!("{failed} {}", if failed == 1 { "method failed" } else { "methods failed" }),
                ))
            })
            .child(info_lines("scan-ran", copy::SCAN_WHAT_RAN.short, rows))
            .into_any_element()
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
    use super::{ACTIONS_PX, column_count, column_widths};

    #[test]
    fn the_find_columns_always_fit_the_table() {
        for avail in [560.0, 700.0, 890.0, 1130.0] {
            for scale in [1.0, 1.15, 1.3] {
                for actions in [false, true] {
                    let w = column_widths(avail, scale, actions);
                    assert_eq!(w.len(), column_count(actions));
                    assert!(w.iter().sum::<f32>() <= avail, "{avail} at {scale} (actions {actions}): {w:?}");
                    assert!(w.iter().all(|c| *c > 40.0), "{w:?}");
                }
            }
        }
        assert!(column_widths(0.0, 1.0, true).iter().sum::<f32>() <= 880.0, "unknown width falls back to a safe 880");
    }

    /// UI review 0.2.3 B1: at Largest text in the smallest window the data columns must keep what their content needs. The widths
    /// below are the ones measured on the 960 x 640 captures at 130 % (the longest demo cell of each column, padding included):
    /// "Unknown device" with its radio, a 15-character address, "7.2.2-72806", "Community-reported" and the "Added by you" chip.
    #[test]
    fn at_largest_text_in_the_smallest_window_no_data_column_is_narrower_than_its_content() {
        let avail = 960.0 - 2.0 * 24.0 - 24.0;
        let w = column_widths(avail, 1.3, true);
        let need = [186.0, 160.0, 116.0, 146.0, 200.0];
        for (i, n) in need.iter().enumerate() {
            assert!(w[i] >= *n, "column {i} is {} px, its content needs {n}: {w:?}", w[i]);
        }
        assert_eq!(w[5], ACTIONS_PX, "the actions are a fixed width, not a share of the data");
        // without added rows nothing is reserved for actions, and the data gets all of it
        let plain = column_widths(avail, 1.3, false);
        assert_eq!(plain.len(), 5);
        assert!(plain.iter().zip(&w).all(|(p, d)| p >= d));
        // the default text size has room to spare
        let d = column_widths(avail, 1.0, true);
        assert!(d.iter().sum::<f32>() > 800.0);
    }
}
