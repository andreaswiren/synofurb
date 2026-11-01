//! The storage diagram: disks -> partitions -> RAID arrays -> LVM / filesystems, drawn from a [`Plan`].
//!
//! `DiagramModel::from_plan` is pure (and tested); `render` turns it into boxes with the design system's colours.
//! Every box carries its own text, and warnings/errors are repeated as banners below, so the picture is never the
//! only carrier of a message.

use super::copy;
use super::format::size_2dp;
use super::kit;
use super::theme::{Palette, radius, space};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, FontWeight, SharedString, div, px};
use synofurb_core::model::SECTOR;
use synofurb_core::plan::{BlockRef, PartRole, Plan};

#[derive(Clone, Debug, PartialEq)]
pub struct PartBox {
    pub number: u32,
    pub role: PartRole,
    pub size: String,
    /// md array this partition is a member of, if any.
    pub array: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiskCol {
    pub name: String,
    pub model: String,
    pub size: String,
    pub new_table: bool,
    pub parts: Vec<PartBox>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayBox {
    pub name: String,
    pub level: String,
    pub members: Vec<String>,
    /// What the array holds, for the label ("system", "/boot", "data").
    pub holds: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LvBox {
    pub name: String,
    pub size: String,
    pub fs: String,
    pub mountpoint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VgBox {
    pub name: String,
    pub pv: String,
    pub lvs: Vec<LvBox>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiagramModel {
    pub disks: Vec<DiskCol>,
    pub untouched: Vec<String>,
    pub arrays: Vec<ArrayBox>,
    pub vgs: Vec<VgBox>,
    pub boot_note: Option<String>,
}

fn pv_label(b: &BlockRef) -> String {
    match b {
        BlockRef::Part(d, n) => format!("{d} partition {n}"),
        BlockRef::Md(m) => m.clone(),
    }
}

impl DiagramModel {
    pub fn from_plan(plan: &Plan) -> DiagramModel {
        let member_of = |disk: &str, n: u32| -> Option<String> {
            plan.arrays.iter().find(|a| a.members.iter().any(|(d, p)| d == disk && *p == n)).map(|a| a.name.clone())
        };
        let disks = plan
            .disks
            .iter()
            .map(|dp| DiskCol {
                name: dp.disk.clone(),
                model: dp.ident.model.clone(),
                size: size_2dp(dp.ident.sectors * SECTOR),
                new_table: dp.new_table,
                parts: dp
                    .create
                    .iter()
                    .map(|c| PartBox {
                        number: c.number,
                        role: c.role,
                        size: size_2dp(c.sectors * SECTOR),
                        array: member_of(&dp.disk, c.number),
                    })
                    .collect(),
            })
            .collect();
        let arrays = plan
            .arrays
            .iter()
            .map(|a| {
                let is_boot = plan.boot == Some(BlockRef::Md(a.name.clone()));
                let is_esp = plan.esp == Some(BlockRef::Md(a.name.clone()));
                let holds = if is_boot {
                    "/boot".to_string()
                } else if is_esp {
                    "EFI system partition".to_string()
                } else if plan.vgs.iter().any(|v| v.pvs.contains(&BlockRef::Md(a.name.clone())) && v.name != "vg_data") {
                    "system (LVM)".to_string()
                } else {
                    "data (LVM)".to_string()
                };
                ArrayBox {
                    name: a.name.clone(),
                    level: if a.level == "1" { "RAID 1 mirror".into() } else { format!("RAID {}", a.level) },
                    members: a.members.iter().map(|(d, n)| format!("{d}{n}")).collect(),
                    holds,
                }
            })
            .collect();
        let vgs = plan
            .vgs
            .iter()
            .map(|v| VgBox {
                name: v.name.clone(),
                pv: v.pvs.iter().map(pv_label).collect::<Vec<_>>().join(", "),
                lvs: v
                    .lvs
                    .iter()
                    .map(|l| LvBox {
                        name: l.name.clone(),
                        size: match (l.mib, l.vg_percent) {
                            (Some(m), _) => super::format::gib_from_mib(m),
                            (None, Some(p)) => format!("{p}% of the group"),
                            _ => String::new(),
                        },
                        fs: l.fs.clone(),
                        mountpoint: l.mountpoint.clone(),
                    })
                    .collect(),
            })
            .collect();
        let boot_note = plan.raw_boot.as_ref().map(|r| {
            format!(
                "U-Boot raw boot area: kernel at {} MiB, initrd at {} MiB, first {} MiB of each system disk reserved",
                r.kernel_mib, r.initrd_mib, r.reserved_mib
            )
        });
        DiagramModel { disks, untouched: plan.untouched.clone(), arrays, vgs, boot_note }
    }
}

fn layer_label(cx: &App, s: &'static str) -> impl IntoElement {
    div().w(px(88.0)).flex_none().pt(px(space::S6)).child(kit::caption(cx, s))
}

fn part_style(p: &Palette, role: PartRole) -> (gpui_kit::Hsla, gpui_kit::Hsla) {
    match role {
        PartRole::Data | PartRole::SystemPv => (p.accent_bg, p.accent_border),
        _ => (p.surface_3, p.border_strong),
    }
}

/// The one hand-made element of the UI (UX spec section 2): no widget draws disk -> partition -> RAID -> LVM stacks.
/// Every box carries its own text and warnings/errors are repeated as Alerts, so the picture is never the only carrier.
pub fn render(p: &Palette, m: &DiagramModel, cx: &App) -> impl IntoElement {
    let border = cx.theme().border;
    let disks_row = h_flex().flex_1().min_w_0().gap(px(space::S8)).children(m.disks.iter().map(|d| {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(space::S2))
            .px(px(space::S8))
            .py(px(space::S6))
            .rounded(px(radius::MD))
            .border_1()
            .border_color(p.border_strong)
            .bg(p.surface)
            .child(kit::mono_sm(cx, d.name.clone()).font_weight(FontWeight::SEMIBOLD))
            .child(kit::caption(cx, format!("{} \u{b7} {}", d.model, d.size)).truncate())
    }));
    let parts_row = h_flex().flex_1().min_w_0().gap(px(space::S8)).children(m.disks.iter().map(|d| {
        h_flex().flex_1().min_w_0().gap(px(space::S4)).flex_wrap().children(d.parts.iter().map(|pt| {
            let (bg, edge) = part_style(p, pt.role);
            v_flex()
                .px(px(space::S6))
                .py(px(space::S2))
                .rounded(px(radius::SM))
                .border_1()
                .border_color(edge)
                .bg(bg)
                .child(div().text_xs().child(format!("p{} {}", pt.number, pt.role.label())))
                .child(kit::caption(cx, pt.size.clone()))
        }))
    }));
    let arrays_row = h_flex().flex_1().min_w_0().gap(px(space::S8)).flex_wrap().children(m.arrays.iter().map(|a| {
        let mirror = a.level.contains("mirror");
        let (bg, edge) = if mirror { (p.ok_bg, p.ok_border) } else { (p.accent_bg, p.accent_border) };
        v_flex()
            .px(px(space::S8))
            .py(px(space::S6))
            .rounded(px(radius::MD))
            .border_1()
            .border_color(edge)
            .bg(bg)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("{} \u{b7} {}", a.name, a.level)))
            .child(kit::caption(cx, format!("{} \u{b7} {}", a.holds, a.members.len())))
    }));
    let lvm_row = v_flex().flex_1().min_w_0().gap(px(space::S6)).children(m.vgs.iter().map(|v| {
        v_flex()
            .gap(px(space::S4))
            .child(
                h_flex().child(
                    kit::mono_sm(cx, format!("PV {} in VG {}", v.pv, v.name))
                        .px(px(space::S6))
                        .py(px(space::S2))
                        .rounded(px(radius::SM))
                        .bg(p.surface_2)
                        .border_1()
                        .border_color(border),
                ),
            )
            .child(h_flex().gap(px(space::S8)).flex_wrap().children(v.lvs.iter().map(|l| {
                v_flex()
                    .px(px(space::S8))
                    .py(px(space::S6))
                    .rounded(px(radius::MD))
                    .border_1()
                    .border_color(p.accent_border)
                    .bg(p.accent_bg)
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("{} \u{b7} {}", l.name, l.fs)))
                    .child(kit::caption(cx, format!("{}  {}", l.mountpoint, l.size)))
            })))
    }));
    v_flex()
        .w_full()
        .gap(px(space::S12))
        .child(h_flex().gap(px(space::S8)).child(layer_label(cx, copy::DIAG_DISKS.short)).child(disks_row))
        .child(h_flex().gap(px(space::S8)).child(layer_label(cx, copy::DIAG_PARTITIONS.short)).child(parts_row))
        .child(h_flex().gap(px(space::S8)).child(layer_label(cx, copy::DIAG_RAID.short)).child(arrays_row))
        .child(h_flex().gap(px(space::S8)).child(layer_label(cx, copy::DIAG_LVM.short)).child(lvm_row))
        .when_some(m.boot_note.clone(), |d, n| d.child(kit::caption(cx, SharedString::from(n))))
        .when(!m.untouched.is_empty(), |d| d.child(kit::caption(cx, format!("{}: {}", copy::DIAG_UNTOUCHED.short, m.untouched.join(", ")))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::inspect::{fixtures, parse_inspect};
    use synofurb_core::plan::{PlanRequest, build_plan};

    #[test]
    fn model_follows_the_plan() {
        let f = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let plan = build_plan(&f, &PlanRequest::default());
        let m = DiagramModel::from_plan(&plan);
        assert_eq!(m.disks.len(), 4, "one column per system/data disk in the plan");
        assert!(m.disks.iter().all(|d| !d.parts.is_empty() && d.new_table));
        assert!(!m.arrays.is_empty());
        assert!(m.vgs.iter().any(|v| v.name == "vg_data" && v.lvs.iter().any(|l| l.mountpoint == "/srv/data")));
        let total_members: usize = m.arrays.iter().map(|a| a.members.len()).sum();
        let in_array = m.disks.iter().flat_map(|d| &d.parts).filter(|p| p.array.is_some()).count();
        assert_eq!(total_members, in_array, "every array member is a partition drawn on a disk");
        assert!(m.arrays.iter().any(|a| a.level == "RAID 1 mirror"));
    }

    #[test]
    fn model_is_empty_for_an_empty_plan_and_does_not_panic() {
        let f = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let mut f2 = f.clone();
        f2.disks.clear();
        let plan = build_plan(&f2, &PlanRequest::default());
        let m = DiagramModel::from_plan(&plan);
        assert!(m.disks.is_empty());
    }
}
