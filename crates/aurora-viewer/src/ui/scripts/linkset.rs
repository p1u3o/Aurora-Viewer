//! The linkset panel of an object's script editor: every script of the
//! linkset beside the source, grouped by the prim that holds it, and the tabs
//! of the scripts the window has open. Aurora's own layout (the Legacy Viewer
//! opens one floater per script, LLPreviewLSL / LLLiveLSLEditor,
//! indra/newview/llpreviewscript.cpp): the tree takes the rows of the
//! inventory's contents (`ui/inventory/tree.rs`), the tabs the look of
//! `widgets::tabs`.

use super::super::widgets;
use super::super::{icons, inventory};
use crate::theme::Palette;
use aurora_net::scripts::ScriptTarget;
use egui::{Id, RichText, Sense, Vec2};
use uuid::Uuid;

/// Room the tree keeps beside the source.
pub const TREE_WIDTH: f32 = 186.0;

/// A script of the linkset, as the tree shows it.
pub struct Script {
    pub item: Uuid,
    pub name: String,
    /// The item's flags: an SLua script gets the Lua icon.
    pub flags: u32,
    /// Whether it runs, once the simulator answered.
    pub running: Option<bool>,
}

/// A prim of the linkset: one group of the tree.
pub struct Prim {
    pub object: Uuid,
    pub name: String,
    /// Its listing is on its way, or was refused: the group does not claim
    /// to hold no script.
    pub loading: bool,
    pub scripts: Vec<Script>,
}

/// Every script of a linkset, grouped by prim, as the app hands it to the
/// editor each frame.
pub struct Linkset {
    pub root: Uuid,
    pub name: String,
    pub prims: Vec<Prim>,
}

fn collapsed_id(root: Uuid) -> Id {
    Id::unique(("script_linkset_collapsed", root))
}

/// The prims whose group is shut, kept with the window's state.
fn collapsed(ctx: &egui::Context, root: Uuid) -> Vec<Uuid> {
    ctx.data_mut(|d| d.get_persisted::<Vec<Uuid>>(collapsed_id(root)))
        .unwrap_or_default()
}

/// The row icon of a name of the skin's set (Tabler and inventory icons
/// alike), `16` points wide, or the room it takes when the set has none.
fn icon(ui: &mut egui::Ui, name: &str, tint: egui::Color32) {
    if let Some(t) = icons::global(name) {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(tint));
    } else {
        ui.add_space(16.0);
    }
}

/// The tree of the linkset: a group per prim, its scripts under it. Returns
/// the script of a clicked row.
pub(super) fn tree(ui: &mut egui::Ui, p: &Palette, linkset: Option<&Linkset>, active: Option<ScriptTarget>) -> Option<ScriptTarget> {
    let ctx = ui.ctx().clone();
    let mut clicked = None;
    ui.label(RichText::new(aurora_i18n::tr!("script-linkset-scripts")).size(11.0).color(p.muted));
    let height = (ui.available_height() - 6.0).max(60.0);
    inventory::tree::frame(ui, p, |ui| {
        inventory::tree::spacing(ui);
        inventory::tree::scroll(ui, height - 8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(linkset) = linkset else {
                ui.label(RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.0).color(p.muted));
                return;
            };
            let mut closed = collapsed(&ctx, linkset.root);
            for prim in &linkset.prims {
                let open = !closed.contains(&prim.object);
                let (caret, label) = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let caret = inventory::tree::caret(ui, p, open);
                        icon(ui, "Inv_Object", p.violet_light);
                        let name = inventory::tree::name(&prim.name, p);
                        (caret, inventory::tree::label(ui, name, false, Sense::click(), None))
                    })
                    .inner;
                if caret.clicked() || label.clicked() {
                    if open {
                        closed.push(prim.object);
                    } else {
                        closed.retain(|id| *id != prim.object);
                    }
                }
                label.on_hover_text(if prim.loading {
                    aurora_i18n::tr!("script-linkset-loading", name = prim.name.as_str())
                } else {
                    aurora_i18n::tr!("script-linkset-count", name = prim.name.as_str(), count = prim.scripts.len() as i64)
                });
                if !open {
                    continue;
                }
                for script in &prim.scripts {
                    let target = ScriptTarget {
                        object: prim.object,
                        item: script.item,
                    };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add_space(ui.spacing().indent - 4.0);
                        let tint = if script.running == Some(true) { p.teal } else { p.muted };
                        icon(ui, inventory::item_icon(10, 10, script.flags), tint);
                        let name = inventory::tree::name(&script.name, p);
                        let selected = active == Some(target);
                        let response = inventory::tree::label(ui, name, selected, Sense::click(), None);
                        if response.clicked() {
                            clicked = Some(target);
                        }
                        response.on_hover_text(if script.running == Some(true) {
                            format!("{}\n{}", script.name, aurora_i18n::tr!("script-running"))
                        } else {
                            script.name.clone()
                        });
                    });
                }
            }
            if closed != collapsed(&ctx, linkset.root) {
                ctx.data_mut(|d| d.insert_persisted(collapsed_id(linkset.root), closed));
            }
        });
    });
    clicked
}

/// A tab of the strip: one open script.
pub(super) struct Tab<'a> {
    pub target: ScriptTarget,
    pub name: &'a str,
    pub lua: bool,
    pub dirty: bool,
}

/// What a click of the strip asked for.
pub(super) enum Tabs {
    None,
    Select(ScriptTarget),
    Close(ScriptTarget),
}

/// The strip of open scripts above the source: a click selects a tab, its
/// cross closes it (the middle button too, as in an editor's document strip).
pub(super) fn tabs(ui: &mut egui::Ui, p: &Palette, tabs: &[Tab<'_>], active: Option<ScriptTarget>) -> Tabs {
    let mut answer = Tabs::None;
    let font = widgets::control_font(ui, 12.0);
    let height = ui.spacing().interact_size.y.max(26.0);
    egui::ScrollArea::horizontal()
        .id_salt("script_linkset_tabs")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for tab in tabs {
                    let lua = u32::from(tab.lua);
                    let name = ui.painter().layout_no_wrap(tab.name.to_owned(), font.clone(), p.muted);
                    let width = 44.0 + name.size().x + if tab.dirty { 10.0 } else { 0.0 };
                    let selected = active == Some(tab.target);
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
                    if selected || response.hovered() {
                        let fill = if selected {
                            p.violet.gamma_multiply(0.18)
                        } else {
                            p.raised.gamma_multiply(0.25)
                        };
                        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
                    }
                    let center = rect.center().y;
                    let mut left = rect.left() + 8.0;
                    if let Some(t) = icons::global(inventory::item_icon(10, 10, lua)) {
                        ui.painter().image(
                            t.id(),
                            egui::Rect::from_center_size(egui::pos2(left + 7.0, center), Vec2::splat(14.0)),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            inventory::item_icon_color(10, 10, p),
                        );
                    }
                    left += 18.0;
                    ui.painter().galley(
                        egui::pos2(left, center - name.size().y / 2.0),
                        name.clone(),
                        if selected { p.ink } else { p.muted },
                    );
                    left += name.size().x;
                    if tab.dirty {
                        ui.painter().circle_filled(egui::pos2(left + 4.0, center), 2.0, p.amber);
                    }
                    let close = egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0, center), Vec2::splat(16.0));
                    let cross = ui.interact(close, response.id.with("close"), Sense::click());
                    if let Some(t) = icons::global("x") {
                        ui.painter().image(
                            t.id(),
                            egui::Rect::from_center_size(close.center(), Vec2::splat(11.0)),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            if cross.hovered() { p.ink } else { p.muted },
                        );
                    }
                    let cross = cross.on_hover_text(aurora_i18n::tr!("script-linkset-close"));
                    // a press of the cross must not also select the tab
                    if cross.clicked() || response.middle_clicked() {
                        answer = Tabs::Close(tab.target);
                    } else if response.clicked() && !selected {
                        answer = Tabs::Select(tab.target);
                    }
                    response.on_hover_text(tab.name);
                }
            });
        });
    answer
}
