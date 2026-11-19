//! Completion of LSL and SLua names, after Alchemy's Script Studio: the words
//! of the keywords in use (the region's own syntax included) and the words the
//! script itself writes, offered under the caret as it is typed, with the
//! signature and the help of the one chosen.
//!
//! The list belongs to its field, as the emoji codes' list does: it lives in
//! the frame's memory under the field's id, takes the arrows, Enter, Tab and
//! Escape before the field sees them, and writes its choice in the source as
//! an edit the undoer knows.

use super::editing;
use crate::theme::Palette;
use crate::ui::{menu, widgets};
use egui::text_edit::TextEditOutput;
use egui::{Id, RichText};
use std::ops::Range;

/// The least letters typed before the list opens: one letter would offer half
/// the language.
const MIN_TYPED: usize = 2;

/// The most names the list holds.
const MOST: usize = 64;

/// The room the list gives its rows before it scrolls.
const ROWS_HEIGHT: f32 = 180.0;

/// What a suggestion is: a word of the script, or a kind of the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    /// A name the script declares or uses: nothing the catalog knows.
    Name,
    Function,
    Event,
    Type,
    Constant,
    Control,
}

/// One name the list offers.
#[derive(Clone)]
pub(super) struct Item {
    pub text: String,
    pub kind: Kind,
    /// Its signature, as the catalog writes it (empty for a name of the script).
    pub detail: String,
    /// What it does (empty for a name of the script).
    pub docs: String,
    /// Its letters in one case, so that `llSay` comes before `llSHA1String`.
    lower: String,
}

impl Kind {
    fn of(group: &str) -> Kind {
        match group {
            "functions" => Kind::Function,
            "events" => Kind::Event,
            "types" => Kind::Type,
            "constants" => Kind::Constant,
            _ => Kind::Control,
        }
    }

    /// The Tabler icon of the kind.
    fn icon(self) -> &'static str {
        match self {
            Kind::Name | Kind::Constant => "variable",
            Kind::Function => "function-filled",
            Kind::Event => "bolt-filled",
            Kind::Type => "braces",
            Kind::Control => "code",
        }
    }
}

/// The name the caret is in: the letters typed so far, and the whole word an
/// accepted name replaces.
pub(super) struct Typed {
    /// The letters before the caret, in char indices.
    pub range: Range<usize>,
    /// The letters typed so far.
    pub text: String,
    /// The whole word around the caret.
    pub word: Range<usize>,
}

fn is_ident(ch: char, lua: bool) -> bool {
    ch == '_' || ch.is_alphanumeric() || (lua && ch == '.')
}

/// What is being typed at `at` (a caret position in char indices), or None
/// when the caret is not in a name, is in a comment or a string, or the name
/// starts with a digit.
pub(super) fn typed(source: &str, at: usize, lua: bool) -> Option<Typed> {
    let chars: Vec<char> = source.chars().collect();
    let at = at.min(chars.len());
    let start = chars[..at].iter().rposition(|ch| !is_ident(*ch, lua)).map_or(0, |index| index + 1);
    if start == at || chars[start].is_ascii_digit() {
        return None;
    }
    let mut end = at;
    while end < chars.len() && is_ident(chars[end], lua) {
        end += 1;
    }
    // the letters of a name, never the text of a comment or a string
    let code = editing::code_mask(&chars);
    if !(start..end).all(|index| code.get(index).copied().unwrap_or(false)) {
        return None;
    }
    Some(Typed {
        range: start..at,
        text: chars[start..at].iter().collect(),
        word: start..end,
    })
}

/// The names that start with what is typed: the catalog's (the region's own
/// syntax included) and the script's own words, the exact case first, then the
/// script's names before the catalog's kinds, alphabetically within each.
pub(super) fn candidates(lua: bool, source: &str, typed: &str, limit: usize) -> Vec<Item> {
    let catalog = super::syntax::catalog(lua);
    let lowered = typed.to_lowercase();
    let mut items: Vec<Item> = Vec::new();
    for entry in catalog.entries() {
        if !entry.name.starts_with(typed) && !entry.name.to_lowercase().starts_with(&lowered) {
            continue;
        }
        items.push(Item {
            text: entry.name.clone(),
            kind: Kind::of(&entry.group),
            detail: entry.signature.clone(),
            docs: entry.tooltip.clone(),
            lower: entry.name.to_lowercase(),
        });
    }
    for name in words(&source.chars().collect::<Vec<_>>(), lua) {
        // the letters being typed are not one of the choices: they are there
        if name == typed || catalog.entry(&name).is_some() {
            continue;
        }
        if !name.starts_with(typed) && !name.to_lowercase().starts_with(&lowered) {
            continue;
        }
        items.push(Item {
            lower: name.to_lowercase(),
            text: name,
            kind: Kind::Name,
            detail: String::new(),
            docs: String::new(),
        });
    }
    items.sort_by(|a, b| order(typed, a).cmp(&order(typed, b)));
    items.dedup_by(|a, b| a.text == b.text);
    items.truncate(limit);
    items
}

/// Where a name stands in the list: the exact word first, then the letters in
/// the same case, then the kind (the script's own names before the catalog's),
/// then the letters, so that `llSay` comes before `llSHA1String`.
fn order<'a>(typed: &str, item: &'a Item) -> (u8, Kind, &'a str, &'a str) {
    let case = if item.text == typed {
        0
    } else if item.text.starts_with(typed) {
        1
    } else {
        2
    };
    (case, item.kind, item.lower.as_str(), item.text.as_str())
}

/// The names the script writes, in the code (not in its comments or strings),
/// each once, by the letters they start with.
fn words(chars: &[char], lua: bool) -> Vec<String> {
    let code = editing::code_mask(chars);
    let mut found = std::collections::BTreeSet::new();
    let mut at = 0;
    while at < chars.len() {
        if !code[at] || !is_ident(chars[at], lua) || chars[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && is_ident(chars[at], lua) && code[at] {
            at += 1;
        }
        if at - start >= 2 {
            found.insert(chars[start..at].iter().collect::<String>());
        }
    }
    found.into_iter().collect()
}

/// The list of one field, kept between frames.
#[derive(Clone, Default)]
struct List {
    /// The word it was made for: where it starts and what is typed.
    token: Option<(usize, String)>,
    items: Vec<Item>,
    selected: usize,
    /// A letter of this word was typed: a caret that a click or a jump left in
    /// a word does not open the list by itself (it would take the window's
    /// Enter and Escape).
    opened: bool,
    /// Escape closed the list for that word: it opens again when the word
    /// changes.
    dismissed: Option<(usize, String)>,
    /// The field had the keyboard in the last frame (egui takes it away on
    /// Escape before the frame's widgets see the key).
    had_focus: bool,
}

impl List {
    fn shown(&self) -> bool {
        self.opened && !self.items.is_empty() && self.dismissed != self.token
    }
}

/// A key of the open list, taken before the field sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Nav {
    Up,
    Down,
    Pick,
    Dismiss,
}

fn list_id(field: Id) -> Id {
    field.with("completion")
}

/// Whether the reader typed or erased a letter this frame: the list belongs to
/// the typing, as a menu of suggestions does.
fn typing(ui: &egui::Ui) -> bool {
    ui.input(|input| {
        input.events.iter().any(|event| match event {
            egui::Event::Text(text) => !text.is_empty(),
            egui::Event::Key {
                key: egui::Key::Backspace | egui::Key::Delete,
                pressed: true,
                ..
            } => true,
            _ => false,
        })
    })
}

/// Before the field is drawn: while its list is shown, the arrows, Tab, Enter
/// and Escape are the list's (Enter does not break the line, Tab does not
/// indent). Nothing is taken while the list is not shown.
pub(super) fn keys(ui: &egui::Ui, field: Id) -> Option<Nav> {
    let list: List = ui.ctx().data(|data| data.get_temp(list_id(field)))?;
    if !list.shown() || !list.had_focus {
        return None;
    }
    let none = egui::Modifiers::NONE;
    let nav = ui.input_mut(|input| {
        if input.consume_key(none, egui::Key::ArrowUp) {
            Some(Nav::Up)
        } else if input.consume_key(none, egui::Key::ArrowDown) {
            Some(Nav::Down)
        } else if input.consume_key(none, egui::Key::Tab) || input.consume_key(none, egui::Key::Enter) {
            Some(Nav::Pick)
        } else if input.consume_key(none, egui::Key::Escape) {
            Some(Nav::Dismiss)
        } else {
            None
        }
    });
    if nav.is_some() {
        // the key was the list's: the field keeps the keyboard
        ui.ctx().memory_mut(|memory| memory.request_focus(field));
    }
    nav
}

/// After the field is drawn: the caret's word is completed, the key taken by
/// [`keys`] applied, its choice written in the source, and the list drawn
/// under the caret.
pub(super) fn after(ui: &egui::Ui, p: &Palette, lua: bool, field: Id, source: &mut String, output: &mut TextEditOutput, nav: Option<Nav>) {
    let mut list: List = ui.ctx().data(|data| data.get_temp(list_id(field))).unwrap_or_default();
    let caret = output
        .cursor_range
        .filter(|range| range.is_empty())
        .map(|range| range.primary.index.0);
    let word = caret
        .filter(|_| output.response.has_focus() && !ui.is_sizing_pass() && ui.is_enabled())
        .and_then(|at| typed(source, at, lua));
    let token = word
        .as_ref()
        .filter(|word| word.text.chars().count() >= MIN_TYPED)
        .map(|word| (word.range.start, word.text.clone()));
    if token != list.token {
        list.items = token
            .as_ref()
            .map(|(_, text)| candidates(lua, source, text, MOST))
            .unwrap_or_default();
        list.selected = 0;
        // a letter typed, or a letter erased: the list is the reader's
        list.opened = typing(ui);
        list.token = token;
    }
    if list.token.is_none() {
        list.dismissed = None;
    }
    let mut pick = None;
    match nav {
        Some(Nav::Up) if !list.items.is_empty() => list.selected = (list.selected + list.items.len() - 1) % list.items.len(),
        Some(Nav::Down) if !list.items.is_empty() => list.selected = (list.selected + 1) % list.items.len(),
        Some(Nav::Pick) => pick = Some(list.selected),
        Some(Nav::Dismiss) => list.dismissed = list.token.clone(),
        _ => {}
    }
    let drawn = list.shown() && pick.is_none();
    let anchor = caret.map(|at| {
        output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(at))
            .translate(output.galley_pos.to_vec2())
    });
    if drawn && let Some(anchor) = anchor {
        pick = draw(ui, p, anchor, &list);
    }
    if let (Some(index), Some(word)) = (pick, word) {
        if let Some(item) = list.items.get(index) {
            accept(ui, field, source, &word, item);
        }
        list = List::default();
    }
    list.had_focus = output.response.has_focus() || nav.is_some();
    ui.ctx().data_mut(|data| data.insert_temp(list_id(field), list));
}

/// Writes the chosen name over the word: a function brings its bracket, the
/// caret after it, so that its arguments can be typed at once.
fn accept(ui: &egui::Ui, field: Id, source: &mut String, word: &Typed, item: &Item) {
    let chars: Vec<char> = source.chars().collect();
    let mut text = item.text.clone();
    if item.kind == Kind::Function && chars.get(word.word.end).copied() != Some('(') {
        text.push('(');
    }
    let at = word.word.start + text.chars().count();
    if editing::replace(ui.ctx(), field, source, word.word.clone(), &text) {
        editing::select(ui.ctx(), field, at..at, false);
        ui.ctx().request_repaint();
    }
}

/// The first lines of a help, on one paragraph: a tooltip of the catalog runs
/// to many lines, and the list keeps the room for its rows.
fn brief(docs: &str) -> String {
    const LINES: usize = 3;
    let mut lines = docs.lines().filter(|line| !line.trim().is_empty());
    let mut text = lines.by_ref().take(LINES).collect::<Vec<_>>().join(" ");
    if lines.next().is_some() {
        text.push('…');
    }
    text
}

/// The list under the caret: a row per name, a lit one for the arrows' choice,
/// and under them the signature and the help of that one; the row clicked, if
/// any.
fn draw(ui: &egui::Ui, p: &Palette, anchor: egui::Rect, list: &List) -> Option<usize> {
    let mut picked = None;
    egui::Area::new(egui::Id::unique("script_completion_list"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(anchor.left_bottom() + egui::vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_min_width(300.0);
                ui.set_max_width(360.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("script_completion_rows")
                    .max_height(ROWS_HEIGHT)
                    .show(ui, |ui| {
                        for (index, item) in list.items.iter().enumerate() {
                            let row = menu::suggestion_row(ui, p, item.kind.icon(), &item.text, "", index == list.selected);
                            if index == list.selected {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if row.clicked() {
                                picked = Some(index);
                            }
                        }
                    });
                // what the one chosen asks for and does, as a hover says it
                if let Some(item) = list.items.get(list.selected)
                    && (!item.detail.is_empty() || !item.docs.is_empty())
                {
                    ui.add_space(4.0);
                    if !item.detail.is_empty() {
                        ui.label(RichText::new(&item.detail).font(egui::FontId::monospace(11.5)).color(p.ink));
                    }
                    if !item.docs.is_empty() {
                        ui.add(egui::Label::new(RichText::new(brief(&item.docs)).size(11.0).color(p.muted)).wrap());
                    }
                }
            });
        });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_is_the_name_before_the_caret_and_never_in_a_comment_or_a_string() {
        let source = "default { state_entry() { llSay(0, \"llSet\"); } } // llSet";
        // on the first letter of the call, then after its first letters
        let typed_at = |needle: &str, offset: usize| {
            let byte = source.find(needle).expect("the needle") + offset;
            typed(source, source[..byte].chars().count(), false)
        };
        let start = source.find("llSay(0").expect("the call");
        let call = typed_at("llSay(0", 3).expect("llS of the call");
        assert_eq!(
            (call.range, call.text.as_str(), call.word),
            (start..start + 3, "llS", start..start + 5)
        );
        // the caret at the end of a word that ends the source
        let end = typed_of("llSay", 5, false).expect("the end of a word");
        assert_eq!((end.range, end.text.as_str(), end.word), (0..5, "llSay", 0..5));
        // a name in a string, in a comment, a number and a lone letter
        let in_string = source.find("\"llSet").expect("the string") + 2;
        assert!(typed(source, source[..in_string].chars().count(), false).is_none());
        let in_comment = source.find("// llSet").expect("the comment") + 4;
        assert!(typed(source, source[..in_comment].chars().count(), false).is_none());
        assert!(typed_of("12", 2, false).is_none());
        assert!(typed_of("(", 1, false).is_none());
        // SLua keeps its library's dot in the name
        let lua = typed_of("ll.Sa", 5, true).expect("an SLua name");
        assert_eq!((lua.text.as_str(), lua.word), ("ll.Sa", 0..5));
    }

    fn typed_of(source: &str, at: usize, lua: bool) -> Option<Typed> {
        typed(source, at, lua)
    }

    #[test]
    fn the_catalog_offers_its_names_by_case_then_kind_then_letter() {
        let names = candidates(false, "", "llSay", MOST);
        assert_eq!(names.first().map(|item| item.text.as_str()), Some("llSay"));
        assert_eq!(names[0].kind, Kind::Function);
        assert!(names[0].detail.starts_with("void llSay("), "{}", names[0].detail);
        assert!(!names[0].docs.is_empty());
        assert!(names.iter().all(|item| item.text.to_lowercase().starts_with("llsay")));
        // a constant and a type, and the exact case before the rest
        let active = candidates(false, "", "ACT", MOST);
        assert!(active.iter().any(|item| item.text == "ACTIVE" && item.kind == Kind::Constant));
        let rotation = candidates(false, "", "rotatio", MOST);
        assert!(rotation.iter().any(|item| item.text == "rotation" && item.kind == Kind::Type));
    }

    #[test]
    fn the_scripts_own_names_come_first_and_only_once() {
        let source = "integer counter = 0;\ndefault { state_entry() { counter = counter + 1; llSay(0, \"\"); } }";
        let names = candidates(false, source, "cou", MOST);
        assert_eq!(names.first().map(|item| item.text.as_str()), Some("counter"));
        assert_eq!(names[0].kind, Kind::Name);
        assert!(names[0].detail.is_empty());
        // a catalog word used in the script is not offered twice
        let say = candidates(false, source, "llSa", MOST);
        assert_eq!(say.iter().filter(|item| item.text == "llSay").count(), 1);
        // a name inside a comment is not one of the script's words
        let commented = candidates(false, "// hiddenname\ndefault { }", "hidden", MOST);
        assert!(commented.iter().all(|item| item.text != "hiddenname"));
    }

    #[test]
    fn a_word_no_catalog_knows_is_offered_as_a_name() {
        let names = candidates(false, "myHelper();", "myH", MOST);
        assert_eq!(names.first().map(|item| item.text.as_str()), Some("myHelper"));
        assert_eq!(names[0].kind, Kind::Name);
    }

    #[test]
    fn the_help_under_the_list_keeps_three_lines() {
        assert_eq!(brief("One.\nTwo.\nThree."), "One. Two. Three.");
        assert_eq!(brief("One.\n\nTwo.\nThree.\nFour."), "One. Two. Three.…");
        assert_eq!(brief(""), "");
    }
}
