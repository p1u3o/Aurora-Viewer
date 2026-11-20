//! LLScriptEdCore menus / button bar and LLScriptEditor gutter (Firestorm,
//! LGPL 2.1). Aurora keeps its flat widgets and always-visible experience list.

use super::{
    Action, Editor, complete, crumbs,
    dialogs::{Help, Panels, Search},
    editing, files, indent, reference,
    settings::{ColorRole, ScriptSettings},
};
use crate::{
    theme::Palette,
    ui::{
        inventory, menu,
        widgets::{self, Floater, flat_button, icon_button},
    },
};
use aurora_net::scripts::ScriptTarget;
use egui::{Id, RichText, Vec2};

#[derive(Default)]
pub(super) struct Tools {
    pub focus_start: bool,
    /// The keyboard to this script, asked once every source of the frame is
    /// drawn: a request made before a text edit draws does not hold (a tab
    /// chosen by a key or a click takes it).
    pub focus_request: bool,
    pub body_top: f32,
    pub search_slot: Option<egui::Ui>,
    header_width: f32,
    header_height: f32,
    pub help: Help,
    pub insert_filter: String,
    pub insert_button: Option<egui::Rect>,
    insertion_open: bool,
    pub library: bool,
    pub goto: bool,
    pub goto_line: usize,
    pub processed: bool,
    pub processed_source: String,
    pub has_processed: bool,
    /// Where the strip (the minimap, or the thin overview) was drawn this
    /// frame.
    pub overview: Option<egui::Rect>,
    /// The minimap of the text as last made, with the text's hash: the map is
    /// made again only when the script changes.
    minimap: Option<(u64, super::minimap::Map)>,
    pub position: (usize, usize),
    pub scroll_to_cursor: bool,
    /// The line at the top of the source's viewport, 0-based, as the last
    /// frame showed it: what the sticky line above the text names.
    pub first_visible: usize,
    /// The line at its bottom, for the minimap's window.
    pub last_visible: usize,
    /// A click on the text this frame, without Ctrl: the split's pane that is
    /// not in front takes the front with it.
    pub clicked: bool,
    /// The name Ctrl+click (or the context menu) asked the window to take the
    /// caret to the declaration of.
    pub goto_word: Option<String>,
    /// The word the context menu asked the dock to find in the window's
    /// scripts.
    pub references_word: Option<String>,
    pub revert: bool,
    pub file_job: Option<files::Job>,
    pub file_snapshot: String,
    pub external: Option<files::Watch>,
    /// The external editor is being launched: the script stays followed
    /// by the editor sync while its watch is replaced.
    pub external_starting: bool,
    pub external_snapshot: String,
    pub incoming: Option<String>,
    incoming_external: bool,
    pub diagnostics_processed: bool,
    pub reset_history: bool,
    /// The notice over this source the reader has put away: it comes back when
    /// another one takes its place.
    pub(super) notice_hidden: Option<Notice>,
    /// The dock's Check again, asked: the window reads again the object's
    /// scripts that could not be read.
    pub(super) check_again: bool,
    /// The snippet the reader is filling, its placeholders walked with Tab.
    pub(super) snippet: Option<complete::Snippet>,
    footer_height: f32,
    can_undo: bool,
    can_redo: bool,
    history_cursor: Option<egui::text::CCursorRange>,
}

impl Tools {
    pub fn opening() -> Self {
        Self {
            focus_start: true,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Command {
    #[default]
    Save,
    SaveAll,
    Reload,
    Revert,
    Open,
    Export,
    Preferences,
    External,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Deselect,
    Find,
    Goto,
    Help,
    Keyword,
    Library,
    Debug,
    Shortcuts,
    Indent,
    Outdent,
    LineComment,
    BlockComment,
    FoldAll,
    ExpandAll,
}

fn visible_id(id: Id, processed: bool) -> Id {
    if processed { id.with("processed") } else { id }
}

fn text(e: &Editor) -> &str {
    if e.tools.processed { &e.tools.processed_source } else { &e.source }
}

fn enabled(command: Command, e: &Editor, ctx: &egui::Context, id: Id) -> bool {
    let editable = e.can_edit && e.loaded && e.saving.is_none() && e.loading.is_none() && !e.tools.processed && !e.colors.active();
    let selection = !editing::cursor(ctx, visible_id(id, e.tools.processed)).is_empty();
    match command {
        Command::Save => e.can_save(),
        Command::Reload => e.can_read && !e.dirty() && e.saving.is_none() && e.loading.is_none(),
        Command::Revert => editable && e.dirty(),
        Command::Open | Command::External => editable && e.tools.file_job.is_none(),
        // a read-only editor saves no file (LLTextEditor::canLoadOrSaveToFile,
        // indra/llui/lltexteditor.cpp: !mReadOnly, set from is_modifiable in
        // LLPreviewLSL::loadAsset, indra/newview/llpreviewscript.cpp)
        Command::Export => e.loaded && e.can_edit && e.tools.file_job.is_none(),
        Command::Cut => editable && selection,
        Command::Copy => e.loaded && selection,
        Command::Paste => editable,
        Command::Undo => editable && e.tools.can_undo,
        Command::Redo => editable && e.tools.can_redo,
        Command::Indent | Command::Outdent | Command::LineComment | Command::BlockComment => editable,
        Command::FoldAll | Command::ExpandAll => e.loaded,
        Command::SelectAll | Command::Deselect | Command::Find | Command::Goto | Command::Keyword => e.loaded,
        _ => true,
    }
}

pub(super) fn run(
    command: Command,
    e: &mut Editor,
    ctx: &egui::Context,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
) {
    if !enabled(command, e, ctx, id) {
        return;
    }
    let owner = id;
    let id = visible_id(id, e.tools.processed);
    match command {
        Command::Save => {
            if let Some(action) = e.save() {
                actions.push(action);
            }
        }
        Command::SaveAll => actions.push(Action::SaveAll {
            linkset: e.linkset,
            target: e.target,
        }),
        Command::Reload => {
            let request = uuid::Uuid::new_v4();
            e.loading = Some(request);
            e.loaded = false;
            e.status = aurora_i18n::tr!("common-loading-2").into();
            actions.push(Action::Reload { request, target: e.target });
        }
        Command::Revert => e.tools.revert = true,
        Command::Open => {
            e.tools.file_snapshot.clone_from(&e.source);
            e.tools.file_job = Some(files::open(settings.convert_tabs));
        }
        Command::Export => e.tools.file_job = Some(files::save(e.name.clone(), e.source.clone(), e.lua())),
        Command::Preferences => {
            prefs.preferences.open = true;
            // the panel is drawn under the windows: bring it over them, or a
            // large script window hides it entirely
            crate::ui::widgets::bring_floater_front(ctx, super::dialogs::Preferences::WINDOW);
        }
        Command::External => {
            e.tools.file_snapshot.clone_from(&e.source);
            // the script takes the same file again: the watch of the last
            // launch ends now, not once the new one has written it
            e.tools.external = None;
            e.tools.external_starting = true;
            e.tools.file_job = Some(files::external(
                e.name.clone(),
                aurora_editor_sync::subscriptions::script_id(e.target.object, e.target.item),
                e.source.clone(),
                settings.external_editor.clone(),
                settings.convert_tabs,
                e.lua(),
            ));
        }
        Command::Undo | Command::Redo => {
            editing::undo(ctx, id, &mut e.source, matches!(command, Command::Redo));
            e.tools.scroll_to_cursor = true;
        }
        Command::Cut | Command::Copy => {
            let selected = editing::bytes(text(e), editing::cursor(ctx, id));
            ctx.copy_text(text(e)[selected].into());
            if matches!(command, Command::Cut) {
                let [a, b] = editing::cursor(ctx, id).sorted_cursors();
                editing::replace(ctx, id, &mut e.source, a.index.0..b.index.0, "");
            }
        }
        Command::Paste => actions.push(Action::Paste(e.target)),
        Command::SelectAll => editing::select(ctx, id, 0..text(e).chars().count(), true),
        Command::Deselect => {
            let at = editing::cursor(ctx, id).primary.index.0;
            editing::select(ctx, id, at..at, true);
        }
        Command::Find => prefs.search.begin(ctx, owner, id, text(e)),
        Command::Goto => e.tools.goto = true,
        Command::Shortcuts => {
            prefs.shortcuts = true;
            // the windows are drawn before this one: bring it over them
            crate::ui::widgets::bring_floater_front(ctx, super::dialogs::Shortcuts::WINDOW);
        }
        Command::Help => ctx.open_url(egui::OpenUrl::new_tab(reference::url("", e.lua()))),
        Command::Keyword => {
            e.tools.help.open = true;
            e.tools.help.visit(editing::word(
                if e.tools.processed { &e.tools.processed_source } else { &e.source },
                editing::cursor(ctx, id).primary.index.0,
                e.lua(),
            ));
        }
        Command::Library => e.tools.library = true,
        Command::Debug => actions.push(Action::Debug),
        Command::Indent | Command::Outdent | Command::LineComment | Command::BlockComment => {
            let command = match command {
                Command::Indent => editing::SelectionCommand::Indent,
                Command::Outdent => editing::SelectionCommand::Outdent,
                Command::LineComment => editing::SelectionCommand::LineComment,
                Command::BlockComment => editing::SelectionCommand::BlockComment,
                _ => unreachable!("selection command"),
            };
            if !editing::selection_command(ctx, id, &mut e.source, command) {
                e.status = aurora_i18n::tr!("script-selection-edit-failed").into();
            }
        }
        Command::FoldAll => {
            e.folding
                .sync(if e.tools.processed { &e.tools.processed_source } else { &e.source });
            e.folding.collapse_all();
            editing::select(ctx, id, 0..0, false);
        }
        Command::ExpandAll => e.folding.expand_all(),
    }
}

/// One line of the Problems dock: a diagnostic of a script the window shows.
pub(super) struct Problem {
    pub source: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// What a script has to reckon with, as the notice over its source says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    /// Its item could not be read: the viewer's reason is in the status.
    Failed,
    /// Saved, but the compiler refused the source.
    CompileFailed,
    /// Longer than the viewer can save.
    TooLarge,
    /// Readable, but the item gives no modify right.
    ReadOnly,
    /// The simulator has not said whether the script runs.
    NotRunning,
}

/// The notice a script asks for, the most urgent first.
fn notice(e: &Editor) -> Option<Notice> {
    if !e.loaded && e.loading.is_none() && !e.status.is_empty() {
        Some(Notice::Failed)
    } else if e.compile_failed && !e.status.is_empty() {
        Some(Notice::CompileFailed)
    } else if e.source.len() > aurora_net::scripts::MAX_SOURCE {
        Some(Notice::TooLarge)
    } else if e.loaded && !e.can_edit {
        Some(Notice::ReadOnly)
    } else if !e.target.object.is_nil() && e.loaded && e.can_read && e.running.is_none() {
        Some(Notice::NotRunning)
    } else {
        None
    }
}

/// The notice over the editor: what the script in front has to reckon with,
/// what can be done about it, and a cross that keeps it out of the way until
/// another one comes up (Alchemy's Script Studio notice bar,
/// ALScriptNoticeBar; Aurora says the same in one row of its own).
pub(super) fn notice_bar(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, actions: &mut Vec<Action>, tab: &mut DockTab) {
    let Some(kind) = notice(e) else {
        e.tools.notice_hidden = None;
        return;
    };
    if e.tools.notice_hidden == Some(kind) {
        return;
    }
    let (icon, tint, text) = match kind {
        Notice::Failed => ("alert-triangle-filled", p.danger, e.status.clone()),
        Notice::CompileFailed => (
            "alert-triangle-filled",
            p.danger,
            aurora_i18n::tr!("script-compile-failed").to_string(),
        ),
        Notice::TooLarge => (
            "alert-triangle-filled",
            p.danger,
            aurora_i18n::tr!("script-source-too-large").to_string(),
        ),
        Notice::ReadOnly => ("lock-filled", p.warn, aurora_i18n::tr!("script-read-only").to_string()),
        Notice::NotRunning => ("hourglass-filled", p.warn, aurora_i18n::tr!("script-waiting-running").to_string()),
    };
    let height = ui.spacing().interact_size.y.max(24.0);
    egui::Frame::new()
        .fill(p.bar.lerp_to_gamma(tint, 0.18))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                super::linkset::icon(ui, icon, tint);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), height),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("script-notice-hide"), true) {
                            e.tools.notice_hidden = Some(kind);
                        }
                        match kind {
                            Notice::CompileFailed => {
                                let clicked = flat_button(ui, p, aurora_i18n::tr!("script-notice-problems")).clicked();
                                if clicked {
                                    *tab = DockTab::Problems;
                                }
                            }
                            Notice::NotRunning => {
                                let clicked = flat_button(ui, p, aurora_i18n::tr!("script-retry")).clicked();
                                if clicked {
                                    actions.push(Action::QueryRunning(e.target));
                                }
                            }
                            _ => {}
                        }
                        // the words take the room left, on the left
                        let width = ui.available_width();
                        ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&text).size(11.0).color(p.ink))
                                    .truncate()
                                    .halign(egui::Align::LEFT),
                            )
                            .on_hover_text(&text);
                        });
                    },
                );
            });
        });
}

pub(super) fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
    own_menus: bool,
    problems: &[Problem],
    all_sources: Option<&mut bool>,
    folded: &mut bool,
    tab: &mut DockTab,
    search: Option<SearchDock<'_>>,
    references: Option<ReferencesDock<'_>>,
    check: Option<CheckDock>,
) -> Option<DockClick> {
    let ctx = ui.ctx().clone();
    e.tools.body_top = ui.cursor().top();
    if own_menus {
        // a window of its own: its menus and toolbar are the body's first rows
        e.tools.search_slot = None;
    }
    // the notice of what the script has to reckon with: a window of its own
    // puts it under its menus; a linkset window puts it over its tree and its
    // sources (`show_linkset`), so that both start level
    if own_menus {
        notice_bar(ui, p, e, actions, tab);
    }
    let gap = ui.spacing().item_spacing.x;
    let search_width = Search::inline_width(ui);
    let inline = own_menus
        && settings.search_inline
        && prefs.search.owns(id)
        && e.tools.header_width > 0.0
        && e.tools.header_height >= Search::inline_min_height(ui)
        && ui.available_width() >= e.tools.header_width + gap + search_width;
    if own_menus && inline {
        let left = ui.available_width() - gap - search_width;
        ui.horizontal_top(|ui| {
            let header = ui.allocate_ui_with_layout(Vec2::new(left, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                header(ui, p, e, id, settings, prefs, actions, own_menus);
            });
            e.tools.header_height = header.response.rect.height();
            let (rect, _) = ui.allocate_exact_size(Vec2::new(search_width, e.tools.header_height), egui::Sense::hover());
            if !ui.is_sizing_pass() && ui.is_enabled() {
                // Render the shared controls after all editor commands have
                // selected their owner, using this editor's layer and style.
                e.tools.search_slot = Some(
                    ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("inline_search")
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    ),
                );
            }
        });
    } else if own_menus {
        let header = ui.scope(|ui| header(ui, p, e, id, settings, prefs, actions, own_menus));
        e.tools.header_height = header.response.rect.height();
    }
    // Measure the actual controls, including object-only rows and diagnostics.
    // Repaint after a width/status change, without replaying asynchronous actions.
    let footer_height = if e.tools.footer_height > 0.0 {
        e.tools.footer_height
    } else {
        155.0
    };
    // The dock keeps its room, with "No problems." when everything compiles,
    // unless the window is too short for it and a readable source; the source
    // takes the rest (its frame paints 4 points above and below it, hence the
    // 8).
    let room = ui.available_height();
    let gap = ui.spacing().item_spacing.y;
    // A folded dock keeps only its header line.
    // the dock never takes more than a third of the window: a short window
    // keeps its text
    let dock_room = if *folded {
        DOCK_HEADER
    } else {
        dock_room(settings.dock_height, room)
    };
    let dock_height = if room >= footer_height + dock_room + SOURCE_MIN + gap * 2.0 + 8.0 {
        dock_room
    } else {
        0.0
    };
    let gaps = if dock_height > 0.0 { 2.0 } else { 1.0 };
    let height = (room - footer_height - dock_height - gap * gaps - 8.0).max(SOURCE_MIN);
    // the path of the caret, under the source: a row of its own, taken from the
    // source's room (a window too short keeps its text instead)
    let crumbs = height > CRUMBS_HEIGHT + SOURCE_MIN + gap;
    let height = if crumbs { height - CRUMBS_HEIGHT - gap } else { height };
    source(ui, p, e, id, settings, height, &prefs.search, problems);
    if crumbs {
        crumbs_row(ui, p, e, id, settings, problems, tab, true);
    }
    let clicked = if dock_height > 0.0 {
        dock(
            ui,
            p,
            e,
            problems,
            all_sources,
            folded,
            dock_height,
            tab,
            &settings.dock_order(),
            search,
            references,
            check,
        )
    } else {
        None
    };
    let footer = ui.scope(|ui| footer(ui, p, e));
    if (e.tools.footer_height - footer.response.rect.height()).abs() > 0.5 {
        e.tools.footer_height = footer.response.rect.height();
        ctx.request_repaint();
    }
    clicked
}

/// Which pane of a split holds the script in front: the window's dock, its
/// readout and its search are that one's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

/// Two scripts side by side in one window: the rows above each pane's source
/// on one line (so both sources start level), then both sources, then the
/// window's dock and the status bar of the script in front. Clicking a pane's
/// source brings that script to the front, without moving either pane.
#[allow(clippy::too_many_arguments)]
pub(super) fn split_body(
    ui: &mut egui::Ui,
    p: &Palette,
    left: &mut Editor,
    left_id: Id,
    right: &mut Editor,
    right_id: Id,
    focused: Side,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    problems: &[Problem],
    all_sources: Option<&mut bool>,
    folded: &mut bool,
    tab: &mut DockTab,
    search: Option<SearchDock<'_>>,
    references: Option<ReferencesDock<'_>>,
    check: Option<CheckDock>,
) -> (Option<DockClick>, Option<ScriptTarget>) {
    // Two equal columns: egui splits the width exactly, and each column's own
    // layout keeps the two sources at the same height.
    let room = ui.available_height();
    let gap_y = ui.spacing().item_spacing.y;
    let footer_height = if left.tools.footer_height > 0.0 {
        left.tools.footer_height
    } else {
        155.0
    };
    // the dock never takes more than a third of the window: a short window
    // keeps its text
    let dock_room = if *folded {
        DOCK_HEADER
    } else {
        dock_room(settings.dock_height, room)
    };
    let dock_height = if room >= footer_height + dock_room + SOURCE_MIN + gap_y * 2.0 + 8.0 {
        dock_room
    } else {
        0.0
    };
    let gaps = if dock_height > 0.0 { 2.0 } else { 1.0 };
    let height = (room - footer_height - dock_height - gap_y * gaps - 8.0).max(SOURCE_MIN);
    // the path of the caret of each pane, on one row under the two sources
    let crumbs = height > CRUMBS_HEIGHT + SOURCE_MIN + gap_y;
    let height = if crumbs { height - CRUMBS_HEIGHT - gap_y } else { height };
    ui.columns(2, |columns| {
        left.tools.body_top = columns[0].cursor().top();
        source(&mut columns[0], p, left, left_id, settings, height, &prefs.search, problems);
        right.tools.body_top = columns[1].cursor().top();
        source(&mut columns[1], p, right, right_id, settings, height, &prefs.search, problems);
    });
    if crumbs {
        ui.columns(2, |columns| {
            let left_front = focused == Side::Left;
            crumbs_row(&mut columns[0], p, left, left_id, settings, problems, tab, left_front);
            crumbs_row(&mut columns[1], p, right, right_id, settings, problems, tab, !left_front);
        });
    }
    // a click in the pane that is not in front brings its script to the front
    let focus = match focused {
        Side::Left if right.tools.clicked => Some(right.target),
        Side::Right if left.tools.clicked => Some(left.target),
        _ => None,
    };
    let front = match focused {
        Side::Left => &mut *left,
        Side::Right => &mut *right,
    };
    let clicked = if dock_height > 0.0 {
        dock(
            ui,
            p,
            front,
            problems,
            all_sources,
            folded,
            dock_height,
            tab,
            &settings.dock_order(),
            search,
            references,
            check,
        )
    } else {
        None
    };
    let footer = ui.scope(|ui| footer(ui, p, front));
    if (front.tools.footer_height - footer.response.rect.height()).abs() > 0.5 {
        front.tools.footer_height = footer.response.rect.height();
        ui.ctx().request_repaint();
    }
    (clicked, focus)
}

/// The room the Problems dock takes under the source.
pub(super) const DOCK_HEIGHT: f32 = 146.0;
/// The least room the dock keeps when its edge is dragged: its header and a
/// row of its table; and the most, before the window's own room has a say.
const DOCK_LEAST: f32 = 90.0;
const DOCK_MOST: f32 = 520.0;
/// Folded, it keeps the room of its header alone.
const DOCK_HEADER: f32 = 30.0;
/// The least height the source keeps: a short window gives the dock's room to
/// the source instead.
const SOURCE_MIN: f32 = 120.0;

/// The room the dock under the source takes in a window body `room` points
/// tall: what the reader dragged its edge to -- never less than its least, nor
/// more than three fifths of the window, so that the text keeps the rest -- or,
/// when nothing was dragged, the default, which a short window brings down to
/// a third.
fn dock_room(stored: u16, room: f32) -> f32 {
    let wanted = if stored == 0 { DOCK_HEIGHT } else { f32::from(stored) };
    let share = if stored == 0 { 0.30 } else { 0.60 };
    wanted.clamp(DOCK_LEAST, DOCK_MOST).min((room * share).max(DOCK_LEAST)).ceil()
}

/// Which list the dock under the source shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DockTab {
    /// LLScriptEdCore's error list, given columns.
    #[default]
    Problems,
    /// The states, events, functions and globals of the script in front.
    Outline,
    /// A word found in every script of the window.
    Search,
    /// Every use of a name in the window's scripts, to rename them at once.
    References,
}

impl DockTab {
    /// Its name in the settings, where the reader's order is kept.
    pub(super) fn name(self) -> &'static str {
        match self {
            DockTab::Problems => "problems",
            DockTab::Outline => "outline",
            DockTab::Search => "search",
            DockTab::References => "references",
        }
    }

    /// The tab a name of the settings stands for.
    pub(super) fn from_name(name: &str) -> Option<Self> {
        [DockTab::Problems, DockTab::Outline, DockTab::Search, DockTab::References]
            .into_iter()
            .find(|tab| tab.name() == name)
    }
}

/// What a click in the dock asked for.
pub(super) enum DockClick {
    /// A row of the diagnostics, by its place in the list.
    Problem(usize),
    /// An item of the outline, by its place in it.
    Outline(usize),
    /// A hit of the search, by its place in the list.
    Search(usize),
    /// A reference of the References tab, by its place in the list.
    Reference(usize),
    /// Rename every reference of the tab to the new name.
    Rename,
    /// Check every script of the window's object.
    Check,
    /// The tabs of the dock, dragged into another order.
    Reorder([DockTab; 4]),
    /// The dock's edge dragged to a new height.
    Resize(f32),
}

/// The Search tab's state for one frame: the query being typed and the hits
/// the window's scripts gave it.
pub(super) struct SearchDock<'a> {
    pub query: &'a mut String,
    pub hits: &'a [super::navigate::Hit],
    /// Every script of the linkset, the ones no tab holds read from their prim.
    pub object: &'a mut bool,
}

/// What the Problems dock says of the object: the scripts it is still
/// reading, and what it found in all of them.
#[derive(Clone, Copy)]
pub(super) struct CheckDock {
    pub reading: usize,
    /// A script could not be read: offer to read the object's scripts again.
    pub retry: bool,
}

/// The References tab's state for one frame: the name the window is renaming,
/// the new name being typed and every place the name is used.
pub(super) struct ReferencesDock<'a> {
    pub word: &'a str,
    pub name: &'a mut String,
    pub hits: &'a [super::navigate::Reference],
}

/// The outline of a document, as the text on screen declares it.
pub(super) fn outline_items(e: &Editor) -> Vec<super::outline::Item> {
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    super::outline::outline(text, e.lua())
}

/// The bookmark F2 (next) or Shift+F2 (previous) goes to from the caret's
/// line, wrapping at either end.
pub(super) fn bookmark_step(lines: &std::collections::BTreeSet<usize>, at: usize, next: bool) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    if next {
        lines.range(at + 1..).next().copied().or_else(|| lines.iter().next().copied())
    } else {
        lines.range(..at).next_back().copied().or_else(|| lines.iter().next_back().copied())
    }
}

/// Puts the cursor on a line of the text on screen (the outline's rows).
pub(super) fn go_to_line(ctx: &egui::Context, e: &mut Editor, id: Id, line: usize, column: usize) {
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    let at = editing::line_cursor(text, line, column);
    editing::select(ctx, visible_id(id, e.tools.processed), at..at, true);
    e.tools.scroll_to_cursor = true;
}

/// The columns of the dock's tables, the same in every tab: the script, the
/// line, then what the row says.
const DOCK_SOURCE: f32 = 140.0;
const DOCK_LINE: f32 = 54.0;
/// The height of a row, and the text of a row and of its columns' header.
const DOCK_ROW: f32 = 16.0;
const DOCK_TEXT: f32 = 11.0;
const DOCK_HEAD: f32 = 10.5;
/// The status of the object's check, smaller than the rows it stands for.
const DOCK_STATUS: f32 = 10.0;

/// One cell of a tab's table: the text takes its room and starts at its left
/// edge (a sized label would centre).
fn cell(ui: &mut egui::Ui, width: f32, height: f32, text: RichText, truncate: bool) {
    ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let label = egui::Label::new(text);
        ui.add(if truncate { label.truncate() } else { label });
    });
}

/// The header of a dock table: the script, the line, and what the tab lists.
fn table_head(ui: &mut egui::Ui, p: &Palette, third: &str) {
    let text = (ui.available_width() - DOCK_SOURCE - DOCK_LINE - 16.0).max(60.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (label, width) in [
            (aurora_i18n::tr!("script-problems-source").to_string(), DOCK_SOURCE),
            (aurora_i18n::tr!("script-problems-line").to_string(), DOCK_LINE),
            (third.to_owned(), text),
        ] {
            cell(ui, width, DOCK_ROW, RichText::new(label).size(DOCK_HEAD).color(p.muted), false);
        }
    });
}

/// One row of a dock table, lit under the pointer: a click goes to its line.
/// `danger` writes what it says in the diagnostic red (a problem), `mono` in
/// the code's font (a problem or a line of a script).
fn table_row(ui: &mut egui::Ui, p: &Palette, source: &str, line: &str, text: &str, danger: bool, mono: bool) -> egui::Response {
    let width = (ui.available_width() - DOCK_SOURCE - DOCK_LINE - 16.0).max(60.0);
    // the highlight goes behind the row's text
    let beneath = ui.painter().add(egui::Shape::Noop);
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        cell(ui, DOCK_SOURCE, DOCK_ROW, RichText::new(source).size(DOCK_TEXT).color(p.ink), true);
        cell(ui, DOCK_LINE, DOCK_ROW, RichText::new(line).size(DOCK_TEXT).color(p.muted), false);
        let mut said = if mono {
            RichText::new(text).monospace()
        } else {
            RichText::new(text)
        }
        .size(DOCK_TEXT);
        said = said.color(if danger { p.danger } else { p.muted });
        cell(ui, width, DOCK_ROW, said, true);
    });
    let hit = ui.interact(row.response.rect, row.response.id.with("row"), egui::Sense::click());
    if hit.hovered() {
        ui.painter().set(
            beneath,
            egui::Shape::rect_filled(
                row.response.rect.expand2(Vec2::new(2.0, 0.0)),
                ui.visuals().widgets.inactive.corner_radius,
                p.raised.gamma_multiply(0.25),
            ),
        );
    }
    hit
}

/// What a table says while it has nothing to list.
fn table_none(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).size(DOCK_TEXT).color(p.muted));
}

/// The scope of the list shown, at the right of the dock's header: the same
/// choice in every tab, in the same place.
fn scope_dropdown(ui: &mut egui::Ui, id: &str, all: &mut bool, off: &str, on: &str) {
    egui::ComboBox::from_id_salt(id)
        .width(124.0)
        .selected_text(if *all { on } else { off })
        .show_ui(ui, |ui| {
            ui.selectable_value(all, false, off);
            ui.selectable_value(all, true, on);
        });
}

/// The name of a tab in the dock's header, with the count of the problems.
fn tab_label(tab: DockTab, problems: usize) -> String {
    match tab {
        DockTab::Problems => aurora_i18n::tr!("script-problems-count", count = problems as i64).to_string(),
        DockTab::Outline => aurora_i18n::tr!("script-dock-outline").to_string(),
        DockTab::Search => aurora_i18n::tr!("script-dock-search").to_string(),
        DockTab::References => aurora_i18n::tr!("script-dock-references").to_string(),
    }
}

/// The whole order of the tabs once `moved` is dropped on `onto`: before it
/// when it comes from its right, after it when it comes from its left.
fn reorder_tabs(order: [DockTab; 4], moved: DockTab, onto: DockTab) -> [DockTab; 4] {
    let from = order.iter().position(|tab| *tab == moved);
    let at = order.iter().position(|tab| *tab == onto);
    let (Some(from), Some(at)) = (from, at) else {
        return order;
    };
    if from == at {
        return order;
    }
    let mut rest: Vec<DockTab> = order.into_iter().filter(|tab| *tab != moved).collect();
    let anchor = rest.iter().position(|tab| *tab == onto).unwrap_or(rest.len().saturating_sub(1));
    let slot = if from < at { anchor + 1 } else { anchor };
    rest.insert(slot.min(rest.len()), moved);
    let mut next = order;
    for (slot, tab) in rest.into_iter().take(4).enumerate() {
        next[slot] = tab;
    }
    next
}

/// The outline list of the dock: each state, event, function and global of the
/// script in front with its line, a click away from it.
fn outline_dock(ui: &mut egui::Ui, p: &Palette, e: &Editor) -> Option<DockClick> {
    let items = outline_items(e);
    let mut clicked = None;
    let first = e.first_line();
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("script_outline")
        .max_height(ui.available_height().max(0.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if items.is_empty() {
                table_none(ui, p, aurora_i18n::tr!("script-outline-none").as_ref());
                return;
            }
            for (index, item) in items.iter().enumerate() {
                let (icon, tint) = match item.kind {
                    super::outline::Kind::State => ("circle-dot-filled", p.violet_light),
                    super::outline::Kind::Event => ("bolt-filled", p.teal),
                    super::outline::Kind::Function => ("function-filled", p.indigo_light),
                    super::outline::Kind::Global => ("variable", p.muted),
                };
                // the text of the dock's tables, at the same size
                let font = crate::ui::widgets::control_font(ui, DOCK_TEXT);
                let label = widgets::name_with_suffix(&item.name, &format!("   {}", item.line + first), font, p.ink);
                let row = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        super::linkset::icon(ui, icon, tint);
                        inventory::tree::label(ui, label, false, egui::Sense::click(), None)
                    })
                    .inner;
                if row.clicked() {
                    clicked = Some(DockClick::Outline(index));
                }
            }
        });
    clicked
}

/// The Search tab of the dock: a field and the word's hits in the window's
/// scripts, a click away from each. `query` is the window's own; the scope of
/// the search is the dock's header.
fn search_dock(ui: &mut egui::Ui, p: &Palette, search: SearchDock<'_>) -> Option<DockClick> {
    let SearchDock { query, hits, .. } = search;
    let mut clicked = None;
    let width = ui.available_width();
    let clear = !query.is_empty();
    ui.add_space(4.0);
    let (_, cleared) = inventory::tree::search_field(
        ui,
        p,
        query,
        aurora_i18n::tr!("script-search-in-scripts"),
        width,
        clear,
        aurora_i18n::tr!("script-linkset-filter-clear"),
    );
    if cleared {
        query.clear();
    }
    ui.add_space(4.0);
    table_head(ui, p, aurora_i18n::tr!("script-dock-text").as_ref());
    egui::ScrollArea::vertical()
        .id_salt("script_search")
        .max_height(ui.available_height().max(0.0))
        .show(ui, |ui| {
            if hits.is_empty() {
                table_none(ui, p, aurora_i18n::tr!("script-search-none").as_ref());
                return;
            }
            for (index, hit) in hits.iter().enumerate() {
                let row = ui
                    .push_id(("search", index), |ui| {
                        table_row(
                            ui,
                            p,
                            &hit.name,
                            &format!("{}:{}", hit.line + hit.first, hit.column),
                            &hit.text,
                            false,
                            true,
                        )
                    })
                    .inner;
                if row.clicked() {
                    clicked = Some(DockClick::Search(index));
                }
            }
        });
    clicked
}

/// The References tab of the dock: the name the window is renaming, a field for
/// the new one and the button that renames every place at once, then the places
/// themselves, a click away from each. `word` is the name the context menu
/// asked about.
fn references_dock(ui: &mut egui::Ui, p: &Palette, dock: ReferencesDock<'_>) -> Option<DockClick> {
    let ReferencesDock { word, name, hits } = dock;
    let mut clicked = None;
    let width = ui.available_width();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        // the new name, as the Search tab's field is drawn
        let ready = !name.trim().is_empty() && name.trim() != word;
        let field_width = (width - 250.0).clamp(80.0, 220.0);
        let (field, cleared) = inventory::tree::search_field(
            ui,
            p,
            name,
            aurora_i18n::tr!("script-references-new-name"),
            field_width,
            !name.is_empty(),
            aurora_i18n::tr!("script-linkset-filter-clear"),
        );
        if cleared {
            name.clear();
        }
        if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) && ready {
            clicked = Some(DockClick::Rename);
        }
        if ui
            .add_enabled_ui(ready, |ui| flat_button(ui, p, aurora_i18n::tr!("script-references-rename")))
            .inner
            .clicked()
        {
            clicked = Some(DockClick::Rename);
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("script-references-count", count = hits.len() as i64))
                .size(DOCK_TEXT)
                .color(p.muted),
        );
    });
    ui.add_space(4.0);
    table_head(ui, p, aurora_i18n::tr!("script-dock-text").as_ref());
    egui::ScrollArea::vertical()
        .id_salt("script_references")
        .max_height(ui.available_height().max(0.0))
        .show(ui, |ui| {
            if hits.is_empty() {
                table_none(ui, p, aurora_i18n::tr!("script-references-none").as_ref());
                return;
            }
            for (index, hit) in hits.iter().enumerate() {
                let row = ui
                    .push_id(("reference", index), |ui| {
                        table_row(
                            ui,
                            p,
                            &hit.name,
                            &format!("{}:{}", hit.line + hit.first, hit.column),
                            &hit.text,
                            false,
                            true,
                        )
                    })
                    .inner;
                if row.clicked() {
                    clicked = Some(DockClick::Reference(index));
                }
            }
        });
    clicked
}

/// The dock under the source: what the window has to show of the object and of
/// the script in front -- the problems, the outline, a word's hits or a name's
/// places -- with the same tabs, the same fields and the same table in every
/// one of them. Returns what a click asked for.
#[allow(clippy::too_many_arguments)]
fn dock(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    rows: &[Problem],
    all_sources: Option<&mut bool>,
    folded: &mut bool,
    height: f32,
    tab: &mut DockTab,
    order: &[DockTab; 4],
    mut search: Option<SearchDock<'_>>,
    references: Option<ReferencesDock<'_>>,
    check: Option<CheckDock>,
) -> Option<DockClick> {
    let mut clicked = None;
    // Exactly the room reserved for it (`body` below), whatever the list holds
    // and whatever the header measures: a dock that grew with its content grew
    // the whole window with it, frame after frame.
    let height = height.max(DOCK_HEADER);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    // the edge above the dock drags its height, and a double click takes the
    // default back, as the tree's edge does for its width
    if let Some(wanted) = widgets::resize_edge(
        ui,
        p,
        rect,
        height,
        DOCK_HEIGHT,
        DOCK_LEAST,
        DOCK_MOST,
        ui.scope_id().with("dock_resize"),
    ) {
        clicked = Some(DockClick::Resize(wanted));
    }
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let clip = rect.intersect(ui.clip_rect());
    ui.set_clip_rect(clip);
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.72))
        .corner_radius(ui.visuals().window_corner_radius)
        // the sides of the status bar below, so the scope's right edge
        // lines up with Save
        .inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 4,
            bottom: 4,
        })
        .show(&mut ui, |ui| {
            ui.set_min_size(ui.available_size());
            // the tabs, in the order the reader put them, and what the list
            // they show looks at, at the right of the same row
            let shown: Vec<DockTab> = order
                .iter()
                .copied()
                .filter(|tab| match tab {
                    DockTab::Search => search.is_some(),
                    DockTab::References => references.is_some(),
                    _ => true,
                })
                .collect();
            let labels: Vec<(String, bool)> = shown.iter().map(|tab| (tab_label(*tab, rows.len()), true)).collect();
            let labels: Vec<(&str, bool)> = labels.iter().map(|(label, enabled)| (label.as_str(), *enabled)).collect();
            // the status of the object's check and the scope's room, so that
            // the tabs take what is left of the row
            // what it is still reading; what it found is the list itself, so
            // that nothing repeats "No problems." under the header
            let status = (*tab == DockTab::Problems).then(|| match &check {
                Some(check) if check.reading > 0 => aurora_i18n::tr!("script-check-reading", count = check.reading as i64).to_string(),
                _ => String::new(),
            });
            let status = status.filter(|text| !text.is_empty());
            let retry = *tab == DockTab::Problems && check.as_ref().is_some_and(|check| check.retry);
            let font = crate::ui::widgets::control_font(ui, DOCK_STATUS);
            let status_width = status.as_ref().map_or(0.0, |text| {
                ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.clone(), font.clone(), p.muted).size().x)
            });
            let reserved = status_width + if status.is_some() { 24.0 } else { 0.0 } + 132.0;
            let row = 24.0f32.max(ui.spacing().interact_size.y);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // The chevron folds the dock to its header, as an IDE's
                // problems view does.
                let left = (ui.available_width() - reserved).max(140.0);
                ui.allocate_ui_with_layout(Vec2::new(left, row), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (icon, tip) = if *folded {
                        ("chevron-right", aurora_i18n::tr!("script-problems-unfold"))
                    } else {
                        ("chevron-down", aurora_i18n::tr!("script-problems-fold"))
                    };
                    if icon_button(ui, p, icon, tip, true) {
                        *folded = !*folded;
                    }
                    let current = shown.iter().position(|shown| shown == tab).unwrap_or(0);
                    let mut selected = current;
                    if let Some((from, to)) = widgets::tabs_reorderable(ui, p, &mut selected, &labels) {
                        clicked = Some(DockClick::Reorder(reorder_tabs(*order, shown[from], shown[to])));
                    } else if selected != current {
                        *tab = shown[selected];
                    }
                });
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), row),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // the scope of the list shown: the script in front or
                        // the object, the window's scripts or the object's
                        match *tab {
                            DockTab::Problems => {
                                if let Some(all) = all_sources {
                                    scope_dropdown(
                                        ui,
                                        "script_problems_scope",
                                        all,
                                        aurora_i18n::tr!("script-problems-this").as_ref(),
                                        aurora_i18n::tr!("script-problems-all").as_ref(),
                                    );
                                }
                            }
                            DockTab::Search => {
                                if let Some(search) = search.as_mut() {
                                    scope_dropdown(
                                        ui,
                                        "script_search_scope",
                                        search.object,
                                        aurora_i18n::tr!("script-search-scope-window").as_ref(),
                                        aurora_i18n::tr!("script-search-scope-object").as_ref(),
                                    );
                                }
                            }
                            _ => {}
                        }
                        if let Some(text) = status {
                            ui.label(RichText::new(text).size(DOCK_STATUS).color(p.muted));
                        }
                        // a script the region would not give: try again
                        if retry && widget_retry(ui, p) {
                            clicked = Some(DockClick::Check);
                        }
                    },
                );
            });
            if *folded {
                return;
            }
            // what the edge above the dock asked for keeps its place: the list
            // under it may have nothing to say
            if *tab == DockTab::Outline {
                clicked = clicked.take().or(outline_dock(ui, p, e));
                return;
            }
            if *tab == DockTab::Search
                && let Some(search) = search
            {
                clicked = clicked.take().or(search_dock(ui, p, search));
                return;
            }
            if *tab == DockTab::References
                && let Some(references) = references
            {
                clicked = clicked.take().or(references_dock(ui, p, references));
                return;
            }
            ui.add_space(4.0);
            table_head(ui, p, aurora_i18n::tr!("script-problems-message").as_ref());
            egui::ScrollArea::vertical()
                .id_salt("script_problems")
                .max_height(ui.available_height().max(0.0))
                .show(ui, |ui| {
                    if rows.is_empty() {
                        table_none(ui, p, aurora_i18n::tr!("script-problems-none").as_ref());
                        return;
                    }
                    for (index, row) in rows.iter().enumerate() {
                        let hit = ui
                            .push_id(("problem", index), |ui| {
                                table_row(
                                    ui,
                                    p,
                                    &row.source,
                                    &format!("{}:{}", row.line, row.column),
                                    &row.message,
                                    true,
                                    true,
                                )
                            })
                            .inner;
                        if hit.clicked() {
                            clicked = Some(DockClick::Problem(index));
                        }
                    }
                });
        });
    clicked
}

/// The button that reads the object's scripts again, beside what the check
/// says of them.
fn widget_retry(ui: &mut egui::Ui, p: &Palette) -> bool {
    icon_button(ui, p, "refresh", aurora_i18n::tr!("script-check-again"), true)
}

/// The File, Edit and Help menus of the editor. A window of the linkset's
/// scripts draws them once, above the tree and the tabs, for the script in
/// front. Insert, which inserts a function at the cursor, is one of them: the
/// Legacy Viewer keeps it in the editor's bottom bar, Aurora in the menus.
pub(super) fn menus(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id, chosen: &mut Option<Command>) {
    let ctx = ui.ctx().clone();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(aurora_i18n::tr!("script-menu-file"), |ui| {
            super::focus::claim(&ctx, ui.layer_id(), id);
            for (command, icon, label) in [
                (Command::Save, "device-floppy-filled", aurora_i18n::tr!("common-save")),
                (Command::Revert, "restore", aurora_i18n::tr!("script-menu-revert")),
                (Command::Reload, "refresh", aurora_i18n::tr!("build-ui-contents-refresh")),
                (Command::Open, "folder-open-filled", aurora_i18n::tr!("script-menu-open")),
                (Command::Export, "download-filled", aurora_i18n::tr!("script-menu-export")),
                (Command::External, "external-link-filled", aurora_i18n::tr!("script-menu-external")),
                (Command::Preferences, "settings-filled", aurora_i18n::tr!("script-menu-preferences")),
            ] {
                if menu::item_if(ui, p, icon, label, enabled(command, e, &ctx, id)) {
                    *chosen = Some(command);
                }
            }
        });
        ui.menu_button(aurora_i18n::tr!("script-menu-edit"), |ui| {
            edit_menu(ui, p, e, id, chosen);
        });
        ui.add_enabled_ui(
            e.loaded && e.can_edit && e.saving.is_none() && !e.tools.processed && !e.colors.active(),
            |ui| {
                let response = ui.menu_button(aurora_i18n::tr!("script-menu-insert"), |ui| {
                    insertion_menu(ui, p, e, id);
                });
                // the demo presses this button and types into its filter
                e.tools.insert_button = Some(response.response.rect);
                e.tools.insertion_open = response.inner.is_some();
            },
        );
        ui.menu_button(aurora_i18n::tr!("script-menu-help"), |ui| {
            super::focus::claim(&ctx, ui.layer_id(), id);
            for (command, icon, label) in [
                (Command::Help, "help-circle-filled", aurora_i18n::tr!("script-menu-help")),
                (Command::Keyword, "book-filled", aurora_i18n::tr!("script-help-keyword")),
                (Command::Library, "library-filled", aurora_i18n::tr!("script-help-library")),
                (Command::Debug, "bug-filled", aurora_i18n::tr!("script-debug-title")),
                (Command::Shortcuts, "keyboard-filled", aurora_i18n::tr!("script-help-shortcuts")),
            ] {
                if menu::item_if(ui, p, icon, label, enabled(command, e, &ctx, id)) {
                    *chosen = Some(command);
                }
            }
        });
    });
}

/// The Insert menu: a filter and the LSL / Lua functions it leaves, clicking
/// one writes its name at the cursor (LLScriptEdCore's insert list).
fn insertion_menu(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id) {
    super::focus::claim(ui.ctx(), ui.layer_id(), id);
    let field = crate::ui::widgets::search_text_edit(ui, p, &mut e.tools.insert_filter, |edit| {
        edit.id(id.with("insert_filter"))
            .desired_width(260.0)
            .hint_text(aurora_i18n::tr!("script-insert-filter"))
    });
    if !e.tools.insertion_open {
        field.request_focus();
    }
    crate::ui::widgets::divider(ui, p);
    // Only the results scroll, keeping the field in view.
    let mut results = egui::ScrollArea::vertical().id_salt((id, "insert_results")).max_height(220.0);
    if field.changed() {
        results = results.vertical_scroll_offset(0.0);
    }
    let lua = e.lua();
    let mut chosen = None;
    let filter = e.tools.insert_filter.trim().to_lowercase();
    results.show(ui, |ui| {
        let mut found = false;
        for entry in reference::all(lua)
            .iter()
            .filter(|entry| entry.group == "functions" && entry.name.to_lowercase().contains(&filter))
        {
            found = true;
            if ui.selectable_label(false, &entry.name).on_hover_text(&entry.signature).clicked() {
                chosen = Some(entry.name.clone());
                ui.close();
            }
        }
        if !found {
            ui.label(RichText::new(aurora_i18n::tr!("script-insert-empty")).color(p.muted));
        }
    });
    if let Some(name) = chosen {
        let [a, b] = editing::cursor(ui.ctx(), id).sorted_cursors();
        if editing::replace(ui.ctx(), id, &mut e.source, a.index.0..b.index.0, &name) {
            ui.ctx().memory_mut(|m| m.request_focus(id));
            e.tools.scroll_to_cursor = true;
            e.tools.help.visit(&name);
        } else {
            e.status = aurora_i18n::tr!("script-source-too-large").into();
        }
    }
}

/// The toolbar of the editor: Save, Open, the clipboard, Undo / Redo, Find,
/// its preferences, and the state of the script. A window of the linkset's
/// scripts draws it once, under the menus, for the script in front.
/// The width the experience selector takes at the right of the toolbar.
const EXPERIENCE_WIDTH: f32 = 180.0;
/// The room of the compile target's combo, at the left of the experience, and
/// of the script's own commands when a window of its own keeps them in the
/// toolbar.
const COMPILE_WIDTH: f32 = 136.0;
pub(super) const RUN_WIDTH: f32 = 112.0;

pub(super) fn toolbar(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
    with_search: bool,
    save_all: Option<bool>,
) {
    let ctx = ui.ctx().clone();
    let mut chosen = None;
    let row = ui.horizontal(|ui| {
        // the same height whether the script's own commands fit at the right
        // or not: the source below must not move with them
        ui.set_min_height(26.0);
        let left = ui.cursor().left();
        // Save sits above the source, with the script's other commands
        // (`run_buttons`), not among the tools here
        let mut buttons = Vec::new();
        if let Some(all) = save_all {
            buttons.push((Command::SaveAll, "files-filled", aurora_i18n::tr!("script-save-all")));
            let _ = all;
        }
        buttons.extend([
            (Command::Export, "download-filled", aurora_i18n::tr!("script-menu-export")),
            (Command::Open, "folder-open-filled", aurora_i18n::tr!("script-menu-open")),
            (Command::External, "external-link-filled", aurora_i18n::tr!("script-menu-external")),
            (Command::Cut, "cut", aurora_i18n::tr!("script-edit-cut")),
            (Command::Copy, "copy-filled", aurora_i18n::tr!("script-edit-copy")),
            (Command::Paste, "clipboard-text-filled", aurora_i18n::tr!("script-edit-paste")),
            (Command::Undo, "arrow-back-up", aurora_i18n::tr!("script-edit-undo")),
            (Command::Redo, "arrow-forward-up", aurora_i18n::tr!("script-edit-redo")),
            (Command::Find, "search-filled", aurora_i18n::tr!("script-search-title")),
            (Command::Preferences, "settings-filled", aurora_i18n::tr!("script-menu-preferences")),
        ]);
        for (command, icon, label) in buttons {
            let enabled = if command == Command::SaveAll {
                save_all.unwrap_or(false)
            } else {
                enabled(command, e, &ctx, id)
            };
            if icon_button(ui, p, icon, label, enabled) {
                chosen = Some(command);
            }
        }
        if e.saving.is_some() {
            ui.spinner();
        } else {
            crate::ui::widgets::status_dot(
                ui,
                if e.compile_failed {
                    p.danger
                } else if e.running == Some(true) {
                    p.success
                } else {
                    p.amber
                },
            );
        }
        e.tools.header_width = ui.cursor().left() - left;
        // At the right of this row: the Find controls when they are inline (in
        // the window of a linkset's scripts; a window of its own puts them
        // beside the body's header), then the experience of the script.
        // At the right of this row, right to left: the script's own commands
        // when it is a window of its own, the experience of the script, the
        // compile target, and the Find controls when they are inline.
        let own = save_all.is_none();
        let experience = (!e.target.object.is_nil()).then_some(EXPERIENCE_WIDTH);
        let find = with_search
            .then(|| Search::inline_width(ui))
            .filter(|_| settings.search_inline && prefs.search.owns(id) && !ui.is_sizing_pass());
        let run = if own { RUN_WIDTH } else { 0.0 };
        let right = experience.unwrap_or(0.0) + COMPILE_WIDTH + run + find.unwrap_or(0.0);
        if ui.available_width() >= right {
            ui.add_space(ui.available_width() - right);
            ui.allocate_ui_with_layout(Vec2::new(right, 24.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if own {
                    run_buttons(ui, p, e, actions, 24.0);
                }
                if let Some(width) = experience {
                    ui.allocate_ui_with_layout(Vec2::new(width, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let editable = e.can_edit && e.loaded && e.saving.is_none();
                        if e.experience.combo(ui, p, editable)
                            && let Some(request) = e.experience.begin()
                        {
                            actions.push(Action::Experiences { request, target: e.target });
                        }
                    });
                }
                // the compile target, at the left of the experience
                ui.add_enabled_ui(
                    e.can_edit && e.loaded && e.saving.is_none() && (e.target.object.is_nil() || e.running.is_some()),
                    |ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(COMPILE_WIDTH, 24.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| compile_target(ui, e),
                        );
                    },
                );
                if let Some(width) = find {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), egui::Sense::hover());
                    e.tools.search_slot = Some(
                        ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt("inline_search")
                                .max_rect(rect)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        ),
                    );
                }
            });
        }
    });
    let _ = row;
    if let Some(command) = chosen {
        run(command, e, &ctx, id, settings, prefs, actions);
    }
    if ctx.memory(|m| m.has_focus(visible_id(id, e.tools.processed))) {
        // F2 and Shift+F2 walk the reader's bookmarks; Ctrl+F2 marks the
        // caret's line (or unmarks it)
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F2)) {
            if !e.bookmarks.remove(&e.tools.position.0) {
                e.bookmarks.insert(e.tools.position.0);
            }
        } else {
            let step = if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F2)) {
                Some(false)
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
                Some(true)
            } else {
                None
            };
            if let Some(next) = step
                && let Some(line) = bookmark_step(&e.bookmarks, e.tools.position.0, next)
            {
                go_to_line(&ctx, e, id, line, 0);
            }
        }
        // Ctrl+Shift+S saves the window's scripts (Save All, as its button)
        if save_all == Some(true) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::S)) {
            run(Command::SaveAll, e, &ctx, id, settings, prefs, actions);
        }
        for (key, command) in [
            (egui::Key::S, Command::Save),
            (egui::Key::G, Command::Goto),
            (egui::Key::O, Command::Open),
        ] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                run(command, e, &ctx, id, settings, prefs, actions);
            }
        }
    }
}

fn header(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
    own_menus: bool,
) {
    let ctx = ui.ctx().clone();
    let mut chosen = None;
    if own_menus {
        menus(ui, p, e, id, &mut chosen);
    }
    if let Some(command) = chosen {
        run(command, e, &ctx, id, settings, prefs, actions);
    }
    toolbar(ui, p, e, id, settings, prefs, actions, false, None);
}

/// The compile target picker (panel_script_ed.xml `compile_target`, in its
/// order). The Lua targets need a region with SLua, as
/// LLScriptEdCore::postBuild enables them from LuaScriptsEnabled.
/// Which of a script's commands its window offers: Save when the source has
/// something to save, Reset when the script may be reset (`LLLiveLSLEditor::
/// onReset`, indra/newview/llpreviewscript.cpp: neither access to the source
/// nor a running reply is needed), and the running state when it has one to
/// flip -- a window of the inventory has neither Reset nor a running state.
fn run_state(e: &Editor) -> (bool, bool, Option<bool>) {
    let object = !e.target.object.is_nil();
    let reset = object && e.can_reset && e.saving.is_none();
    let running = (object && e.can_edit && e.running.is_some() && e.saving.is_none()).then(|| e.running.unwrap_or(false));
    (e.can_save(), reset, running)
}

/// The commands of the script, as icons above the source: save at the right,
/// then reset, then pause or resume, which says what a click does.
pub(super) fn run_buttons(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, actions: &mut Vec<Action>, side: f32) {
    let (save, reset, running) = run_state(e);
    if crate::ui::widgets::icon_button_sized(ui, p, "device-floppy-filled", aurora_i18n::tr!("common-save"), save, side) {
        actions.extend(e.save());
    }
    if reset && crate::ui::widgets::icon_button_sized(ui, p, "reload", aurora_i18n::tr!("script-reset"), true, side) {
        actions.push(Action::Reset(e.target));
    }
    if let Some(running) = running {
        // the icon says what pressing it does
        let (icon, label) = if running {
            ("player-pause-filled", aurora_i18n::tr!("script-pause"))
        } else {
            ("player-play-filled", aurora_i18n::tr!("script-resume"))
        };
        if crate::ui::widgets::icon_button_sized(ui, p, icon, label, true, side) {
            actions.push(Action::Running {
                target: e.target,
                running: !running,
            });
        }
    }
}

fn compile_target(ui: &mut egui::Ui, e: &mut Editor) {
    use aurora_net::scripts::CompileTarget;
    let label = |t: CompileTarget| match t {
        CompileTarget::Mono => aurora_i18n::tr!("script-target-mono"),
        CompileTarget::Lsl2 => aurora_i18n::tr!("script-target-lsl2"),
        CompileTarget::LslLuau => aurora_i18n::tr!("script-target-lsl-luau"),
        CompileTarget::Luau => aurora_i18n::tr!("script-target-luau"),
    };
    egui::ComboBox::from_id_salt(("script_compile_target", e.target.object, e.target.item))
        .selected_text(label(e.compile))
        .show_ui(ui, |ui| {
            for t in [
                CompileTarget::Mono,
                CompileTarget::Lsl2,
                CompileTarget::LslLuau,
                CompileTarget::Luau,
            ] {
                let lua = matches!(t, CompileTarget::LslLuau | CompileTarget::Luau);
                ui.add_enabled_ui(!lua || e.lua_scripts, |ui| ui.selectable_value(&mut e.compile, t, label(t)));
            }
        })
        .response
        .on_hover_text(aurora_i18n::tr!("script-target-hint"));
}

fn footer(ui: &mut egui::Ui, p: &Palette, e: &mut Editor) {
    // The reason a script failed, when the notice over the source does not
    // say it already (it does unless the reader has put it away).
    let covered = notice(e).is_some_and(|kind| e.tools.notice_hidden != Some(kind));
    if !e.status.is_empty() && !covered {
        ui.label(RichText::new(&e.status).color(if e.compile_failed { p.danger } else { p.muted }));
    }
    // The status bar under the source: where the cursor is, and what the script
    // weighs. Its commands (save, reset, running, the compile target) are the
    // icons above the source, beside its tabs.
    let bar = egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), ui.spacing().interact_size.y.max(26.0)),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let position = aurora_i18n::tr!(
                        "script-position",
                        line = (e.tools.position.0 + e.first_line()) as i64,
                        column = (e.tools.position.1 + e.first_line()) as i64
                    );
                    let spaces = aurora_i18n::tr!("script-spaces", count = indent::TAB_SIZE as i64);
                    let size = aurora_i18n::tr!(
                        "script-source-size",
                        used = size_label(e.source.len()),
                        limit = size_label(aurora_net::scripts::MAX_SOURCE)
                    );
                    let readout = format!("{position}  ·  {spaces}  ·  {size}");
                    ui.add(egui::Label::new(RichText::new(readout).small().color(p.muted)).truncate());
                },
            );
        });
    // the hairline that sets the bar apart from the source above it
    ui.painter().hline(
        bar.response.rect.x_range(),
        bar.response.rect.top(),
        egui::Stroke::new(1.0, p.muted_dim.gamma_multiply(0.7)),
    );
}

/// The width of the overview strip at the right of the source.
const OVERVIEW_WIDTH: f32 = 9.0;

/// The row the sticky state/event line takes above the source.
const STICKY_HEIGHT: f32 = 18.0;

/// The row the path of the caret takes under the source.
const CRUMBS_HEIGHT: f32 = 22.0;

/// The overview strip at the right of the source: the whole script in the
/// height of the window, with the part on screen (a thumb), the lines whose
/// text differs from the saved script (amber) and the diagnostics (the danger
/// colour). A click puts the caret on the line there, as an IDE's overview
/// ruler does. Painted over the scroll area's clip, at screen coordinates: it
/// stays put while the text scrolls.
fn overview(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    strip: egui::Rect,
    viewport: egui::Rect,
    rows: &[egui::epaint::text::PlacedRow],
    problems: &[Problem],
) {
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    let content = output_height(rows, text);
    let at = |y: f32| strip.top() + (y / content).clamp(0.0, 1.0) * strip.height();
    let painter = ui.painter().with_clip_rect(strip);
    // the part of the script the window shows, when it does not show it all
    if viewport.height() < content {
        let top = at(viewport.min.y.max(0.0));
        let bottom = at(viewport.max.y.min(content));
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(strip.left(), top), egui::pos2(strip.right(), bottom.max(top + 2.0))),
            ui.visuals().window_corner_radius,
            p.raised.gamma_multiply(0.35),
        );
    }
    // the changed lines and the diagnostics, as thin marks at the right edge
    let changed = if e.loaded && !e.tools.processed {
        changed_lines(text, &e.baseline)
    } else {
        None
    };
    let mut line = e.first_line();
    let mut line_start = true;
    for row in rows {
        if line_start {
            let index = line.saturating_sub(e.first_line());
            let y = at(row.pos.y);
            let tick = egui::Rect::from_min_max(egui::pos2(strip.right() - 4.0, y), egui::pos2(strip.right() - 1.0, y + 2.0));
            if let Some((first, last)) = changed
                && (first..=last).contains(&index)
            {
                painter.rect_filled(tick, 0.0, p.amber);
            }
            if problems.iter().any(|problem| problem.line == line && problem.source == e.name) {
                painter.rect_filled(tick.expand2(egui::Vec2::new(1.0, 0.0)), 0.0, p.danger);
            }
            if e.bookmarks.contains(&index) {
                painter.rect_filled(tick.expand2(egui::Vec2::new(1.0, 0.0)), 0.0, p.indigo_light);
            }
        }
        if row.ends_with_newline {
            line += 1;
        }
        line_start = row.ends_with_newline;
    }
    let response = ui.interact(strip, egui::Id::unique((id, "overview")), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let y = ((pos.y - strip.top()) / strip.height()).clamp(0.0, 1.0) * content;
        let mut target = None;
        let mut line = e.first_line();
        for row in rows {
            if (row.pos.y..row.pos.y + row.height()).contains(&y) {
                target = Some(line.saturating_sub(e.first_line()));
                break;
            }
            if row.ends_with_newline {
                line += 1;
            }
        }
        let target = target.unwrap_or_else(|| text.lines().count().saturating_sub(1));
        let at = editing::line_cursor(text, target, 0);
        editing::select(ui.ctx(), id, at..at, true);
        e.tools.scroll_to_cursor = true;
    }
    e.tools.overview = Some(strip);
}

/// The minimap: the script in a column at the right of the source, a bar per
/// line from its indentation to its last character, in the colour of what the
/// line starts with (Aurora's own; the thin overview strip of [`overview`]
/// stands here when the setting is off). The part of the script on screen is
/// framed, the changed lines, the diagnostics and the bookmarks hold the right
/// edge, a click puts the caret on the line there and a drag moves the view
/// alone.
#[allow(clippy::too_many_arguments)]
fn minimap(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id, settings: &ScriptSettings, strip: egui::Rect, problems: &[Problem]) {
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    // the map is made again only when the script changes
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    };
    if e.tools.minimap.as_ref().is_none_or(|(known, _)| *known != hash) {
        e.tools.minimap = Some((hash, super::minimap::map(text, e.lua())));
    }
    let Some((_, map)) = &e.tools.minimap else {
        return;
    };
    let total = map.total.max(1);
    // a line is a few points tall: a short script does not stretch to the
    // window, and a long one keeps a point per line
    let step = (strip.height() / total as f32).clamp(1.0, 3.0);
    // the marks keep the right edge, the bars the rest
    let band = 4.0;
    let bars = egui::Rect::from_min_max(strip.min, egui::pos2(strip.right() - band, strip.bottom()));
    let scale = bars.width() / super::minimap::COLUMNS;
    let painter = ui.painter().with_clip_rect(strip);
    // the part of the script on screen, behind the bars
    let first = e.tools.first_visible.min(total - 1);
    let last = e.tools.last_visible.max(first).min(total - 1);
    if last.saturating_sub(first) + 1 < total {
        let top = strip.top() + first as f32 * step;
        let bottom = strip.top() + (last + 1) as f32 * step;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(strip.left(), top), egui::pos2(strip.right(), bottom.max(top + 2.0))),
            ui.visuals().window_corner_radius,
            p.raised.gamma_multiply(0.35),
        );
    }
    let changed = if e.loaded && !e.tools.processed {
        changed_lines(text, &e.baseline)
    } else {
        None
    };
    for line in &map.lines {
        let y = strip.top() + line.number as f32 * step;
        if let Some((from, to)) = line.bar {
            let x0 = bars.left() + from * scale;
            let x1 = (bars.left() + to * scale).min(bars.right()).max(x0 + 1.0);
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, y), egui::pos2(x1, y + step)),
                0.0,
                settings.color(line.role, p).gamma_multiply(0.92),
            );
        }
        let tick = egui::Rect::from_min_max(egui::pos2(bars.right() + 1.0, y), egui::pos2(strip.right(), y + step.max(2.0)));
        if let Some((first, last)) = changed
            && (first..=last).contains(&line.number)
        {
            painter.rect_filled(tick, 0.0, p.amber);
        }
        if problems
            .iter()
            .any(|problem| problem.line == line.number && problem.source == e.name)
        {
            painter.rect_filled(tick, 0.0, p.danger);
        }
        if e.bookmarks.contains(&line.number) {
            painter.rect_filled(tick, 0.0, p.indigo_light);
        }
    }
    let response = ui.interact(strip, egui::Id::unique((id, "minimap")), egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    e.tools.overview = Some(strip);
    // a click or a drag through the map takes the caret with it, and the source
    // scrolls to it at its next show
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let line = (((pos.y - strip.top()) / step).max(0.0) as usize).min(total - 1);
        let at = editing::line_cursor(text, line, 0);
        editing::select(ui.ctx(), id, at..at, true);
        e.tools.scroll_to_cursor = true;
    }
}

/// The height the overview strip maps: the text's rows, or the text's lines
/// when the galley has not laid them out yet.
fn output_height(rows: &[egui::epaint::text::PlacedRow], text: &str) -> f32 {
    rows.last()
        .map(|row| row.pos.y + row.height())
        .unwrap_or_else(|| text.lines().count().max(1) as f32)
        .max(1.0)
}

/// F8 (`Some(true)`) or Shift+F8 (`Some(false)`), for the diagnostics in walk:
/// the keys belong to the script window while its source has the keyboard.
pub(super) fn problem_key(ctx: &egui::Context) -> Option<bool> {
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::SHIFT, egui::Key::F8) {
            Some(false)
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F8) {
            Some(true)
        } else {
            None
        }
    })
}

/// The row F8 or Shift+F8 goes to, from the caret: the first diagnostic of the
/// dock's order after it (or the last one when it is past them), the last one
/// before it (or the first one when it is before them). `rows` holds each
/// row's script and line, `at` the caret's.
pub(super) fn problem_row(rows: &[(usize, usize)], at: (usize, usize), next: bool) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    if next {
        rows.iter().position(|row| *row > at).or(Some(0))
    } else {
        rows.iter().rposition(|row| *row < at).or(Some(rows.len() - 1))
    }
}

/// Puts the cursor on a diagnostic's line and column, in the processed source
/// when the diagnostics shown are its own (LLScriptEdCore::onErrorList).
pub(super) fn go_to_error(ctx: &egui::Context, e: &mut Editor, id: Id, line: usize, column: usize) {
    if line == 0 {
        return;
    }
    e.tools.processed = e.tools.diagnostics_processed && !e.tools.processed_source.is_empty();
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    // LLScriptEdCore::onErrorList: a Lua error's line counts from 1
    let at = editing::line_cursor(text, line.saturating_sub(e.first_line()), column);
    editing::select(ctx, visible_id(id, e.tools.processed), at..at, true);
    e.tools.scroll_to_cursor = true;
}

/// A byte count for the status line: "512 B", "1.8 KB", "1 MB".
fn size_label(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.0} MB", bytes as f32 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f32 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// The columns of indentation of a line: a tab reaches the next stop.
fn indent_columns(line: &str) -> usize {
    let mut columns = 0;
    for ch in line.chars() {
        match ch {
            ' ' => columns += 1,
            '\t' => columns += indent::TAB_SIZE - columns % indent::TAB_SIZE,
            _ => break,
        }
    }
    columns
}

/// The lines of `text` that differ from `baseline`, as an inclusive range:
/// the region the two differ in, without marking the lines they share around
/// it (a line inserted in the middle marks that region, not the rest).
fn changed_lines(text: &str, baseline: &str) -> Option<(usize, usize)> {
    if text == baseline {
        return None;
    }
    let edited: Vec<&str> = text.lines().collect();
    let saved: Vec<&str> = baseline.lines().collect();
    let mut first = 0;
    while first < edited.len() && first < saved.len() && edited[first] == saved[first] {
        first += 1;
    }
    let (mut last_edited, mut last_saved) = (edited.len(), saved.len());
    while last_edited > first && last_saved > first && edited[last_edited - 1] == saved[last_saved - 1] {
        last_edited -= 1;
        last_saved -= 1;
    }
    let last = if last_edited > first { last_edited - 1 } else { first }.min(edited.len().saturating_sub(1));
    Some((first, last))
}

fn edit_menu(ui: &mut egui::Ui, p: &Palette, e: &Editor, id: Id, chosen: &mut Option<Command>) {
    super::focus::claim(ui.ctx(), ui.layer_id(), id);
    for (command, icon, label) in [
        (Command::Undo, "arrow-back-up", aurora_i18n::tr!("script-edit-undo")),
        (Command::Redo, "arrow-forward-up", aurora_i18n::tr!("script-edit-redo")),
        (Command::Cut, "cut", aurora_i18n::tr!("script-edit-cut")),
        (Command::Copy, "copy-filled", aurora_i18n::tr!("script-edit-copy")),
        (Command::Paste, "clipboard-text-filled", aurora_i18n::tr!("script-edit-paste")),
        (Command::SelectAll, "select-all", aurora_i18n::tr!("script-edit-select-all")),
        (Command::Deselect, "deselect", aurora_i18n::tr!("script-edit-deselect")),
        (Command::Find, "search-filled", aurora_i18n::tr!("script-search-title")),
        (Command::Goto, "arrow-bar-to-down", aurora_i18n::tr!("script-goto")),
        (Command::LineComment, "slashes", aurora_i18n::tr!("script-edit-line-comment")),
        (
            Command::BlockComment,
            "code-asterisk",
            aurora_i18n::tr!("script-edit-block-comment"),
        ),
        (Command::Indent, "indent-increase", aurora_i18n::tr!("script-edit-indent")),
        (Command::Outdent, "indent-decrease", aurora_i18n::tr!("script-edit-outdent")),
        (Command::FoldAll, "caret-right-filled", aurora_i18n::tr!("script-fold-all")),
        (Command::ExpandAll, "caret-down-filled", aurora_i18n::tr!("script-expand-all")),
    ] {
        if menu::item_if(ui, p, icon, label, enabled(command, e, ui.ctx(), id)) {
            *chosen = Some(command);
        }
    }
}

fn source(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    height: f32,
    search: &Search,
    problems: &[Problem],
) {
    if std::mem::take(&mut e.tools.reset_history) {
        egui::text_edit::TextEditState::default().store(ui.ctx(), id);
        egui::text_edit::TextEditState::default().store(ui.ctx(), visible_id(id, true));
        e.tools.can_undo = false;
        e.tools.can_redo = false;
        e.tools.history_cursor = None;
    }
    let base_id = id;
    let id = visible_id(id, e.tools.processed);
    // A new floater first lays out an invisible, disabled sizing pass.
    // Keep the request until the source can actually take the keyboard.
    if e.loaded && !ui.is_sizing_pass() && ui.is_enabled() && std::mem::take(&mut e.tools.focus_start) {
        editing::select(ui.ctx(), id, 0..0, true);
        e.tools.scroll_to_cursor = true;
    }
    e.folding
        .sync(if e.tools.processed { &e.tools.processed_source } else { &e.source });
    e.folding.reveal(editing::cursor(ui.ctx(), id).primary);
    // The sticky line above the text: the state and the event or function the
    // top of the source is in, from the outline (an IDE's sticky scroll). It
    // takes a row of its own, so the text starts under it; the line is the
    // last frame's, one frame behind the scroll.
    let text_now = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    // (a window too short for its row keeps its whole source instead)
    let sticky = if e.loaded && height > STICKY_HEIGHT + SOURCE_MIN {
        super::outline::enclosing(&super::outline::outline(text_now, e.lua()), e.tools.first_visible).map(|(state, inner)| match state {
            Some(state) => format!("{} › {}", state.name, inner.name),
            None => inner.name.clone(),
        })
    } else {
        None
    };
    let mut drop = 0.0;
    if let Some(label) = &sticky {
        // on the editor's own background, as the source below it
        let band = egui::Frame::new()
            .fill(settings.color(ColorRole::Background, p))
            .inner_margin(egui::Margin {
                left: 6,
                right: 4,
                top: 2,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(label).size(11.0).color(p.muted)).truncate());
            });
        // its row and the spacing under it come out of the source's room
        drop = band.response.rect.height() + ui.spacing().item_spacing.y;
    }
    let height = (height - drop).max(60.0);
    egui::Frame::new()
        .fill(settings.color(ColorRole::Background, p))
        .inner_margin(4.0)
        .show(ui, |ui| {
            ui.scope(|ui| {
                // the pane's own edge: in a split, a column's clip is the
                // window's, and the rail would land on the right pane's
                let pane_right = ui.max_rect().right();
                ui.visuals_mut().selection.bg_fill = settings.color(ColorRole::Selection, p);
                ui.visuals_mut().text_cursor.stroke.color = settings.color(ColorRole::Cursor, p);
                let font = egui::FontId::monospace(settings.font_size.clamp(8.0, 32.0));
                let query = if search.owns(base_id) {
                    search.query.clone()
                } else {
                    String::new()
                };
                let insensitive = search.insensitive;
                let whole_words = search.whole_words;
                let color_editable = e.loaded && e.can_edit && e.saving.is_none() && e.loading.is_none() && !e.tools.processed;
                let editable = color_editable && !e.colors.active();
                // the completion list's keys, taken before the text sees them,
                // then a snippet's, which only walk while it is being filled
                let nav = if editable && !ui.is_sizing_pass() {
                    complete::keys(ui, id)
                } else {
                    None
                };
                let step = if editable && nav.is_none() && e.tools.snippet.is_some() && !ui.is_sizing_pass() {
                    complete::snippet_keys(ui)
                } else {
                    None
                };
                let mut scroll = egui::ScrollArea::new([!settings.word_wrap, true]);
                if settings.word_wrap {
                    scroll = scroll.horizontal_scroll_offset(0.0);
                }
                scroll
                    .id_salt((id, "source"))
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show_viewport(ui, |ui, viewport| {
                        ui.horizontal_top(|ui| {
                            let (gutter, _) = ui.allocate_exact_size(Vec2::new(56.0, 1.0), egui::Sense::hover());
                            let interactive = e.loaded && (e.can_read || !e.source.is_empty());
                            let lua = e.lua();
                            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                                let mut job =
                                    e.highlighter
                                        .custom(text.as_str(), p, font.clone(), settings, &query, insensitive, whole_words, lua);
                                // panel_script_ed.xml enables wrapping; only the visual rows change.
                                job.wrap.max_width = if settings.word_wrap { wrap } else { f32::INFINITY };
                                e.folding.sync(text.as_str());
                                e.folding.layout(&mut job);
                                ui.fonts_mut(|f| f.layout_job(job))
                            };
                            let mut show = |ui: &mut egui::Ui, buffer: &mut dyn egui::TextBuffer| {
                                // a slot under the text for the caret's row and
                                // the indent guides
                                let beneath = ui.painter().add(egui::Shape::Noop);
                                // the rail (the minimap, or the thin overview
                                // strip) and the scroll bar keep their room at
                                // the right of the text
                                let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
                                let rail = if settings.minimap { super::minimap::WIDTH } else { OVERVIEW_WIDTH };
                                let width = (ui.available_width() - rail - bar).max(0.0);
                                let output = egui::TextEdit::multiline(buffer)
                                    .id(id)
                                    .code_editor()
                                    .layouter(&mut layouter)
                                    .desired_width(width)
                                    .desired_rows(1)
                                    .min_size(Vec2::new(0.0, viewport.height()))
                                    .frame(egui::Frame::NONE)
                                    .interactive(interactive)
                                    .show(ui);
                                (output, beneath)
                            };
                            let (mut output, beneath) = if editable {
                                indent::edit(ui, id, &mut e.source, show)
                            } else {
                                let mut text = if e.tools.processed {
                                    e.tools.processed_source.as_str()
                                } else {
                                    e.source.as_str()
                                };
                                show(ui, &mut text)
                            };
                            // the name being typed, completed under the caret,
                            // and the snippet's next placeholder
                            if editable && !ui.is_sizing_pass() {
                                if let Some(step) = step {
                                    complete::snippet_step(ui, id, e, step);
                                }
                                complete::after(ui, p, lua, id, e, &mut output, nav);
                            }
                            let mut chosen = None;
                            // the word the pointer is on, for the window's
                            // definition and references commands
                            let pointed = output
                                .response
                                .hover_pos()
                                .map(|pos| output.galley.cursor_from_pos(pos - output.galley_pos).index.0)
                                .map(|at| editing::word(e.source.as_str(), at, lua).to_owned())
                                .unwrap_or_default();
                            let (mut goto, mut references) = (None, None);
                            menu::context_menu(&output.response, p, |ui| {
                                edit_menu(ui, p, e, base_id, &mut chosen);
                                if !pointed.is_empty() && !e.linkset.is_nil() {
                                    menu::separator(ui, p);
                                    if menu::item(ui, p, "arrow-narrow-right", aurora_i18n::tr!("script-goto-definition")) {
                                        goto = Some(pointed.clone());
                                    }
                                    if menu::item(ui, p, "search-filled", aurora_i18n::tr!("script-find-references")) {
                                        references = Some(pointed.clone());
                                    }
                                }
                            });
                            if let Some(word) = goto {
                                e.tools.goto_word = Some(word);
                            }
                            if let Some(word) = references {
                                e.tools.references_word = Some(word);
                            }
                            if let Some(command) = chosen {
                                // Context commands use the same dispatcher as the menus.
                                ui.ctx().data_mut(|d| d.insert_temp(Id::unique((base_id, "command")), command));
                            }
                            let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
                            if ui
                                .ctx()
                                .data_mut(|d| d.remove_temp::<bool>(id.with("selection_error")))
                                .unwrap_or(false)
                            {
                                e.status = aurora_i18n::tr!("script-selection-edit-failed").into();
                            }
                            e.colors.draw(ui, p, text, &output, &e.highlighter.colors, color_editable);
                            // The script highlighter already paints every result a subtle amber.
                            // Make only the active Find result prominent, even while the Find
                            // floater owns the keyboard focus. Match notecard search styling.
                            if search.owns(base_id)
                                && let Some(range) = search.active_range(text)
                            {
                                let color = egui::Color32::from_rgba_unmultiplied(p.amber.r(), p.amber.g(), p.amber.b(), 145);
                                let mut shapes = Vec::new();
                                for rect in crate::ui::chat::range_rects(&output.galley, output.galley_pos, range) {
                                    if !rect.intersects(output.text_clip_rect) {
                                        continue;
                                    }
                                    let rect = rect.intersect(output.text_clip_rect);
                                    shapes.push(egui::Shape::rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, color));
                                    shapes.push(egui::Shape::rect_stroke(
                                        rect,
                                        ui.visuals().widgets.inactive.corner_radius,
                                        egui::Stroke::new(1.5, p.amber),
                                        egui::StrokeKind::Inside,
                                    ));
                                }
                                ui.painter().with_clip_rect(output.text_clip_rect).add(egui::Shape::Vec(shapes));
                            }
                            if editable
                                && (output.response.changed()
                                    || e.tools.history_cursor != output.cursor_range
                                    || ui
                                        .ctx()
                                        .data_mut(|d| d.remove_temp::<bool>(Id::unique((id, "history_dirty"))))
                                        .unwrap_or(false))
                            {
                                let range = output.cursor_range.unwrap_or_default();
                                let current = (range, e.source.clone());
                                let undo = output.state.undoer();
                                e.tools.can_undo = undo.has_undo(&current);
                                e.tools.can_redo = undo.has_redo(&current);
                                e.tools.history_cursor = output.cursor_range;
                            }
                            if let Some(range) = output.cursor_range {
                                e.tools.position = editing::position(text, range.primary.index.0);
                            }
                            if std::mem::take(&mut e.tools.scroll_to_cursor) {
                                let rect = output
                                    .galley
                                    .pos_from_cursor(editing::cursor(ui.ctx(), id).primary)
                                    .translate(output.galley_pos.to_vec2());
                                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                            }
                            // The other places the word under the caret appears,
                            // dimmed behind the text (the box of a reference),
                            // then the caret's row and the indentation guides.
                            let mut beneath_shapes = Vec::new();
                            if e.loaded && !ui.is_sizing_pass() {
                                let caret_at = editing::cursor(ui.ctx(), id).primary.index.0;
                                if let Some((mine, found)) = editing::occurrences(text, caret_at, lua) {
                                    for range in found {
                                        if range == mine {
                                            continue;
                                        }
                                        for rect in crate::ui::chat::range_rects(&output.galley, output.galley_pos, range) {
                                            if !rect.intersects(output.text_clip_rect) {
                                                continue;
                                            }
                                            beneath_shapes.push(egui::Shape::rect_filled(
                                                rect.intersect(output.text_clip_rect).expand2(Vec2::new(0.5, 0.0)),
                                                ui.visuals().widgets.inactive.corner_radius,
                                                p.raised.gamma_multiply(0.38),
                                            ));
                                        }
                                    }
                                }
                                let caret = output
                                    .galley
                                    .pos_from_cursor(editing::cursor(ui.ctx(), id).primary)
                                    .translate(output.galley_pos.to_vec2());
                                let right = output.text_clip_rect.right();
                                if caret.height() > 1.0 && caret.top() < output.text_clip_rect.bottom() {
                                    let band = egui::Rect::from_min_max(
                                        egui::pos2(output.galley_pos.x, caret.top()),
                                        egui::pos2(right, caret.bottom()),
                                    )
                                    .intersect(output.text_clip_rect);
                                    if band.height() > 1.0 {
                                        beneath_shapes.push(egui::Shape::rect_filled(band, 0.0, p.raised.gamma_multiply(0.30)));
                                    }
                                }
                                let space = ui.fonts_mut(|f| f.glyph_width(&font, ' '));
                                let lines: Vec<&str> = text.lines().collect();
                                let mut line = e.first_line();
                                let mut line_start = true;
                                for row in &output.galley.rows {
                                    let y = output.galley_pos.y + row.pos.y;
                                    if y > output.text_clip_rect.bottom() {
                                        break;
                                    }
                                    if line_start
                                        && row.height() >= 1.0
                                        && y + row.height() >= output.text_clip_rect.top()
                                        && let Some(prefix) = lines.get(line.saturating_sub(e.first_line()))
                                    {
                                        for level in 1..=indent_columns(prefix) / indent::TAB_SIZE {
                                            let x = output.galley_pos.x + (level * indent::TAB_SIZE) as f32 * space;
                                            if x >= output.text_clip_rect.right() - 2.0 {
                                                break;
                                            }
                                            beneath_shapes.push(egui::Shape::line_segment(
                                                [egui::pos2(x, y), egui::pos2(x, y + row.height())],
                                                egui::Stroke::new(1.0, p.muted_dim.gamma_multiply(0.6)),
                                            ));
                                        }
                                    }
                                    if row.ends_with_newline {
                                        line += 1;
                                    }
                                    line_start = row.ends_with_newline;
                                }
                                // the bracket at the caret and its match, lit
                                // behind the text
                                if let Some((first, second)) = editing::bracket_match(text, caret_at) {
                                    for index in [first, second] {
                                        let start = output
                                            .galley
                                            .pos_from_cursor(egui::text::CCursor::new(index))
                                            .translate(output.galley_pos.to_vec2());
                                        let end = output
                                            .galley
                                            .pos_from_cursor(egui::text::CCursor::new(index + 1))
                                            .translate(output.galley_pos.to_vec2());
                                        // a bracket split by the wrapping marks nothing
                                        if (start.top() - end.top()).abs() > 1.0 {
                                            continue;
                                        }
                                        let rect = egui::Rect::from_min_max(start.min, end.max);
                                        if rect.intersects(output.text_clip_rect) {
                                            beneath_shapes.push(egui::Shape::rect_filled(
                                                rect.expand2(Vec2::new(1.0, 0.0)),
                                                ui.visuals().widgets.inactive.corner_radius,
                                                p.raised.gamma_multiply(0.55),
                                            ));
                                        }
                                    }
                                }
                            }
                            ui.painter().set(beneath, egui::Shape::Vec(beneath_shapes));
                            let left = gutter.left() + viewport.min.x;
                            let clip = egui::Rect::from_min_max(
                                egui::pos2(left, ui.clip_rect().top()),
                                egui::pos2(left + 56.0, ui.clip_rect().bottom()),
                            );
                            let painter = ui.painter().with_clip_rect(clip);
                            painter.rect_filled(clip, 0.0, settings.color(ColorRole::Background, p));
                            // the lines whose text differs from the saved script
                            let changed = if e.loaded && !e.tools.processed {
                                changed_lines(text, &e.baseline)
                            } else {
                                None
                            };
                            let mut line = e.first_line();
                            let mut line_start = true;
                            for row in &output.galley.rows {
                                let y = output.galley_pos.y + row.pos.y;
                                if y > clip.bottom() {
                                    break;
                                }
                                if line_start && row.height() >= 1.0 && y + row.height() >= clip.top() {
                                    painter.text(
                                        egui::pos2(left + 50.0, y),
                                        egui::Align2::RIGHT_TOP,
                                        line.to_string(),
                                        font.clone(),
                                        if line == e.tools.position.0 + e.first_line() {
                                            settings.color(ColorRole::Cursor, p)
                                        } else {
                                            p.muted
                                        },
                                    );
                                    if let Some((first, last)) = changed
                                        && (first..=last).contains(&line.saturating_sub(e.first_line()))
                                    {
                                        painter.rect_filled(
                                            egui::Rect::from_min_max(
                                                egui::pos2(left + 52.0, y + 1.0),
                                                egui::pos2(left + 55.0, y + row.height() - 1.0),
                                            ),
                                            0.0,
                                            p.amber,
                                        );
                                    }
                                    // the lines a diagnostic is reported on (the
                                    // dock may list every script of the window)
                                    if problems.iter().any(|problem| problem.line == line && problem.source == e.name) {
                                        painter.circle_filled(egui::pos2(left + 4.0, y + row.height() / 2.0), 2.5, p.danger);
                                    }
                                    // a reader's bookmark: a mark in the gutter, and a
                                    // click on the line's place in it toggles it
                                    let index = line.saturating_sub(e.first_line());
                                    if e.bookmarks.contains(&index) {
                                        painter.rect_filled(
                                            egui::Rect::from_min_max(
                                                egui::pos2(left + 8.0, y + 3.0),
                                                egui::pos2(left + 12.0, y + row.height() - 3.0),
                                            ),
                                            0.0,
                                            p.indigo_light,
                                        );
                                    }
                                    let hit = ui.interact(
                                        egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(left + 16.0, y + row.height())),
                                        egui::Id::unique((id, "gutter", index)),
                                        egui::Sense::click(),
                                    );
                                    if hit.clicked() && !e.bookmarks.remove(&index) {
                                        e.bookmarks.insert(index);
                                    }
                                }
                                if row.ends_with_newline {
                                    line += 1;
                                }
                                line_start = row.ends_with_newline;
                            }
                            painter.line_segment(
                                [egui::pos2(left + 55.0, clip.top()), egui::pos2(left + 55.0, clip.bottom())],
                                egui::Stroke::new(1.0, p.muted_dim),
                            );
                            // The diagnostics of this script in the code: a
                            // squiggle under the span the compile points at
                            // (LSL gives a place, not a length), so that a row
                            // of the dock is visible where it matters.
                            let squiggle = ui.painter().with_clip_rect(output.text_clip_rect);
                            for problem in problems.iter().filter(|problem| problem.source == e.name) {
                                if problem.line == 0 {
                                    continue;
                                }
                                let line = problem.line.saturating_sub(e.first_line());
                                let span = editing::error_span(text, line, problem.column);
                                let warning = problem.message.to_ascii_uppercase().contains("WARNING");
                                let color = if warning { p.amber } else { p.danger };
                                for rect in crate::ui::chat::range_rects(&output.galley, output.galley_pos, span) {
                                    if !rect.intersects(output.text_clip_rect) {
                                        continue;
                                    }
                                    let y = rect.bottom() - 2.0;
                                    let mut points = Vec::new();
                                    let mut x = rect.left();
                                    let mut up = true;
                                    while x <= rect.right() {
                                        points.push(egui::pos2(x, if up { y } else { y - 1.8 }));
                                        x += 2.0;
                                        up = !up;
                                    }
                                    squiggle.add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
                                }
                            }
                            if e.folding.reveal(output.cursor_range.unwrap_or_default().primary) {
                                ui.ctx().request_repaint();
                            }
                            e.folding.draw(ui, p, id, &output, left);
                            // the help of the word under the pointer, over code
                            // only: a name in a comment or a string is not the
                            // keyword (LLScriptEdCore::updateDynamicHelp)
                            if let Some(pos) = output.response.hover_pos() {
                                let at = output.galley.cursor_from_pos(pos - output.galley_pos).index.0;
                                if let Some(entry) = reference::hover(text, at, lua) {
                                    (*output.response).clone().on_hover_ui(|ui| hover_docs(ui, p, entry));
                                }
                            }
                            // a click on the text brings this pane to the front
                            // (a split's other pane, from `split_body`)
                            e.tools.clicked = output.response.clicked() && !ui.ctx().input(|i| i.modifiers.command || i.modifiers.ctrl);
                            // Ctrl (Cmd) on a name and a click: the window takes
                            // the caret to where the name is declared
                            if output.response.clicked()
                                && ui.ctx().input(|i| i.modifiers.command || i.modifiers.ctrl)
                                && let Some(pos) = output.response.interact_pointer_pos()
                            {
                                let at = output.galley.cursor_from_pos(pos - output.galley_pos).index.0;
                                let word = editing::word(text, at, lua).to_owned();
                                if !word.is_empty() {
                                    e.tools.goto_word = Some(word);
                                }
                            }
                            // the line at the top of the viewport: the sticky
                            // line of the next frame names what holds it
                            let top = output.text_clip_rect.top();
                            let bottom = output.text_clip_rect.bottom();
                            let mut index = 0;
                            let mut first = None;
                            let mut last = 0;
                            for row in &output.galley.rows {
                                let y = output.galley_pos.y + row.pos.y;
                                if first.is_none() && y + row.height() > top {
                                    first = Some(index);
                                }
                                if y < bottom {
                                    last = index;
                                }
                                if row.ends_with_newline {
                                    index += 1;
                                }
                            }
                            e.tools.first_visible = first.unwrap_or(0);
                            e.tools.last_visible = last;
                            // The overview strip at the right of the source: the
                            // whole script in the height of the window, with
                            // what is visible, the changed lines and the
                            // diagnostics; a click goes there. Last, so that it
                            // paints over the text it maps.
                            e.tools.overview = None;
                            if e.loaded && !ui.is_sizing_pass() && !output.galley.rows.is_empty() {
                                let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
                                let right = pane_right - bar - 1.0;
                                let rail = if settings.minimap { super::minimap::WIDTH } else { OVERVIEW_WIDTH };
                                let strip = egui::Rect::from_min_max(
                                    egui::pos2(right - rail, ui.clip_rect().top() + 1.0),
                                    egui::pos2(right, ui.clip_rect().bottom() - 1.0),
                                );
                                if strip.width() > 1.0 && strip.height() > 1.0 {
                                    if settings.minimap {
                                        minimap(ui, p, e, id, settings, strip, problems);
                                    } else {
                                        overview(ui, p, e, id, strip, viewport, &output.galley.rows, problems);
                                    }
                                }
                            }
                        });
                    });
            });
        });
}

/// The bar under the source: the path of the caret -- the script, then the
/// text that opens each block it is in, outermost first -- each step a click
/// away from its line, and at the right what the script counts and whether it
/// can be changed. Alchemy's Script Studio puts the same bar under its editor
/// (ALScriptCrumbsBar), with a longer trailer.
#[allow(clippy::too_many_arguments)]
pub(super) fn crumbs_row(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    problems: &[Problem],
    tab: &mut DockTab,
    trailer: bool,
) {
    // the source or what the preprocessor made of it, when it runs: the choice
    // is the pane's, so it sits in the pane's own bar under the source
    let either = settings.preprocessor || e.tools.has_processed || e.tools.processed;
    if !either {
        e.tools.processed = false;
    }
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    let path = crumbs::path(text, e.lua(), e.tools.position.0);
    let name = e.name.clone();
    let read_only = e.loaded && !e.can_edit;
    let counted = problems.len();
    // what the call under the caret takes, at the right of the path: the
    // parameter being filled in ink (signature help), when the setting is on
    let signature = settings
        .signature_help
        .then(|| super::signature::at_caret(text, editing::cursor(ui.ctx(), id).primary.index.0, e.lua()))
        .flatten();
    let mut jump = None;
    // The bar of the path, under the source: a quiet face, and a chip per
    // block that holds the caret, the one it is in lit.
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CRUMBS_HEIGHT), egui::Sense::hover());
    ui.painter()
        .rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, p.bar.gamma_multiply(0.55));
    let inner = egui::Rect::from_min_max(bar.min + Vec2::new(8.0, 0.0), bar.max - Vec2::new(6.0, 0.0));
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    // the trailer: what the script counts, then whether it can be changed (the
    // pane in front alone says the count of the dock)
    if trailer && counted > 0 {
        let label = aurora_i18n::tr!("script-problems-count", count = counted as i64);
        if flat_button(&mut ui, p, &label).clicked() {
            *tab = DockTab::Problems;
        }
    }
    if either {
        // the two views, as words: the one shown in ink, the other dimmed (the
        // bar is too thin for the pills of a tab strip)
        let processed = e.tools.processed;
        if switch_word(&mut ui, p, aurora_i18n::tr!("script-tab-processed"), processed) {
            e.tools.processed = true;
        }
        if switch_word(&mut ui, p, aurora_i18n::tr!("script-tab-source"), !processed) {
            e.tools.processed = false;
        }
    }
    if let Some(signature) = &signature {
        ui.add(egui::Label::new(signature_job(p, signature)).truncate());
        ui.add_space(10.0);
    }
    if trailer
        && read_only
        && let Some(t) = crate::ui::icons::global("lock-filled")
    {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(14.0)).tint(p.warn))
            .on_hover_text(aurora_i18n::tr!("script-read-only"));
    }
    // the path, on the left, in the panes' order as the source is
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        Vec2::new(width, CRUMBS_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            crumb(ui, p, &name, false, false);
            for (index, step) in path.iter().enumerate() {
                if let Some(t) = crate::ui::icons::global("chevron-right") {
                    ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(10.0)).tint(p.muted_dim));
                }
                let last = index + 1 == path.len();
                if crumb(ui, p, &step.text, last, true).clicked() {
                    jump = Some(step.line);
                }
            }
        },
    );
    if let Some(line) = jump {
        go_to_line(ui.ctx(), e, id, line, 0);
    }
}

/// One word of the source / preprocessed switch: in ink when it is the view
/// shown, dimmed otherwise.
fn switch_word(ui: &mut egui::Ui, p: &Palette, text: &str, on: bool) -> bool {
    let color = if on { p.ink } else { p.muted };
    let response = ui.add(
        egui::Label::new(RichText::new(text).size(11.0).color(color))
            .truncate()
            .sense(egui::Sense::click()),
    );
    if on {
        return false;
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// One step of the crumbs: its words, a click away from its line.
/// The signature of the call the caret is in: its name, then its parameters,
/// the one being filled in ink and the others muted.
fn signature_job(p: &Palette, signature: &super::signature::Signature) -> egui::text::LayoutJob {
    let font = egui::FontId::proportional(11.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &format!("{}(", signature.name),
        0.0,
        egui::TextFormat::simple(font.clone(), p.muted),
    );
    for (index, param) in signature.params.iter().enumerate() {
        if index > 0 {
            job.append(", ", 0.0, egui::TextFormat::simple(font.clone(), p.muted));
        }
        let color = if index == signature.at { p.ink } else { p.muted };
        job.append(param, 0.0, egui::TextFormat::simple(font.clone(), color));
    }
    job.append(")", 0.0, egui::TextFormat::simple(font, p.muted));
    job
}

/// One word of the path: a chip, lit when it is the block the caret is in, and
/// lit again under the pointer when a click goes to it.
fn crumb(ui: &mut egui::Ui, p: &Palette, text: &str, current: bool, jumps: bool) -> egui::Response {
    let font = crate::ui::widgets::control_font(ui, 11.0);
    let room = (ui.available_width() - 160.0).max(48.0);
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, font);
    let pad = Vec2::new(6.0, 1.0);
    let (rect, response) = ui.allocate_exact_size(
        galley.size() + pad * 2.0,
        if jumps { egui::Sense::click() } else { egui::Sense::hover() },
    );
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if current {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
    } else if jumps && response.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.30));
    }
    let color = if current { p.ink } else { p.muted };
    ui.painter().galley(
        egui::pos2(rect.left() + pad.x, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if jumps {
        response
            .on_hover_text(aurora_i18n::tr!("script-crumbs-jump"))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The help of a word, over its name: the signature as the catalog writes it,
/// then what it does (LLScriptEdCore::updateDynamicHelp).
fn hover_docs(ui: &mut egui::Ui, p: &Palette, entry: &reference::Entry) {
    ui.set_max_width(360.0);
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.label(RichText::new(&entry.signature).font(egui::FontId::monospace(12.0)).color(p.ink));
    if !entry.tooltip.is_empty() {
        ui.add(egui::Label::new(RichText::new(&entry.tooltip).size(11.0).color(p.muted)).wrap());
    }
}

pub(super) fn dialogs(
    ctx: &egui::Context,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &mut ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
) {
    if let Some(command) = ctx.data_mut(|d| d.remove_temp::<Command>(Id::unique((id, "command")))) {
        run(command, e, ctx, id, settings, prefs, actions);
    }
    let original_id = id;
    let id = visible_id(id, e.tools.processed);
    e.tools.help.lua = e.lua();
    if e.tools.help.show(
        ctx,
        p,
        id,
        if e.tools.processed { &e.tools.processed_source } else { &e.source },
        settings,
    ) {
        actions.push(Action::SettingsChanged);
    }
    if e.tools.goto {
        let mut open = true;
        Floater::new(
            &format!("{id:?}_goto"),
            aurora_i18n::tr!("script-goto"),
            ctx.content_rect().center(),
            Vec2::new(285.0, 115.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.horizontal(|ui| {
                ui.label(if e.lua() {
                    aurora_i18n::tr!("script-goto-line-lua")
                } else {
                    aurora_i18n::tr!("script-goto-line")
                });
                let input = ui.add(crate::ui::widgets::restricted_number(
                    egui::DragValue::new(&mut e.tools.goto_line).range(0..=usize::MAX),
                    crate::ui::widgets::TextPolicy::Digits,
                ));
                if flat_button(ui, p, aurora_i18n::tr!("script-goto-go")).clicked()
                    || (input.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)))
                {
                    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
                    let at = editing::line_cursor(text, e.tools.goto_line.saturating_sub(e.first_line()), 0);
                    editing::select(ctx, id, at..at, true);
                    e.tools.goto = false;
                    e.tools.scroll_to_cursor = true;
                }
            });
        });
        e.tools.goto &= open;
    }
    if e.tools.library {
        let lua = e.lua();
        Floater::new(
            &format!("{id:?}_library"),
            aurora_i18n::tr!("script-help-library"),
            ctx.content_rect().center() - Vec2::new(280.0, 240.0),
            Vec2::new(560.0, 480.0),
        )
        .show(ctx, p, &mut e.tools.library, |ui| {
            crate::ui::widgets::search_text_edit(ui, p, &mut e.tools.insert_filter, |edit| edit);
            let filter = e.tools.insert_filter.to_lowercase();
            let mut keyword = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in reference::all(lua).iter().filter(|e| e.name.to_lowercase().contains(&filter)) {
                    if ui
                        .selectable_label(false, RichText::new(&entry.signature).monospace())
                        .on_hover_text(&entry.tooltip)
                        .clicked()
                    {
                        keyword = Some(entry.name.clone());
                    }
                }
            });
            if let Some(name) = keyword {
                e.tools.help.keyword = name;
                e.tools.help.open = true;
            }
        });
    }
    if e.tools.revert {
        let mut open = true;
        Floater::new(
            &format!("{id:?}_revert"),
            aurora_i18n::tr!("script-menu-revert"),
            ctx.content_rect().center(),
            Vec2::new(405.0, 130.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("script-revert-question"));
            ui.horizontal(|ui| {
                if flat_button(ui, p, aurora_i18n::tr!("script-menu-revert")).clicked() {
                    let end = e.source.chars().count();
                    editing::replace(ctx, original_id, &mut e.source, 0..end, &e.baseline);
                    e.compile = e.baseline_compile;
                    e.experience.selected = e.experience.baseline;
                    e.tools.revert = false;
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    e.tools.revert = false;
                }
            });
        });
        e.tools.revert &= open;
    }
    if e.tools.incoming.is_some() {
        let mut open = true;
        Floater::new(
            &format!("{id:?}_file_conflict"),
            aurora_i18n::tr!("script-menu-open"),
            ctx.content_rect().center(),
            Vec2::new(420.0, 150.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("script-file-conflict"));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled_ui(e.can_edit && e.saving.is_none(), |ui| {
                        flat_button(ui, p, aurora_i18n::tr!("script-file-use"))
                    })
                    .inner
                    .clicked()
                    && let Some(source) = e.tools.incoming.take()
                {
                    let end = e.source.chars().count();
                    editing::replace(ctx, original_id, &mut e.source, 0..end, &source);
                    e.tools.external_snapshot.clone_from(&e.source);
                    if e.tools.incoming_external {
                        actions.extend(e.save());
                    }
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    e.tools.incoming = None;
                }
            });
        });
        if !open {
            e.tools.incoming = None;
        }
    }
}

pub(super) fn search(
    ctx: &egui::Context,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    search: &mut Search,
    (parent_id, parent): (Id, egui::Rect),
    prefer_left: bool,
) {
    let id = visible_id(id, e.tools.processed);
    let cursor = editing::cursor(ctx, id);
    let editable = e.can_edit && e.loaded && e.saving.is_none() && e.loading.is_none() && !e.tools.processed && !e.colors.active();
    if let Some(mut ui) = e.tools.search_slot.take() {
        if search.focus {
            ctx.move_to_top(ui.layer_id());
        }
        search.controls(
            &mut ui,
            p,
            id,
            if e.tools.processed {
                &mut e.tools.processed_source
            } else {
                &mut e.source
            },
            editable,
            true,
        );
        e.tools.scroll_to_cursor |= cursor != editing::cursor(ctx, id);
        return;
    }
    let (pos, _overlap) = Search::placement(ctx, parent, e.tools.body_top, prefer_left);
    // Dependent Find follows its active script parent whether overlapping or
    // beside it. Do not force Find over unrelated windows when the editor is inactive.
    let owner = egui::LayerId::new(egui::Order::Middle, parent_id);
    let child = egui::LayerId::new(egui::Order::Middle, Id::unique(Search::WINDOW));
    ctx.set_sublayer(owner, child);
    // A keyboard-focused Find field can remain focused while another window
    // is explicitly raised. Only an actual interaction with this editor/find
    // pair (or the initial Find opening) should raise the pair again.
    let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    let pressed_here = pressed.is_some_and(|at| matches!(ctx.layer_id_at(at), Some(layer) if layer == owner || layer == child));
    if (search.focus && pressed.is_none()) || pressed_here || ctx.is_being_dragged(child.id.with("move")) {
        ctx.move_to_top(owner);
    }
    search.show(
        ctx,
        p,
        id,
        if e.tools.processed {
            &mut e.tools.processed_source
        } else {
            &mut e.source
        },
        editable,
        pos,
    );
    e.tools.scroll_to_cursor |= cursor != editing::cursor(ctx, id);
}

/// The file job of one document (opened, exported, sent to the external
/// editor): its result reaches the document even while its tab is not the one
/// in front.
pub(super) fn poll_files(ctx: &egui::Context, e: &mut Editor, id: Id, settings: &ScriptSettings, actions: &mut Vec<Action>) {
    if let Some(done) = e.tools.file_job.as_ref().and_then(|j| j.0.try_recv().ok()) {
        e.tools.file_job = None;
        e.tools.external_starting = false;
        match done {
            files::Done::Opened(source) if e.can_edit && e.loaded && e.saving.is_none() && e.source == e.tools.file_snapshot => {
                let end = e.source.chars().count();
                editing::replace(ctx, id, &mut e.source, 0..end, &source);
                e.status = aurora_i18n::tr!("script-file-opened").into();
            }
            files::Done::Opened(source) => {
                e.tools.incoming = Some(source);
                e.tools.incoming_external = false;
            }
            files::Done::Saved => e.status = aurora_i18n::tr!("script-file-saved").into(),
            files::Done::External(watch) => {
                e.tools.external = Some(watch);
                e.tools.external_snapshot.clone_from(&e.tools.file_snapshot);
                e.status = aurora_i18n::tr!("script-external-started").into();
            }
            files::Done::Failed(reason) => e.status = reason,
            files::Done::Cancelled => {}
        }
    }
    let mut latest = None;
    if let Some(watch) = e.tools.external.as_ref() {
        for source in watch.changes.try_iter() {
            latest = Some(source);
        }
    }
    if let Some(result) = latest {
        match result {
            Ok(source) if e.can_edit && e.loaded && e.saving.is_none() && e.source == e.tools.external_snapshot => {
                let end = e.source.chars().count();
                editing::replace(ctx, id, &mut e.source, 0..end, &source);
                e.tools.external_snapshot.clone_from(&e.source);
                if let Some(action) = e.save() {
                    actions.push(action);
                }
            }
            Ok(source) => {
                e.tools.incoming = Some(source);
                e.tools.incoming_external = true;
            }
            Err(reason) => e.status = reason,
        }
    }
    let _ = settings;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dock keeps the height its edge was dragged to, within its least and
    /// the room the window leaves once the text has its own; the default is a
    /// third of the window at most, and folding gives it its header alone.
    #[test]
    fn the_dock_keeps_the_height_its_edge_was_dragged_to() {
        // nothing dragged: the default, a third of the window at most
        assert_eq!(dock_room(0, 2000.0), DOCK_HEIGHT);
        assert_eq!(dock_room(0, 300.0), 90.0);
        // what the reader dragged is kept, up to three fifths of the window
        assert_eq!(dock_room(240, 2000.0), 240.0);
        assert_eq!(dock_room(400, 500.0), 300.0);
        // and never less than its header and a row
        assert_eq!(dock_room(10, 2000.0), DOCK_LEAST);
    }

    /// A tab dragged onto another takes its place: before it when it comes
    /// from its right, after it when it comes from its left.
    #[test]
    fn a_dragged_tab_takes_the_place_of_the_one_it_is_dropped_on() {
        use DockTab::{Outline, Problems, References, Search};
        let order = [Problems, Outline, Search, References];
        assert_eq!(
            reorder_tabs(order, Search, Outline),
            [Problems, Search, Outline, References],
            "from the right, before it"
        );
        assert_eq!(
            reorder_tabs(order, Outline, References),
            [Problems, Search, References, Outline],
            "from the left, after it"
        );
        assert_eq!(reorder_tabs(order, Problems, Problems), order, "dropped on itself");
    }

    #[test]
    fn the_changed_lines_are_the_region_between_the_saved_and_the_edited_text() {
        // no change: no bars
        assert_eq!(changed_lines("a\nb\n", "a\nb\n"), None);
        // one line edited in the middle
        assert_eq!(changed_lines("a\nB\nc\n", "a\nb\nc\n"), Some((1, 1)));
        // a line added in the middle marks it alone
        assert_eq!(changed_lines("a\nnew\nb\nc\n", "a\nb\nc\n"), Some((1, 1)));
        // a line removed marks the line the rest moved onto
        assert_eq!(changed_lines("a\nc\n", "a\nb\nc\n"), Some((1, 1)));
        // everything differs
        assert_eq!(changed_lines("x\n", "a\nb\n"), Some((0, 0)));
        // the end of the file
        assert_eq!(changed_lines("a\nb\nc\n", "a\nb\n"), Some((2, 2)));
    }

    #[test]
    fn indentation_is_counted_in_columns_and_a_tab_reaches_the_next_stop() {
        assert_eq!(indent_columns("    x"), 4);
        assert_eq!(indent_columns("\tx"), 4);
        assert_eq!(indent_columns("  \tx"), 4);
        assert_eq!(indent_columns("\t\tx"), 8);
        assert_eq!(indent_columns("x"), 0);
        assert_eq!(indent_columns(""), 0);
        // the spaces of a comment that ends the line are not indentation
        assert_eq!(indent_columns("//      note"), 0);
    }

    #[test]
    fn the_status_line_sizes_read_as_a_person_expects() {
        assert_eq!(size_label(512), "512 B");
        assert_eq!(size_label(2048), "2.0 KB");
        assert_eq!(size_label(1024 * 1024), "1 MB");
    }

    #[test]
    fn script_reset_click_does_not_require_source_permissions_or_running_info() {
        use aurora_net::scripts::ScriptTarget;
        use uuid::Uuid;

        for (in_object, readable, editable, available, saving, expected) in [
            (true, false, false, true, false, true),
            (true, true, false, true, false, true),
            (true, true, true, true, false, true),
            (true, false, false, false, false, false),
            (true, true, true, true, true, false),
            (false, true, true, false, false, false),
        ] {
            let target = ScriptTarget {
                object: if in_object { Uuid::from_u128(9) } else { Uuid::nil() },
                item: Uuid::from_u128(10),
            };
            let mut editors = super::super::Editors::default();
            editors.open(target, "Script".into(), readable, editable, 0);
            let e = &mut editors.documents[0];
            e.can_reset = available;
            if saving {
                e.saving = Some(super::super::PendingSave {
                    request: Uuid::from_u128(11),
                    source: String::new(),
                    compile: aurora_net::scripts::CompileTarget::Mono,
                    experience: None,
                    running: false,
                });
            }
            assert!(e.running.is_none(), "the reply has not come");
            let (save, reset, running) = run_state(e);
            assert_eq!(
                reset, expected,
                "readable={readable}, editable={editable}, available={available}, saving={saving}"
            );
            assert_eq!(running, None, "no running state to flip without the reply");
            assert!(!save, "nothing to save");
            assert!(e.source.is_empty());
        }
    }

    #[test]
    fn word_wrap_reflows_without_changing_source_selection_or_line_numbers() {
        use crate::theme::Theme;
        use aurora_net::scripts::ScriptTarget;
        use uuid::Uuid;

        let original = format!(
            "// {}\nstring value = \"{}\";\nllOwnerSay(value);",
            "é long comment ".repeat(12),
            "x".repeat(140)
        );
        for (object, editable, processed) in [(false, true, false), (true, true, false), (false, false, false), (true, true, true)] {
            let ctx = egui::Context::default();
            let theme = Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let target = ScriptTarget {
                object: if object { Uuid::from_u128(9) } else { Uuid::nil() },
                item: Uuid::from_u128(10),
            };
            let mut editors = super::super::Editors::default();
            let request = editors.open(target, "Wrapping".into(), true, editable, 0).expect("load");
            editors.loaded(request, target, Ok(original.clone()));
            let e = &mut editors.documents[0];
            e.tools.reset_history = false;
            e.tools.focus_start = false;
            e.tools.processed = processed;
            e.tools.processed_source.clone_from(&original);
            let id = Id::unique("wrapping_source");
            let text_id = visible_id(id, processed);
            editing::select(&ctx, text_id, 3..12, true);
            let selection = editing::cursor(&ctx, text_id);
            let mut narrow_rows = 0;
            for (wrap, width) in [(false, 320.0), (true, 320.0), (true, 540.0), (false, 320.0), (true, 320.0)] {
                let settings = ScriptSettings {
                    word_wrap: wrap,
                    ..Default::default()
                };
                let mut frame = || {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 900.0))),
                            focused: true,
                            ..Default::default()
                        },
                        |ui| source(ui, &p, e, id, &settings, 800.0, &Search::default(), &[]),
                    )
                };
                for _ in 0..2 {
                    frame().textures_delta.clear();
                }
                let mut output = frame();
                let texts: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) => Some(text),
                        _ => None,
                    })
                    .collect();
                let galley = &texts
                    .iter()
                    .find(|text| text.galley.job.text == original)
                    .expect("source galley")
                    .galley;
                if wrap {
                    assert!(galley.size().x <= width - 64.0, "wrapped width: {:?} / {width}", galley.size());
                    assert!(galley.rows.len() > 3);
                    if width == 320.0 {
                        narrow_rows = galley.rows.len();
                    } else {
                        assert!(galley.rows.len() < narrow_rows, "resizing must reflow the source");
                    }
                } else {
                    assert_eq!(galley.rows.len(), 3);
                    assert!(galley.size().x > width);
                }
                let numbers: Vec<_> = texts.iter().filter_map(|text| text.galley.job.text.parse::<usize>().ok()).collect();
                assert_eq!(numbers, [0, 1, 2], "one number per source line, never on a continuation");
                assert_eq!(
                    editing::cursor(&ctx, text_id),
                    selection,
                    "wrap={wrap}, width={width}, editable={editable}, processed={processed}"
                );
                assert_eq!(e.source, original);
                assert!(!e.dirty());
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn resizing_fills_the_editor_without_painting_the_gutter_over_the_footer() {
        use crate::theme::Theme;
        for object in [false, true] {
            let ctx = egui::Context::default();
            let theme = Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let target = aurora_net::scripts::ScriptTarget {
                object: if object { uuid::Uuid::from_u128(9) } else { uuid::Uuid::nil() },
                item: uuid::Uuid::from_u128(10),
            };
            let mut editors = super::super::Editors::default();
            let request = editors.open(target, "Resize".into(), true, true, 0).unwrap();
            editors.loaded(request, target, Ok("default {\n    event() {\n        code;\n    }\n}".into()));
            let e = &mut editors.documents[0];
            let settings = ScriptSettings::default();
            let mut prefs = Panels::default();
            let short_source = "default {\n    event() {\n        code;\n    }\n}";
            let long_source = format!("// {}\n{short_source}", "long line ".repeat(100));
            for size in [Vec2::new(660.0, 440.0), Vec2::new(1100.0, 880.0), Vec2::new(520.0, 380.0)] {
                for (source, folded) in [(short_source, false), (short_source, true), (long_source.as_str(), false)] {
                    e.source = source.into();
                    e.folding.sync(&e.source);
                    if folded {
                        e.folding.collapse_all();
                    } else {
                        e.folding.expand_all();
                    }
                    let mut bounds = egui::Rect::NOTHING;
                    let mut bottom = 0.0;
                    let mut frame = || {
                        ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                                ..Default::default()
                            },
                            |ui| {
                                bounds = ui.available_rect_before_wrap();
                                body(
                                    ui,
                                    &p,
                                    e,
                                    Id::unique("resize_source"),
                                    &settings,
                                    &mut prefs,
                                    &mut vec![],
                                    true,
                                    &[],
                                    None,
                                    &mut false,
                                    &mut DockTab::default(),
                                    None,
                                    None,
                                    None,
                                );
                                bottom = ui.min_rect().bottom();
                            },
                        )
                    };
                    // read_response exposes the previous frame's widget geometry.
                    for _ in 0..2 {
                        let mut warmup = frame();
                        warmup.textures_delta.clear();
                    }
                    let mut output = frame();
                    let background = settings.color(ColorRole::Background, &p);
                    let rectangles: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::epaint::Shape::Rect(rect) if rect.fill == background => Some(rect.rect),
                            _ => None,
                        })
                        .collect();
                    let editor = rectangles
                        .iter()
                        .max_by(|a, b| a.area().total_cmp(&b.area()))
                        .expect("editor background");
                    let gutter = rectangles.iter().find(|rect| (rect.width() - 56.0).abs() < 0.5).expect("gutter");
                    let input = ctx.read_response(Id::unique("resize_source")).expect("source input");
                    assert!(
                        input.rect.height() >= gutter.height() - 0.5,
                        "clickable editor: {:?} / {editor:?}",
                        input.rect
                    );
                    let least = if size.y > 800.0 { size.y - 360.0 } else { 120.0 };
                    assert!(editor.height() > least, "{editor:?}");
                    assert!(
                        (4.0..25.0).contains(&(editor.bottom() - gutter.bottom())),
                        "{editor:?} / {gutter:?}"
                    );
                    assert!(
                        (bounds.bottom() - bottom).abs() < 1.0,
                        "unused/overflowing space: {bounds:?} / {bottom}"
                    );
                    output.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn commands_enforce_permissions_and_processed_selection_does_not_replace_the_source_cursor() {
        let target = aurora_net::scripts::ScriptTarget {
            object: uuid::Uuid::nil(),
            item: uuid::Uuid::from_u128(10),
        };
        let mut editors = super::super::Editors::default();
        let request = editors.open(target, "Main".into(), true, false, 0).unwrap();
        editors.loaded(request, target, Ok("éあ original".into()));
        let e = &mut editors.documents[0];
        let ctx = egui::Context::default();
        let id = Id::unique("commands");
        let settings = ScriptSettings::default();
        let mut prefs = Panels::default();
        let mut actions = vec![];
        editing::select(&ctx, id, 0..2, false);
        assert!(enabled(Command::Copy, e, &ctx, id));
        assert!(!enabled(Command::Cut, e, &ctx, id));
        assert!(!enabled(Command::Export, e, &ctx, id), "a read-only script saves no file");
        run(Command::Cut, e, &ctx, id, &settings, &mut prefs, &mut actions);
        run(Command::Paste, e, &ctx, id, &settings, &mut prefs, &mut actions);
        assert_eq!(e.source, "éあ original");
        assert!(actions.is_empty());
        e.can_edit = true;
        assert!(enabled(Command::Export, e, &ctx, id), "an editable script saves to a file");
        e.tools.processed = true;
        e.tools.processed_source = "generated source with different length".into();
        run(Command::SelectAll, e, &ctx, id, &settings, &mut prefs, &mut actions);
        assert_eq!(
            editing::cursor(&ctx, visible_id(id, true)).as_sorted_char_range().end.0,
            e.tools.processed_source.chars().count()
        );
        assert_eq!(editing::cursor(&ctx, id).as_sorted_char_range().end.0, 2);
        assert!(!enabled(Command::Paste, e, &ctx, id));
        assert!(!enabled(Command::Cut, e, &ctx, id));
        assert_eq!(e.source, "éあ original");
    }
}
