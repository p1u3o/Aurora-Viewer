//! The linkset panel of an object's script editor: every script of the
//! linkset beside the source, grouped by the prim that holds it and marked
//! with a star while its open document has unsaved text. Aurora's own layout
//! (the Legacy Viewer opens one floater per script, LLPreviewLSL /
//! LLLiveLSLEditor, indra/newview/llpreviewscript.cpp): the tree takes the
//! rows of the inventory's contents (`ui/inventory/tree.rs`).

use super::super::widgets;
use super::super::{icons, inventory, menu};
use crate::theme::Palette;
use aurora_net::scripts::ScriptTarget;
use egui::{Id, RichText, Sense, Vec2};
use uuid::Uuid;

/// Room the tree keeps beside the source.
pub const TREE_WIDTH: f32 = 186.0;

/// What the contents window's menu offers for one script of the tree, as the
/// app decided it: the tree applies the contents window's own rules, it does
/// not read permissions itself.
#[derive(Clone, Copy)]
pub struct ScriptMenu {
    /// "Move to inventory" rather than "Copy to inventory": the item leaves
    /// the primitive (it has no Copy right).
    pub moves: bool,
    /// Why the copy is refused, when it is: the entry is greyed with it.
    pub refused: Option<&'static str>,
    /// The hint of the entry when it is offered.
    pub tip: &'static str,
    /// The prim may be modified: Delete is offered.
    pub can_delete: bool,
}

/// A script of the linkset, as the tree shows it.
pub struct Script {
    pub item: Uuid,
    pub name: String,
    /// The item's flags: an SLua script gets the Lua icon.
    pub flags: u32,
    /// Whether it runs, once the simulator answered.
    pub running: Option<bool>,
    /// Its open document has unsaved text: the row marks it with a star, in
    /// place of a tab strip's dot.
    pub dirty: bool,
    pub menu: ScriptMenu,
}

/// A prim of the linkset: one group of the tree.
pub struct Prim {
    pub object: Uuid,
    pub name: String,
    /// Its listing is on its way, or was refused: the group does not claim
    /// to hold no script.
    pub loading: bool,
    /// The prim may be modified: "New script" is offered.
    pub can_new_script: bool,
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

/// What a row of the tree asked for, from a click or its context menu.
pub(super) enum TreeAction {
    /// Open the script in the editor, or bring it to the front.
    Open(ScriptTarget),
    /// Close its document (middle click, or the menu's Close).
    Close(ScriptTarget),
    Take(ScriptTarget),
    Delete(ScriptTarget),
    /// A new script in that prim's contents.
    NewScript(Uuid),
}

/// The tree of the linkset: a group per prim, its scripts under it. Returns
/// what a click or a menu entry of a row asked for. `filter` hides the
/// scripts whose name does not contain its text (a prim whose name matches
/// keeps all of its scripts).
pub(super) fn tree(
    ui: &mut egui::Ui,
    p: &Palette,
    linkset: Option<&Linkset>,
    active: Option<ScriptTarget>,
    filter: &mut String,
) -> Option<TreeAction> {
    let ctx = ui.ctx().clone();
    let mut chosen = None;
    let width = ui.available_width();
    let (_, cleared) = inventory::tree::search_field(
        ui,
        p,
        filter,
        aurora_i18n::tr!("script-linkset-filter"),
        width,
        !filter.is_empty(),
        aurora_i18n::tr!("script-linkset-filter-clear"),
    );
    if cleared {
        filter.clear();
    }
    let needle = filter.trim().to_lowercase();
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
            let filtering = !needle.is_empty();
            for prim in &linkset.prims {
                let prim_matches = prim.name.to_lowercase().contains(&needle);
                let scripts: Vec<&Script> = prim
                    .scripts
                    .iter()
                    .filter(|script| !filtering || prim_matches || script.name.to_lowercase().contains(&needle))
                    .collect();
                if filtering && !prim_matches && scripts.is_empty() {
                    continue;
                }
                let open = filtering || !closed.contains(&prim.object);
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
                let label = label.on_hover_text(if prim.loading {
                    aurora_i18n::tr!("script-linkset-loading", name = prim.name.as_str())
                } else {
                    aurora_i18n::tr!("script-linkset-count", name = prim.name.as_str(), count = prim.scripts.len() as i64)
                });
                menu::context_menu(&label, p, |ui| {
                    if menu::item_if(ui, p, "plus", aurora_i18n::tr!("common-new-script"), prim.can_new_script) {
                        chosen = Some(TreeAction::NewScript(prim.object));
                    }
                });
                if !open {
                    continue;
                }
                for script in scripts {
                    let target = ScriptTarget {
                        object: prim.object,
                        item: script.item,
                    };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add_space(ui.spacing().indent - 4.0);
                        let tint = if script.running == Some(true) { p.teal } else { p.muted };
                        icon(ui, inventory::item_icon(10, 10, script.flags), tint);
                        // the star of unsaved text, in place of a tab strip's dot
                        let name = if script.dirty {
                            widgets::name_with_suffix(&script.name, " *", inventory::tree::font(), p.ink)
                        } else {
                            inventory::tree::name(&script.name, p)
                        };
                        let selected = active == Some(target);
                        let response = inventory::tree::label(ui, name, selected, Sense::click(), None);
                        if response.clicked() {
                            chosen = Some(TreeAction::Open(target));
                        }
                        if response.middle_clicked() {
                            chosen = Some(TreeAction::Close(target));
                        }
                        let response = response.on_hover_text(if script.running == Some(true) {
                            format!("{}\n{}", script.name, aurora_i18n::tr!("script-running"))
                        } else {
                            script.name.clone()
                        });
                        // the entries of the contents window's menu, with the
                        // rights it reads
                        menu::context_menu(&response, p, |ui| {
                            if menu::item(ui, p, "folder-open-filled", aurora_i18n::tr!("common-open")) {
                                chosen = Some(TreeAction::Open(target));
                            }
                            let label = if script.menu.moves {
                                aurora_i18n::tr!("task-transfer-move-menu")
                            } else {
                                aurora_i18n::tr!("build-ui-contents-copy-to-inventory")
                            };
                            let tip = script.menu.refused.unwrap_or(script.menu.tip);
                            if menu::item_tip(ui, p, "briefcase-filled", label, script.menu.refused.is_none(), tip) {
                                chosen = Some(TreeAction::Take(target));
                            }
                            if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), script.menu.can_delete) {
                                chosen = Some(TreeAction::Delete(target));
                            }
                            menu::separator(ui, p);
                            if menu::item(ui, p, "x", aurora_i18n::tr!("common-close")) {
                                chosen = Some(TreeAction::Close(target));
                            }
                        });
                    });
                }
            }
            if closed != collapsed(&ctx, linkset.root) {
                ctx.data_mut(|d| d.insert_persisted(collapsed_id(linkset.root), closed));
            }
        });
    });
    chosen
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

/// The strip of open scripts, flush above the source: the tab in front takes
/// the editor's own background, so it sits on its top edge. A tab marks its
/// unsaved text with a star, as the tree does, and its cross closes it (the
/// middle button too).
pub(super) fn tabs(ui: &mut egui::Ui, p: &Palette, tabs: &[Tab<'_>], active: Option<ScriptTarget>, background: egui::Color32) -> Tabs {
    let mut answer = Tabs::None;
    let font = widgets::control_font(ui, 12.0);
    let height = 24.0;
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    egui::ScrollArea::horizontal()
        .id_salt("script_linkset_tabs")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for tab in tabs {
                    let label = if tab.dirty {
                        format!("{} *", tab.name)
                    } else {
                        tab.name.to_owned()
                    };
                    let name = ui.painter().layout_no_wrap(label, font.clone(), p.muted);
                    let width = 26.0 + name.size().x + 20.0;
                    let selected = active == Some(tab.target);
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
                    let fill = if selected {
                        background
                    } else if response.hovered() {
                        p.raised.gamma_multiply(0.30)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
                    let center = rect.center().y;
                    let mut left = rect.left() + 6.0;
                    if let Some(t) = icons::global(inventory::item_icon(10, 10, u32::from(tab.lua))) {
                        ui.painter().image(
                            t.id(),
                            egui::Rect::from_center_size(egui::pos2(left + 6.0, center), Vec2::splat(13.0)),
                            uv,
                            inventory::item_icon_color(10, 10, p),
                        );
                    }
                    left += 15.0;
                    ui.painter().galley(
                        egui::pos2(left, center - name.size().y / 2.0),
                        name.clone(),
                        if selected { p.ink } else { p.muted },
                    );
                    let close = egui::Rect::from_center_size(egui::pos2(rect.right() - 9.0, center), Vec2::splat(15.0));
                    let cross = ui.interact(close, response.id.with("close"), Sense::click());
                    if let Some(t) = icons::global("x") {
                        ui.painter().image(
                            t.id(),
                            egui::Rect::from_center_size(close.center(), Vec2::splat(10.0)),
                            uv,
                            if cross.hovered() { p.ink } else { p.muted },
                        );
                    }
                    let cross = cross.on_hover_text(aurora_i18n::tr!("script-linkset-close"));
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
