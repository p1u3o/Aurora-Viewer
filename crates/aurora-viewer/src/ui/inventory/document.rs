//! LLPreviewNotecard document lifecycle (Firestorm, originally LGPL 2.1).
//! Keep the draft until its own upload is acknowledged, including on close.

use super::*;
use aurora_net::inventory::notecard::Notecard;

pub(super) struct Description {
    pub item: Uuid,
    pub original: String,
    pub text: String,
}

pub fn document_demo_events(ctx: &egui::Context, frame: u64, st: &mut InventoryUi) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_INVENTORY") else {
        return Vec::new();
    };
    if matches!(
        mode.as_str(),
        "note-markdown" | "note-source" | "note-classic" | "note-format" | "note-remember" | "note-lsl" | "note-lsl-markdown"
    ) {
        if frame < 600 && mode != "note-remember" {
            st.notecard_view = Some(match mode.as_str() {
                "note-markdown" | "note-lsl-markdown" => super::markdown::View::Markdown,
                "note-source" | "note-format" | "note-lsl" => super::markdown::View::Edit,
                _ => super::markdown::View::Classic,
            });
        }
        if frame == 610
            && let Some(item) = st.preview
        {
            // Saved auxiliary panels can overlap the scenario's document.
            ctx.move_to_top(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::unique(preview_window_id(st.ui_id, item, 7)),
            ));
        }
        if mode == "note-format"
            && let Some(item) = st.preview
        {
            let id = egui::Id::unique(("inventory_document_text", st.ui_id, item));
            if frame == 650
                && let Some(Ok(text)) = &st.preview_text
                && let Some(start) = text.find("- ").map(|at| at + 2)
            {
                let end = text[start..].find(char::is_whitespace).map_or(text.len(), |at| start + at);
                let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(text[..start].chars().count()),
                    egui::text::CCursor::new(text[..end].chars().count()),
                )));
                state.store(ctx, id);
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
            let button = match frame {
                670 | 671 => Some("demo_format_bold"),
                710 | 711 => Some("demo_format_help"),
                _ => None,
            };
            if let Some(rect) = button.and_then(|key| ctx.data_mut(|data| data.get_temp::<egui::Rect>(id.with(key)))) {
                return vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: matches!(frame, 670 | 710),
                        modifiers: egui::Modifiers::NONE,
                    },
                ];
            }
        }
        return Vec::new();
    }
    if !matches!(
        mode.as_str(),
        "note-edit"
            | "note-save"
            | "note-close"
            | "note-error"
            | "note-embedded"
            | "note-task-save"
            | "note-task-error"
            | "note-drop"
            | "note-image"
            | "note-landmark"
            | "note-copy"
            | "note-menu-right"
            | "note-rez"
    ) {
        return Vec::new();
    }
    let Some(item) = st.preview else { return Vec::new() };
    let editor = egui::Id::unique(("inventory_document_text", st.ui_id, item));
    // note-rez: the embedded object dragged out of the text from frame 650,
    // released over the world at frame 680 (RezObjectFromNotecard)
    if mode == "note-rez" {
        let Some(rect) = ctx.data_mut(|data| data.get_temp::<egui::Rect>(editor.with(("demo_attachment", '\u{100002}')))) else {
            return Vec::new();
        };
        let start = rect.center();
        let screen = ctx.content_rect();
        let target = egui::pos2(screen.right() - screen.width() * 0.12, screen.bottom() - screen.height() * 0.3);
        let pos = match frame {
            650 => start,
            651..=679 => start + (target - start) * ((frame - 650) as f32 / 30.0),
            680 => target,
            _ => return Vec::new(),
        };
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if matches!(frame, 650 | 680) {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: frame == 650,
                modifiers: Default::default(),
            });
        }
        return events;
    }
    if mode == "note-drop" {
        let Some(rect) = ctx.data_mut(|data| data.get_temp::<egui::Rect>(editor.with("demo_rect"))) else {
            return Vec::new();
        };
        let pos = rect.min + egui::vec2(24.0, 12.0);
        if frame == 650 {
            egui::DragAndDrop::set_payload(ctx, InvDrag(Uuid::from_u128(8104)));
        }
        if matches!(frame, 650 | 651) {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 650,
                    modifiers: Default::default(),
                },
            ];
        }
        if frame == 670 {
            ctx.memory_mut(|m| m.request_focus(editor));
        }
    }
    if matches!(mode.as_str(), "note-image" | "note-landmark" | "note-copy" | "note-menu-right") {
        if mode == "note-copy"
            && matches!(frame, 670 | 671)
            && let Some(rect) = ctx.data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("notecard_copy_confirm", st.ui_id))))
        {
            return vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: frame == 670,
                    modifiers: Default::default(),
                },
            ];
        }
        let marker = if matches!(mode.as_str(), "note-image" | "note-menu-right") {
            '\u{100000}'
        } else if mode == "note-landmark" {
            '\u{100001}'
        } else {
            '\u{100002}'
        };
        if !matches!(frame, 650 | 651) {
            return Vec::new();
        }
        let Some(rect) = ctx.data_mut(|data| data.get_temp::<egui::Rect>(editor.with(("demo_attachment", marker)))) else {
            return Vec::new();
        };
        let pos = rect.center();
        return vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: if mode == "note-menu-right" {
                    egui::PointerButton::Secondary
                } else {
                    egui::PointerButton::Primary
                },
                pressed: frame.is_multiple_of(2),
                modifiers: Default::default(),
            },
        ];
    }
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    };
    if frame == 650 && mode != "note-drop" {
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("inventory_document_text", st.ui_id, item))));
        return vec![
            key(egui::Key::End),
            egui::Event::Text(aurora_i18n::tr!("notecard-demo-edit").into()),
        ];
    }
    if frame == 670
        && matches!(
            mode.as_str(),
            "note-save" | "note-error" | "note-task-save" | "note-task-error" | "note-drop"
        )
    {
        return vec![key(egui::Key::S)];
    }
    if frame == 670 && mode == "note-close" {
        st.dialog = Some(EditDialog::Discard(item));
    }
    Vec::new()
}

impl InventoryUi {
    /// LLAnimationBridge::performAction / LLPreviewAnim::play: playback uses
    /// the existing preview, or opens one for a context-menu action.
    pub fn open_animation(&mut self, ctx: &egui::Context, inv: &Inventory, asset: Uuid) {
        let matches = |item: &aurora_net::inventory::InvItem| item.asset_type == 20 && item.asset_id == asset;
        let item = std::iter::once(&*self)
            .chain(self.windows.iter().map(|w| &*w.state))
            .filter_map(|state| state.preview.and_then(|id| inv.items.get(&id)))
            .find(|item| matches(item))
            .or_else(|| inv.items.values().find(|item| matches(item)))
            .map(|item| item.id);
        if let Some(item) = item {
            self.open_document(ctx, item, 20, None);
        }
    }

    /// LLFloaterReg::showInstance: each item owns its document. Opening an
    /// existing item raises/restores it without replacing an unsaved draft.
    pub fn open_document(&mut self, ctx: &egui::Context, item: Uuid, kind: i32, owner: Option<egui::Id>) -> bool {
        let existing = if self.preview == Some(item) {
            self.document_owner = owner;
            Some((self.ui_id, self.preview_text.is_none()))
        } else {
            self.windows.iter_mut().find(|w| w.state.preview == Some(item)).map(|w| {
                w.state.document_owner = owner;
                (w.state.ui_id, w.state.preview_text.is_none())
            })
        };
        if let Some((editor, loading)) = existing {
            // Reopening an existing document should activate it too, even if
            // another application field currently holds keyboard focus.
            if kind == 7 {
                if self.ui_id == editor {
                    self.notecard_focus_pending = true;
                } else if let Some(window) = self.windows.iter_mut().find(|w| w.state.ui_id == editor) {
                    window.state.notecard_focus_pending = true;
                }
            }
            let id = preview_window_id(editor, item, kind);
            super::super::widgets::open_floater(ctx, &id, &mut true);
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(id)));
            return loading;
        }
        let mut state = Self::document(item);
        state.document_owner = owner;
        self.windows.push(InventoryWindow {
            id: state.ui_id,
            root: Uuid::nil(),
            additional: false,
            state: Box::new(state),
            open: false,
        });
        true
    }

    pub fn document(item: Uuid) -> Self {
        Self {
            ui_id: Uuid::new_v4(),
            preview: Some(item),
            ..Default::default()
        }
    }

    pub fn reset_document(&mut self) {
        self.preview_text = None;
        self.preview_can_edit = false;
        self.preview_source = None;
        self.notecard_view = Default::default();
        self.notecard_find = Default::default();
        self.notecard_focus_pending = false;
        self.notecard_markdown = Default::default();
        self.attachment_preview = None;
        self.preview_original.clear();
        self.preview_description = None;
        self.preview_message.clear();
        self.preview_dirty = false;
        self.save_pending = false;
        self.content_save = None;
        self.close_after_save = false;
    }

    pub fn load_document(&mut self, kind: i32, can_edit: bool, result: &Result<Vec<u8>, String>) {
        // A late download (another window or a refresh) must not erase a draft.
        if self.preview_dirty || self.save_pending {
            return;
        }
        if self.preview_text.as_ref().is_some_and(Result::is_ok)
            && let Err(reason) = result
        {
            self.preview_message.clone_from(reason);
            return;
        }
        // A newly loaded document is ready to take focus on its first
        // interactive frame, not while an egui floater is sizing itself.
        let was_loaded = self.preview_text.as_ref().is_some_and(Result::is_ok);
        self.preview_source = None;
        self.preview_can_edit = false;
        let result = result.as_ref().map_err(Clone::clone).and_then(|data| {
            if kind == 7 {
                let note = Notecard::decode(data)?;
                let text = note.text.clone();
                self.preview_source = Some(note);
                self.preview_can_edit = can_edit;
                Ok(text)
            } else {
                let (text, editable) = crate::world::inventory::preview::text(kind, data)?;
                self.preview_can_edit = can_edit && editable;
                Ok(text)
            }
        });
        if let Ok(text) = &result {
            self.preview_original.clone_from(text);
        }
        if kind == 7 && result.is_ok() && !was_loaded {
            self.notecard_focus_pending = true;
        }
        self.preview_text = Some(result);
    }

    pub fn document_changed(&mut self) {
        self.preview_dirty = self
            .preview_text
            .as_ref()
            .is_some_and(|r| r.as_ref().is_ok_and(|t| t != &self.preview_original));
        self.preview_message.clear();
    }

    pub fn start_content_save(&mut self, request: Uuid, kind: i32, text: String, close: bool) -> Result<Vec<u8>, String> {
        if self.save_pending
            || !self.preview_can_edit
            || !self.preview_dirty
            || self.preview_text.as_ref().and_then(|r| r.as_ref().ok()) != Some(&text)
        {
            return Err(aurora_i18n::tr!("notecard-edit-denied").into());
        }
        let data = if kind == 7 {
            self.preview_source
                .as_ref()
                .ok_or_else(|| aurora_i18n::tr!("world-inventory-preview-invalid-notecard").to_owned())?
                .encode(&text)?
        } else {
            text.as_bytes().to_vec()
        };
        self.content_save = Some((request, data.clone(), text));
        self.save_pending = true;
        self.close_after_save = close;
        self.preview_message.clear();
        Ok(data)
    }

    pub fn finish_content_save(&mut self, request: Uuid, error: Option<&str>) -> bool {
        if self.content_save.as_ref().is_none_or(|(id, _, _)| *id != request) {
            return false;
        }
        let Some((_, data, saved_text)) = self.content_save.take() else {
            return false;
        };
        self.save_pending = false;
        if let Some(reason) = error {
            self.close_after_save = false;
            self.preview_message = aurora_i18n::tr!("notecard-save-failed", reason = reason);
            return true;
        }
        self.preview_original.clone_from(&saved_text);
        self.document_changed();
        if !self.preview_dirty
            && self.preview_source.is_some()
            && let Ok(note) = Notecard::decode(&data)
        {
            self.preview_original.clone_from(&note.text);
            self.preview_text = Some(Ok(note.text.clone()));
            self.preview_source = Some(note);
        }
        self.preview_message = aurora_i18n::tr!("app-document-saved").into();
        if self.close_after_save && !self.preview_dirty {
            self.preview = None;
            self.reset_document();
            self.dialog = None;
        }
        self.close_after_save = false;
        true
    }
}

pub(super) fn preview_window_id(editor: Uuid, item: Uuid, kind: i32) -> String {
    format!(
        "{}_{}_{}",
        if kind == 7 { "notecard_preview" } else { "inventory_preview" },
        editor,
        item
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_playback_reuses_its_window_and_preserves_another_document() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        let asset = Uuid::from_u128(10);
        let items = [Uuid::from_u128(2), Uuid::from_u128(3)];
        inv.add_items(
            items
                .map(|id| aurora_net::inventory::InvItem {
                    id,
                    asset_id: asset,
                    asset_type: 20,
                    ..Default::default()
                })
                .to_vec(),
        );
        for item in items {
            let mut state = draft("Unsaved note");
            state.open_document(&ctx, item, 20, None);
            let editor = state.windows[0].id;
            state.windows[0].state.animation_description = Some((item, "Edited description".into()));
            for _ in 0..3 {
                state.open_animation(&ctx, &inv, asset);
                assert_eq!(state.windows.len(), 1);
                assert_eq!(state.windows[0].id, editor);
                assert_eq!(state.windows[0].state.preview, Some(item));
                assert_eq!(
                    state.windows[0].state.animation_description,
                    Some((item, "Edited description".into()))
                );
                assert_eq!(state.preview, Some(Uuid::from_u128(1)));
                assert_eq!(state.preview_text, Some(Ok("Unsaved note".into())));
                assert!(state.preview_dirty);
            }
        }
    }

    #[test]
    fn animation_menu_playback_opens_once_and_reuses_a_main_preview() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        let item = Uuid::from_u128(2);
        let asset = Uuid::from_u128(10);
        inv.add_items(vec![aurora_net::inventory::InvItem {
            id: item,
            asset_id: asset,
            asset_type: 20,
            ..Default::default()
        }]);
        let mut state = InventoryUi::default();
        state.open_animation(&ctx, &inv, Uuid::from_u128(11));
        assert!(state.preview.is_none() && state.windows.is_empty());
        for _ in 0..3 {
            state.open_animation(&ctx, &inv, asset);
            assert!(state.preview.is_none());
            assert_eq!(state.windows.len(), 1);
            assert_eq!(state.windows[0].state.preview, Some(item));
        }
        let mut main = InventoryUi::document(item);
        main.open_animation(&ctx, &inv, asset);
        assert_eq!(main.preview, Some(item));
        assert!(main.windows.is_empty());
    }

    #[test]
    fn cascade_notecards_keep_independent_drafts_and_restore_existing_items() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        let items = [Uuid::from_u128(1), Uuid::from_u128(2)];
        inv.add_items(
            items
                .map(|id| aurora_net::inventory::InvItem {
                    id,
                    asset_type: 7,
                    name: format!("Note {id}"),
                    ..Default::default()
                })
                .to_vec(),
        );
        let mut state = InventoryUi::default();
        let mut frame = 0;
        for (n, item) in items.into_iter().enumerate() {
            assert!(state.open_document(&ctx, item, 7, None));
            state.windows[n].state.load_document(7, true, &Ok(b"Original".to_vec()));
            for _ in 0..8 {
                frame += 1;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |ui| {
                        for w in &mut state.windows {
                            show_document(
                                ui.ctx(),
                                &crate::theme::Theme::default().palette(),
                                &Icons::default(),
                                &inv,
                                &mut w.state,
                                &inv,
                                &Default::default(),
                                &mut InventoryPreferences::default(),
                                &Default::default(),
                            );
                        }
                    },
                );
                output.textures_delta.clear();
            }
            state.windows[n].state.preview_text = Some(Ok(format!("Draft {n}")));
            state.windows[n].state.document_changed();
        }
        let ids: Vec<_> = state
            .windows
            .iter()
            .zip(items)
            .map(|(w, item)| preview_window_id(w.id, item, 7))
            .collect();
        let rects: Vec<_> = ids
            .iter()
            .map(|id| ctx.memory(|m| m.area_rect(egui::Id::unique(id)).expect("note window")))
            .collect();
        assert!(((rects[1].min - rects[0].min) - egui::Vec2::splat(32.0)).length() < 1.0);
        crate::ui::widgets::minimize_floater(&ctx, &ids[0]);
        assert!(!state.open_document(&ctx, items[0], 7, None));
        assert_eq!(state.windows.len(), 2);
        assert!(state.windows.iter().all(|w| w.root.is_nil() && !w.open && w.state.preview_dirty));
        assert_eq!(state.windows[0].state.preview_text, Some(Ok("Draft 0".into())));
        assert_eq!(state.windows[1].state.preview_text, Some(Ok("Draft 1".into())));
        assert_eq!(ctx.memory(|m| m.area_rect(egui::Id::unique(&ids[0]))), Some(rects[0]));
    }

    fn draft(text: &str) -> InventoryUi {
        let mut st = InventoryUi {
            preview: Some(Uuid::from_u128(1)),
            ..Default::default()
        };
        st.load_document(7, true, &Ok(Vec::new()));
        st.preview_text = Some(Ok(text.into()));
        st.document_changed();
        st
    }

    #[test]
    fn failed_save_keeps_the_draft_and_success_closes_only_after_acknowledgement() {
        let mut st = draft("Été 🐺");
        let request = Uuid::from_u128(2);
        st.start_content_save(request, 7, "Été 🐺".into(), true).expect("upload");
        assert!(st.preview.is_some());
        assert!(!st.finish_content_save(Uuid::from_u128(3), None));
        assert!(st.save_pending);
        assert!(st.finish_content_save(request, Some("HTTP 503")));
        assert!(st.preview_dirty && st.preview.is_some());
        assert_eq!(st.preview_text, Some(Ok("Été 🐺".into())));
        assert!(!st.save_pending);
        st.start_content_save(request, 7, "Été 🐺".into(), true).expect("retry");
        st.finish_content_save(request, None);
        assert!(st.preview.is_none());
        assert!(!st.preview_dirty);
    }

    #[test]
    fn undo_restores_pristine_and_refresh_preserves_unsaved_changes() {
        let mut st = draft("Texte modifié");
        st.load_document(7, true, &Ok(b"old content".to_vec()));
        assert_eq!(st.preview_text, Some(Ok("Texte modifié".into())));
        st.preview_text = Some(Ok(String::new()));
        st.document_changed();
        assert!(!st.preview_dirty);
    }

    #[test]
    fn saving_one_window_does_not_mark_a_second_draft_saved() {
        let mut a = draft("Première fenêtre");
        let mut b = draft("Autre brouillon");
        let request = Uuid::from_u128(2);
        a.start_content_save(request, 7, "Première fenêtre".into(), false).expect("save");
        assert!(!b.finish_content_save(request, None));
        a.finish_content_save(request, None);
        assert!(!a.preview_dirty && b.preview_dirty);
        assert_eq!(b.preview_text, Some(Ok("Autre brouillon".into())));
    }
}

pub(super) mod notecard_find {
    //! Find and Replace for an inventory or object notecard's source text.
    //! A document owns this state; the rendered Markdown preview is not edited.

    use std::ops::Range;

    use crate::{
        theme::Palette,
        ui::widgets::{Floater, checkbox, flat_button, search_text_edit, text_field},
    };
    use aurora_net::inventory::notecard::{FIRST_EMBEDDED_CHAR, Notecard};
    use egui::{
        Id, RichText, TextEdit, Vec2,
        text::{CCursor, CCursorRange},
    };

    pub(in crate::ui::inventory) struct Search {
        pub open: bool,
        query: String,
        replacement: String,
        insensitive: bool,
        whole_words: bool,
        up: bool,
        focus: bool,
        current: Option<Range<usize>>,
        status: String,
    }

    impl Default for Search {
        fn default() -> Self {
            Self {
                open: false,
                query: String::new(),
                replacement: String::new(),
                insensitive: true,
                whole_words: false,
                up: false,
                focus: false,
                current: None,
                status: String::new(),
            }
        }
    }

    /// Search decorations are attached to the source editor, not the rendered
    /// Markdown preview. They remain visible when the find floater has focus.
    #[derive(Clone, Default)]
    pub(in crate::ui::inventory) struct Highlights {
        pub ranges: Vec<Range<usize>>,
        pub current: Option<Range<usize>>,
    }

    fn selected(ctx: &egui::Context, id: Id) -> Range<usize> {
        egui::TextEdit::load_state(ctx, id)
            .and_then(|state| state.cursor.char_range())
            .map_or(0..0, |range| {
                let [start, end] = range.sorted_cursors();
                start.index.0..end.index.0
            })
    }

    fn select(ctx: &egui::Context, id: Id, range: Range<usize>) {
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end))));
        state.store(ctx, id);
        ctx.request_repaint();
    }

    fn matches(text: &str, needle: &str, insensitive: bool, whole_words: bool) -> Vec<Range<usize>> {
        // Embedded attachments are one reserved character each. Never treat
        // their private-use markers as ordinary editable notecard text.
        let chars: Vec<_> = text.chars().collect();
        crate::ui::scripts::search_matches_filtered(text, needle, insensitive, whole_words)
            .into_iter()
            .filter(|range| !chars[range.start..range.end].iter().any(|c| *c as u32 >= FIRST_EMBEDDED_CHAR))
            .collect()
    }

    fn valid_replacement(text: &str) -> Result<(), String> {
        if text.chars().any(|c| c as u32 >= FIRST_EMBEDDED_CHAR) {
            return Err(aurora_i18n::tr!("world-inventory-preview-invalid-notecard").into());
        }
        Ok(())
    }

    /// Return an updated string, checking notecard (not script) limits before
    /// touching either the editor's draft or its undo history.
    fn replaced(text: &str, range: Range<usize>, replacement: &str) -> Result<String, String> {
        valid_replacement(replacement)?;
        let boundaries: Vec<_> = text.char_indices().map(|(byte, _)| byte).chain([text.len()]).collect();
        let Some((&start, &end)) = boundaries.get(range.start).zip(boundaries.get(range.end)) else {
            return Err(aurora_i18n::tr!("world-inventory-preview-invalid-notecard").into());
        };
        let mut updated = text.to_owned();
        updated.replace_range(start..end, replacement);
        Notecard::validate_text(&updated)?;
        Ok(updated)
    }

    /// Use TextEdit's native undo snapshots just like notecard Markdown formatting.
    /// In particular, Replace All is one undoable operation.
    fn commit(ctx: &egui::Context, id: Id, text: &mut String, updated: String, caret: usize) -> bool {
        if updated == *text {
            return false;
        }
        let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
        let before = state.cursor.char_range().unwrap_or_default();
        let mut undo = state.undoer();
        undo.add_undo(&(before, text.clone()));
        *text = updated;
        let after = CCursorRange::one(CCursor::new(caret));
        state.cursor.set_char_range(Some(after));
        undo.add_undo(&(after, text.clone()));
        state.set_undoer(undo);
        state.store(ctx, id);
        ctx.request_repaint();
        true
    }

    impl Search {
        /// Choose the first match before Find is clicked. Keep the visible active
        /// result, the native text selection, and scrolling in sync.
        fn activate_first(&mut self, ctx: &egui::Context, id: Id, text: &str) {
            let first = matches(text, &self.query, self.insensitive, self.whole_words).into_iter().next();
            self.current = first.clone();
            self.status.clear();
            if let Some(range) = first {
                select(ctx, id, range.clone());
                ctx.data_mut(|data| data.insert_temp(id.with("notecard_find_scroll_to"), range));
            }
            ctx.request_repaint();
        }

        pub fn begin(&mut self, ctx: &egui::Context, id: Id, text: &str) {
            let range = selected(ctx, id);
            if range.end > range.start {
                let selection: String = text.chars().skip(range.start).take(range.end - range.start).collect();
                if !selection.is_empty() && !selection.chars().any(|c| c as u32 >= FIRST_EMBEDDED_CHAR) {
                    self.query = selection;
                }
            }
            if !self.open {
                self.current = None;
            }
            self.open = true;
            self.focus = true;
            self.activate_first(ctx, id, text);
            crate::ui::widgets::expand_floater(ctx, &format!("notecard_search_{id:?}"));
        }

        /// Ctrl+F toggles the active notecard Find floater without modifying text.
        pub fn close(&mut self) {
            self.open = false;
            self.focus = false;
            self.current = None;
        }

        /// Publish matches before the notecard editor is painted. It uses these
        /// to render visible highlighting even while the find field has focus.
        pub fn mark(&self, ctx: &egui::Context, id: Id, text: &str) {
            let ranges = if self.open {
                matches(text, &self.query, self.insensitive, self.whole_words)
            } else {
                Vec::new()
            };
            let selection = selected(ctx, id);
            // The separate Find floater owns keyboard focus. The editor may
            // discard an unfocused native selection, so retain our active match.
            let current = self
                .current
                .as_ref()
                .filter(|range| ranges.contains(range))
                .cloned()
                .or_else(|| ranges.iter().find(|range| **range == selection).cloned());
            ctx.data_mut(|data| data.insert_temp(id.with("notecard_find_highlights"), Highlights { ranges, current }));
        }

        /// A separate floater with the same widgets and row layout as the script
        /// editor's Find / Replace. Its ID is unique to this notecard window.
        pub fn show(
            &mut self,
            ctx: &egui::Context,
            p: &Palette,
            id: Id,
            text: &mut String,
            editable: bool,
            parent: Option<egui::Rect>,
            parent_id: Id,
        ) -> bool {
            if !self.open {
                return false;
            }
            let name = format!("notecard_search_{id:?}");
            let size = Vec2::new(360.0, 216.0);
            let room = crate::ui::layout::work_area(ctx);
            let parent = parent.unwrap_or(room);
            let x = if parent.right() + size.x <= room.right() {
                parent.right()
            } else if parent.left() - size.x >= room.left() {
                parent.left() - size.x
            } else {
                parent.right() - size.x
            };
            let position = egui::pos2(
                x.clamp(room.left(), (room.right() - size.x).max(room.left())),
                parent.top().clamp(room.top(), (room.bottom() - size.y).max(room.top())),
            );
            // Match scripts::dialogs::Search: the window can be dragged but
            // returns to its anchored position when the drag is released.
            crate::ui::widgets::move_floater(ctx, &name, position);
            // Dependent Find follows its active notecard parent without taking
            // priority over unrelated windows when the notecard is not active.
            let owner = egui::LayerId::new(egui::Order::Middle, parent_id);
            let child = egui::LayerId::new(egui::Order::Middle, Id::unique(&name));
            ctx.set_sublayer(owner, child);
            let focused_layer = ctx
                .memory(|memory| memory.focused())
                .and_then(|id| ctx.read_response(id).map(|response| response.layer_id));
            let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
            let pressed_here = pressed.is_some_and(|at| matches!(ctx.layer_id_at(at), Some(layer) if layer == owner || layer == child));
            let front = ctx.memory(|memory| {
                memory
                    .layer_ids()
                    .filter(|layer| layer.order != egui::Order::Background && memory.areas().is_visible(layer))
                    .last()
                    .is_some_and(|layer| layer == owner || layer == child)
            });
            if (self.focus && pressed.is_none())
                || pressed_here
                || (front && (focused_layer == Some(owner) || focused_layer == Some(child)))
                || (front && ctx.is_being_dragged(child.id.with("move")))
            {
                ctx.move_to_top(owner);
            }
            let mut open = true;
            let mut toggle_closed = false;
            let mut changed = false;
            Floater::new(&name, aurora_i18n::tr!("script-search-title"), position, size).show(ctx, p, &mut open, |ui| {
                // Ctrl+F from inside Find / Replace closes it as well.
                toggle_closed = crate::ui::find_shortcut::take(ui, &[id.with("notecard_find_query"), id.with("notecard_replace_query")]);
                if !toggle_closed {
                    changed = self.controls(ui, p, id, text, editable);
                }
            });
            self.open = open;
            if !open || toggle_closed {
                self.close();
                ctx.memory_mut(|memory| {
                    memory.request_focus(if editable { id } else { id.with("notecard_window_focus") });
                });
            }
            changed
        }

        fn find(&mut self, ctx: &egui::Context, id: Id, text: &str) {
            let ranges = matches(text, &self.query, self.insensitive, self.whole_words);
            let from = self.current.clone().unwrap_or_else(|| selected(ctx, id));
            if let Some(next) = crate::ui::scripts::next_search_match(&ranges, from, self.up) {
                select(ctx, id, next.clone());
                // Actual scrolling happens when the source editor renders next frame.
                ctx.data_mut(|data| data.insert_temp(id.with("notecard_find_scroll_to"), next.clone()));
                self.current = Some(next);
                self.status.clear();
            } else {
                self.status = aurora_i18n::tr!("script-search-not-found").into();
            }
        }

        fn replace(&mut self, ctx: &egui::Context, id: Id, text: &mut String) -> bool {
            let ranges = matches(text, &self.query, self.insensitive, self.whole_words);
            let native = selected(ctx, id);
            // When Find owns keyboard focus, the native editor selection can be
            // empty or stale. Use the visible active match for Replace.
            let range = if ranges.contains(&native) {
                native
            } else {
                self.current.clone().unwrap_or(native)
            };
            let mut changed = false;
            if ranges.contains(&range) {
                match replaced(text, range.clone(), &self.replacement) {
                    Ok(updated) => {
                        changed = commit(ctx, id, text, updated, range.start + self.replacement.chars().count());
                    }
                    Err(message) => {
                        self.status = message;
                        return false;
                    }
                }
            }
            self.current = None;
            self.find(ctx, id, text);
            changed
        }

        fn replace_all(&mut self, ctx: &egui::Context, id: Id, text: &mut String) -> bool {
            let ranges = matches(text, &self.query, self.insensitive, self.whole_words);
            if ranges.is_empty() {
                self.status = aurora_i18n::tr!("script-search-not-found").into();
                return false;
            }
            if let Err(message) = valid_replacement(&self.replacement) {
                self.status = message;
                return false;
            }
            let boundaries: Vec<_> = text.char_indices().map(|(byte, _)| byte).chain([text.len()]).collect();
            let mut updated = String::new();
            let mut previous = 0;
            for range in &ranges {
                updated.push_str(&text[previous..boundaries[range.start]]);
                updated.push_str(&self.replacement);
                previous = boundaries[range.end];
            }
            updated.push_str(&text[previous..]);
            if let Err(message) = Notecard::validate_text(&updated) {
                self.status = message;
                return false;
            }
            let count = ranges.len();
            let changed = commit(ctx, id, text, updated, 0);
            self.current = None;
            self.status = aurora_i18n::tr!("script-search-replaced", count = count as i64);
            changed
        }

        /// Keep the same four-row Find / Replace layout as script editors.
        fn controls(&mut self, ui: &mut egui::Ui, p: &Palette, id: Id, text: &mut String, editable: bool) -> bool {
            let mut changed = false;
            let query_id = id.with("notecard_find_query");
            let replace_id = id.with("notecard_replace_query");
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("script-search-query"));
                let field = search_text_edit(ui, p, &mut self.query, |edit| {
                    edit.id(query_id)
                        .hint_text(aurora_i18n::tr!("script-search-query"))
                        .desired_width(220.0)
                });
                if !ui.is_sizing_pass() && std::mem::take(&mut self.focus) {
                    select(ui.ctx(), field.id, 0..self.query.chars().count());
                    field.request_focus();
                }
                if field.changed() {
                    self.activate_first(ui.ctx(), id, text);
                }
                if (field.has_focus() || field.lost_focus())
                    && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    self.find(ui.ctx(), id, text);
                    field.request_focus();
                }
            });
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("script-search-replacement"));
                text_field(ui, editable, &mut self.replacement, |buffer, _| {
                    TextEdit::singleline(buffer).id(replace_id).desired_width(220.0)
                });
            });
            ui.horizontal(|ui| {
                let before = self.insensitive;
                checkbox(ui, p, &mut self.insensitive, aurora_i18n::tr!("script-search-insensitive"));
                if self.insensitive != before {
                    self.activate_first(ui.ctx(), id, text);
                }
                checkbox(ui, p, &mut self.up, aurora_i18n::tr!("script-search-up"));
            });
            ui.horizontal(|ui| {
                let before = self.whole_words;
                checkbox(ui, p, &mut self.whole_words, aurora_i18n::tr!("script-search-whole-words"));
                if self.whole_words != before {
                    self.activate_first(ui.ctx(), id, text);
                }
            });
            ui.horizontal(|ui| {
                if flat_button(ui, p, aurora_i18n::tr!("script-search-find")).clicked() {
                    self.find(ui.ctx(), id, text);
                }
                ui.add_enabled_ui(editable && !self.query.is_empty(), |ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("script-search-replace")).clicked() {
                        changed |= self.replace(ui.ctx(), id, text);
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("script-search-replace-all")).clicked() {
                        changed |= self.replace_all(ui.ctx(), id, text);
                    }
                });
            });
            ui.label(RichText::new(&self.status).color(p.muted));
            changed
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn searches_unicode_in_both_directions_without_matching_attachments() {
            let marker = char::from_u32(FIRST_EMBEDDED_CHAR).unwrap();
            let text = format!("Été {marker} été ÉTÉ");
            let ranges = matches(&text, "été", true, false);
            assert_eq!(ranges, vec![0..3, 6..9, 10..13]);
            assert!(matches(&text, &marker.to_string(), false, false).is_empty());
            assert_eq!(crate::ui::scripts::next_search_match(&ranges, 10..13, false), Some(0..3));
            assert_eq!(crate::ui::scripts::next_search_match(&ranges, 0..3, true), Some(10..13));
        }

        #[test]
        fn whole_words_option_filters_notecard_find_and_attachment_markers() {
            let marker = char::from_u32(FIRST_EMBEDDED_CHAR).unwrap();
            let text = format!("This hi high hi_there hi-there {marker} (hi)");
            let found = matches(&text, "hi", true, true);
            assert_eq!(found.len(), 4);
            assert!(matches(&text, "hi", true, false).len() > found.len());
            let mut search = Search {
                query: "hi".into(),
                whole_words: true,
                ..Default::default()
            };
            let ctx = egui::Context::default();
            let id = Id::unique("notecard_whole_words");
            search.find(&ctx, id, &text);
            assert_eq!(search.current, found.first().cloned());
            search.open = true;
            search.close();
            assert!(!search.open);
            assert!(search.current.is_none());
        }

        #[test]
        fn new_query_activates_the_first_result_and_replace_works_immediately() {
            let ctx = egui::Context::default();
            let id = Id::unique("notecard_immediate_replace");
            let mut text = "hi high hi_there".to_owned();
            let mut search = Search {
                open: true,
                query: "hi".into(),
                replacement: "HELLO".into(),
                whole_words: true,
                ..Default::default()
            };
            search.activate_first(&ctx, id, &text);
            assert_eq!(search.current, Some(0..2));
            assert_eq!(selected(&ctx, id), 0..2);
            search.mark(&ctx, id, &text);
            let marked = ctx
                .data(|data| data.get_temp::<Highlights>(id.with("notecard_find_highlights")))
                .unwrap();
            assert_eq!(marked.current, Some(0..2));
            assert!(search.replace(&ctx, id, &mut text));
            assert_eq!(text, "HELLO high hi_there");
        }

        #[test]
        fn find_navigation_tracks_its_active_match_and_requests_scroll() {
            let ctx = egui::Context::default();
            let id = Id::unique("notecard_find_navigation");
            let mut search = Search {
                query: "hello".into(),
                ..Default::default()
            };
            let text = "Hello there, hello again HELLO";
            search.find(&ctx, id, text);
            assert_eq!(search.current, Some(0..5));
            search.find(&ctx, id, text);
            assert_eq!(search.current, Some(13..18));
            search.find(&ctx, id, text);
            assert_eq!(search.current, Some(25..30));
            search.find(&ctx, id, text);
            assert_eq!(search.current, Some(0..5));
            let pending = ctx.data(|data| data.get_temp::<Range<usize>>(id.with("notecard_find_scroll_to")));
            assert_eq!(pending, Some(0..5));
            search.up = true;
            search.find(&ctx, id, text);
            assert_eq!(search.current, Some(25..30));
        }

        #[test]
        fn source_highlights_all_matches_and_marks_the_selected_one() {
            let ctx = egui::Context::default();
            let id = Id::unique("notecard_highlight_test");
            let text = "Hello there, HELLO again";
            let search = Search {
                open: true,
                query: "hello".into(),
                ..Default::default()
            };
            select(&ctx, id, 13..18);
            search.mark(&ctx, id, text);
            let highlights = ctx
                .data(|data| data.get_temp::<Highlights>(id.with("notecard_find_highlights")))
                .unwrap();
            assert_eq!(highlights.ranges, vec![0..5, 13..18]);
            assert_eq!(highlights.current, Some(13..18));
        }

        #[test]
        fn replacement_preserves_markers_and_enforces_notecard_limits() {
            let marker = char::from_u32(FIRST_EMBEDDED_CHAR).unwrap();
            let text = format!("Hi {marker} café!");
            assert_eq!(replaced(&text, 5..9, "tea").unwrap(), format!("Hi {marker} tea!"));
            assert!(replaced(&text, 5..9, &marker.to_string()).is_err());
            assert!(replaced(&text, 5..9, &"x".repeat(aurora_net::inventory::notecard::MAX_TEXT_BYTES)).is_err());
            assert!(replaced(&text, 5..9, "\0").is_err());
        }

        #[test]
        fn replace_all_is_one_undoable_document_edit() {
            let ctx = egui::Context::default();
            let id = Id::unique("notecard_find_undo");
            let mut text = "café and café".to_owned();
            let mut search = Search {
                query: "café".into(),
                replacement: "tea".into(),
                ..Default::default()
            };
            assert!(search.replace_all(&ctx, id, &mut text));
            assert_eq!(text, "tea and tea");
            let state = TextEdit::load_state(&ctx, id).unwrap();
            let mut undo = state.undoer();
            let previous = undo.undo(&(state.cursor.char_range().unwrap_or_default(), text.clone()));
            assert!(previous.is_some_and(|(_, before)| before == "café and café"));
        }
    }
}
