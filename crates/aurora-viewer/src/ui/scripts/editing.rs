//! LLScriptEdCore::performAction, LLTextEditor selection / search (Firestorm,
//! LGPL 2.1). Commands share egui's cursor and undo history with keyboard edits.

use egui::{
    Id, TextBuffer,
    text::{CCursor, CCursorRange, CharIndex},
};
use std::ops::Range;

#[derive(Clone, Copy)]
pub(super) enum SelectionCommand {
    Indent,
    Outdent,
    LineComment,
    BlockComment,
}

struct Replacement {
    bytes: Range<usize>,
    text: String,
}

fn selected_lines(source: &str, range: Range<usize>) -> Range<usize> {
    let start = source[..range.start].rfind('\n').map_or(0, |n| n + 1);
    let last = if range.end > range.start {
        source[..range.end].char_indices().next_back().map_or(range.start, |(i, _)| i)
    } else {
        range.end
    };
    let end = source[last..].find('\n').map_or(source.len(), |n| last + n);
    start..end
}

fn replacements(source: &str, selection: Range<usize>, command: SelectionCommand, scope: Range<usize>) -> Option<Vec<Replacement>> {
    let lines = selected_lines(source, selection.clone());
    if matches!(command, SelectionCommand::BlockComment) {
        let empty = selection.is_empty();
        let at = selection.start;
        let mut range = if empty { lines } else { selection };
        let cursor = if empty { at } else { range.start };
        if let Some(comment) = super::syntax::block_comment_at(&source[scope.clone()], cursor - scope.start)
            .map(|range| scope.start + range.start..scope.start + range.end)
            && (empty || range.end <= comment.end)
        {
            range = comment;
        }
        let text = &source[range.clone()];
        let trimmed = text.trim();
        if trimmed.starts_with("/*") && trimmed.ends_with("*/") && trimmed.len() >= 4 {
            let start = range.start + text.len() - text.trim_start().len();
            let end = range.end - (text.len() - text.trim_end().len());
            return Some(vec![
                Replacement {
                    bytes: start..start + 2,
                    text: String::new(),
                },
                Replacement {
                    bytes: end - 2..end,
                    text: String::new(),
                },
            ]);
        }
        if range.start >= 2 && source[..range.start].ends_with("/*") && source[range.end..].starts_with("*/") {
            range.start -= 2;
            range.end += 2;
            return Some(vec![
                Replacement {
                    bytes: range.start..range.start + 2,
                    text: String::new(),
                },
                Replacement {
                    bytes: range.end - 2..range.end,
                    text: String::new(),
                },
            ]);
        }
        // LSL block comments do not nest. Refuse an early terminator rather
        // than making the rest of the selected code unexpectedly executable.
        if text.contains("/*") || text.contains("*/") {
            return None;
        }
        return Some(vec![
            Replacement {
                bytes: range.start..range.start,
                text: "/*".into(),
            },
            Replacement {
                bytes: range.end..range.end,
                text: "*/".into(),
            },
        ]);
    }
    let mut offset = lines.start;
    let line_ranges: Vec<_> = source[lines]
        .split('\n')
        .map(|line| {
            let start = offset;
            offset += line.len() + 1;
            (start, line)
        })
        .collect();
    let uncomment = line_ranges
        .iter()
        .filter(|(_, l)| !l.trim().is_empty())
        .all(|(_, l)| l.trim_start().starts_with("//"));
    let mut edits = Vec::new();
    for (start, line) in line_ranges {
        let indent = line.bytes().take_while(|b| matches!(b, b' ' | b'\t')).count();
        match command {
            SelectionCommand::Indent => edits.push(Replacement {
                bytes: start..start,
                text: "    ".into(),
            }),
            SelectionCommand::Outdent => {
                let count = if line.starts_with('\t') {
                    1
                } else {
                    line.bytes().take_while(|b| *b == b' ').take(4).count()
                };
                if count != 0 {
                    edits.push(Replacement {
                        bytes: start..start + count,
                        text: String::new(),
                    });
                }
            }
            SelectionCommand::LineComment => {
                if line.trim().is_empty() && !selection.is_empty() {
                    continue;
                }
                let at = start + indent;
                if uncomment && line[indent..].starts_with("//") {
                    let count = if line[indent..].starts_with("// ") { 3 } else { 2 };
                    edits.push(Replacement {
                        bytes: at..at + count,
                        text: String::new(),
                    });
                } else {
                    edits.push(Replacement {
                        bytes: at..at,
                        text: "// ".into(),
                    });
                }
            }
            SelectionCommand::BlockComment => unreachable!("handled above"),
        }
    }
    Some(edits)
}

/// One history entry for the whole operation, including its final selection.
/// Keeping both cursors also preserves backward selections and repeated Tab.
pub(super) fn selection_command(ctx: &egui::Context, id: Id, source: &mut String, command: SelectionCommand) -> bool {
    selection_command_in(ctx, id, source, command, 0..source.len())
}

fn selection_command_in(ctx: &egui::Context, id: Id, source: &mut String, command: SelectionCommand, scope: Range<usize>) -> bool {
    let before = cursor(ctx, id);
    let selection = bytes(source, before);
    let Some(edits) = replacements(source, selection.clone(), command, scope) else {
        return false;
    };
    if edits.is_empty() {
        return true;
    }
    let mut updated = source.clone();
    for edit in edits.iter().rev() {
        updated.replace_range(edit.bytes.clone(), &edit.text);
    }
    if updated.len() > aurora_net::scripts::MAX_SOURCE {
        return false;
    }
    let map = |mut cursor: CCursor| {
        let at = source.byte_index_from_char_index(cursor.index).0;
        let mut delta = 0_isize;
        for edit in &edits {
            let start = source[..edit.bytes.start].chars().count();
            let removed = source[edit.bytes.clone()].chars().count();
            let added = edit.text.chars().count();
            if at >= edit.bytes.end {
                delta += added as isize - removed as isize;
            } else if at >= edit.bytes.start {
                cursor.index = CharIndex(start + added);
                break;
            }
        }
        cursor.index = CharIndex(cursor.index.0.saturating_add_signed(delta));
        cursor
    };
    let mut after = CCursorRange {
        primary: map(before.primary),
        secondary: map(before.secondary),
        ..before
    };
    if !selection.is_empty() && matches!(command, SelectionCommand::BlockComment) && edits.first().is_some_and(|e| e.text == "/*") {
        // Keep the delimiters selected so repeating the shortcut removes the
        // same wrapper, also when the selection was made backwards.
        if before.primary.index < before.secondary.index {
            after.primary.index = before.primary.index;
        } else {
            after.secondary.index = before.secondary.index;
        }
    }
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let mut undo = state.undoer();
    undo.add_undo(&(before, source.clone()));
    *source = updated;
    state.cursor.set_char_range(Some(after));
    undo.add_undo(&(after, source.clone()));
    state.set_undoer(undo);
    state.store(ctx, id);
    ctx.data_mut(|d| d.insert_temp(Id::unique((id, "history_dirty")), true));
    ctx.memory_mut(|m| m.request_focus(id));
    true
}

/// Commands run at an input boundary. If native cursor movement / typing came
/// first, defer the suffix to the next frame so it uses egui's actual cursor.
pub(super) fn keyboard(ui: &mut egui::Ui, id: Id, source: &mut String, scope: &dyn Fn(&str, usize) -> Option<Range<usize>>) -> bool {
    if !ui.memory(|m| m.has_focus(id)) {
        return false;
    }
    let queue_id = id.with("selection_keys");
    let mut events = ui
        .ctx()
        .data_mut(|d| d.remove_temp::<Vec<egui::Event>>(queue_id))
        .unwrap_or_default();
    events.extend(ui.input_mut(|i| std::mem::take(&mut i.events)));
    let mut native = Vec::new();
    let mut changed = false;
    let mut events = events.into_iter();
    while let Some(event) = events.next() {
        let candidate = match &event {
            egui::Event::Key {
                key: egui::Key::K,
                pressed: true,
                modifiers,
                ..
            } if modifiers.command && !modifiers.alt => Some(if modifiers.shift {
                SelectionCommand::BlockComment
            } else {
                SelectionCommand::LineComment
            }),
            egui::Event::Key {
                key: egui::Key::Slash,
                pressed: true,
                modifiers,
                ..
            } if modifiers.command && !modifiers.alt => Some(SelectionCommand::LineComment),
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => Some(if modifiers.shift {
                SelectionCommand::Outdent
            } else {
                SelectionCommand::Indent
            }),
            _ => None,
        };
        if let Some(command) = candidate {
            let selected = bytes(source, cursor(ui.ctx(), id));
            let selection_input = native.iter().any(|e| {
                matches!(e,
                    egui::Event::Key { modifiers, pressed: true, .. } if modifiers.shift || modifiers.command
                ) || matches!(e, egui::Event::PointerButton { pressed: true, .. })
            });
            if matches!(command, SelectionCommand::Indent) && !source[selected.clone()].contains('\n') && !selection_input {
                native.push(event);
                continue;
            }
            if native.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            }) {
                let tail = std::iter::once(event).chain(events).collect::<Vec<_>>();
                ui.ctx().data_mut(|d| d.insert_temp(queue_id, tail));
                ui.ctx().request_repaint();
                break;
            }
            let language = scope(source, selected.start).filter(|range| selected.end <= range.end);
            if matches!(command, SelectionCommand::LineComment | SelectionCommand::BlockComment) && language.is_none() {
                native.push(event);
                continue;
            }
            if selection_command_in(ui.ctx(), id, source, command, language.unwrap_or(0..source.len())) {
                changed = true;
            } else {
                ui.ctx().data_mut(|d| d.insert_temp(id.with("selection_error"), true));
            }
        } else {
            native.push(event);
        }
    }
    ui.input_mut(|i| i.events = native);
    changed
}

pub(super) fn cursor(ctx: &egui::Context, id: Id) -> CCursorRange {
    egui::TextEdit::load_state(ctx, id)
        .and_then(|s| s.cursor.char_range())
        .unwrap_or_default()
}

pub(super) fn select(ctx: &egui::Context, id: Id, range: Range<usize>, focus: bool) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end))));
    state.store(ctx, id);
    if focus {
        ctx.memory_mut(|m| m.request_focus(id));
    }
}

pub(super) fn bytes(source: &str, range: CCursorRange) -> Range<usize> {
    let [start, end] = range.sorted_cursors();
    source.byte_index_from_char_index(start.index).0..source.byte_index_from_char_index(end.index).0
}

pub(super) fn replace(ctx: &egui::Context, id: Id, source: &mut String, range: Range<usize>, text: &str) -> bool {
    let start = source.byte_index_from_char_index(CharIndex(range.start)).0;
    let end = source.byte_index_from_char_index(CharIndex(range.end)).0;
    if source.len() - (end - start) + text.len() > aurora_net::scripts::MAX_SOURCE {
        return false;
    }
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let mut undo = state.undoer();
    undo.add_undo(&(cursor(ctx, id), source.clone()));
    source.replace_range(start..end, text);
    let at = CCursorRange::one(CCursor::new(range.start + text.chars().count()));
    state.cursor.set_char_range(Some(at));
    undo.add_undo(&(at, source.clone()));
    state.set_undoer(undo);
    state.store(ctx, id);
    ctx.data_mut(|d| d.insert_temp(Id::unique((id, "history_dirty")), true));
    true
}

pub(super) fn undo(ctx: &egui::Context, id: Id, source: &mut String, redo: bool) {
    let Some(mut state) = egui::TextEdit::load_state(ctx, id) else {
        return;
    };
    let current = (cursor(ctx, id), source.clone());
    let mut undo = state.undoer();
    let next = if redo { undo.redo(&current) } else { undo.undo(&current) };
    if let Some((range, text)) = next {
        source.clone_from(text);
        state.cursor.set_char_range(Some(*range));
    }
    state.set_undoer(undo);
    state.store(ctx, id);
    ctx.data_mut(|d| d.insert_temp(Id::unique((id, "history_dirty")), true));
}

pub(super) fn position(source: &str, at: usize) -> (usize, usize) {
    let byte = source.byte_index_from_char_index(CharIndex(at)).0;
    let prefix = &source[..byte];
    (
        prefix.bytes().filter(|c| *c == b'\n').count(),
        prefix.rsplit('\n').next().unwrap_or_default().chars().count(),
    )
}

pub(super) fn line_cursor(source: &str, line: usize, column: usize) -> usize {
    let mut at = 0;
    for (n, text) in source.split('\n').enumerate() {
        if n == line {
            return at + column.min(text.trim_end_matches('\r').chars().count());
        }
        at += text.chars().count() + 1;
    }
    source.chars().count()
}

pub(super) fn error_location(error: &str) -> Option<(usize, usize)> {
    let text = error.trim().strip_prefix('(')?.split_once(')')?.0;
    let (line, column) = text.split_once(',')?;
    Some((line.trim().parse().ok()?, column.trim().parse().ok()?))
}

/// The pair of brackets to light up for a caret at char `at`: the bracket just
/// before it, or the one at it, and its match (LLScriptEditor::highlightMatchingBracket,
/// indra/newview/llpreviewscript.cpp). A bracket in a line or block comment,
/// or in a quoted string, is not one of the code's.
pub(super) fn bracket_match(source: &str, at: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = source.chars().collect();
    let code = code_mask(&chars);
    let index = if at > 0 && kind(chars[at - 1]).is_some() && code[at - 1] {
        at - 1
    } else if at < chars.len() && kind(chars[at]).is_some() && code[at] {
        at
    } else {
        return None;
    };
    let (open, close, forward) = kind(chars[index])?;
    let (step, wanted) = if forward { (1isize, close) } else { (-1isize, open) };
    let mut depth = 0usize;
    let mut i = index as isize;
    loop {
        i += step;
        if i < 0 || i as usize >= chars.len() {
            return None;
        }
        let at = i as usize;
        if !code[at] {
            continue;
        }
        let ch = chars[at];
        if ch == chars[index] {
            depth += 1;
        } else if ch == wanted {
            if depth == 0 {
                return Some((index.min(at), index.max(at)));
            }
            depth -= 1;
        }
    }
}

/// The bracket a character is: its opening and closing forms, and whether it
/// opens.
fn kind(ch: char) -> Option<(char, char, bool)> {
    Some(match ch {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => ('(', ')', false),
        ']' => ('[', ']', false),
        '}' => ('{', '}', false),
        _ => return None,
    })
}

/// Which characters are code: not in a `//` line comment, a `/* */` block
/// comment or a quoted string (`"` or `'`, a backslash escaping the next
/// character).
pub(super) fn code_mask(chars: &[char]) -> Vec<bool> {
    let mut code = vec![true; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                code[i] = false;
                i += 1;
            }
        } else if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            code[i] = false;
            code[i + 1] = false;
            i += 2;
            while i < chars.len() {
                code[i] = false;
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    code[i + 1] = false;
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else if matches!(chars[i], '"' | '\'') {
            let quote = chars[i];
            code[i] = false;
            i += 1;
            while i < chars.len() {
                code[i] = false;
                if chars[i] == '\\' {
                    if i + 1 < chars.len() {
                        code[i + 1] = false;
                    }
                    i += 2;
                    continue;
                }
                if chars[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    code
}

/// The characters a diagnostic's line and column point at: the word there (or
/// the next one on the line, when the column falls on a space), or the line's
/// last character when it points past its text. LSL errors give their place,
/// not their length.
pub(super) fn error_span(source: &str, line: usize, column: usize) -> std::ops::Range<usize> {
    let chars: Vec<char> = source.chars().collect();
    let start = line_cursor(source, line, column);
    let line_end = chars[start.min(chars.len())..]
        .iter()
        .position(|ch| *ch == '\n')
        .map_or(chars.len(), |at| start + at);
    let word = |ch: char| ch == '_' || ch.is_alphanumeric();
    // the word the place is in (a column inside it lights all of it), or the
    // next one on the line when the column falls on a space
    let begin = if start < line_end && word(chars[start]) {
        let mut begin = start;
        while begin > 0 && word(chars[begin - 1]) {
            begin -= 1;
        }
        begin
    } else {
        match (start..line_end).find(|at| word(chars[*at])) {
            Some(begin) => begin,
            None => {
                let end = line_end.max(start);
                // an empty line has nothing to underline; a column past a
                // line's text takes its last character
                if end > 0 && chars[end - 1] == '\n' {
                    return end..end;
                }
                return end.saturating_sub(1)..end;
            }
        }
    };
    let end = (begin..line_end).find(|at| !word(chars[*at])).unwrap_or(line_end);
    begin..end
}

/// The word under the caret (its range) and every place it appears as a whole
/// word, for the boxes of its other occurrences. A number, a keyword or a
/// built-in name is not one: they are everywhere.
pub(super) fn occurrences(source: &str, at: usize, lua: bool) -> Option<(std::ops::Range<usize>, Vec<std::ops::Range<usize>>)> {
    let word = self::word(source, at, lua);
    if word.is_empty() || word.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if word.chars().count() < 2 || crate::lsl_keywords::current().entry(word).is_some() {
        return None;
    }
    let found = matches_filtered(source, word, false, true);
    if found.len() < 2 {
        return None;
    }
    // the occurrence the caret is in, or the one ending at it
    let mine = found.iter().position(|range| range.contains(&at) || range.end == at).unwrap_or(0);
    Some((found[mine].clone(), found))
}

/// The word around `at`; an SLua word keeps its library (`ll.Say`).
pub(super) fn word(source: &str, at: usize, lua: bool) -> &str {
    let byte = source.byte_index_from_char_index(CharIndex(at)).0;
    let is_word = |c: char| c == '_' || c.is_alphanumeric() || (lua && c == '.');
    let start = source[..byte]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_word(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = source[byte..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(source.len(), |(i, _)| byte + i);
    &source[start..end]
}

/// Unicode lowercase may change byte length: keep original character ranges.
pub(crate) fn matches(source: &str, needle: &str, insensitive: bool) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    if !insensitive {
        let mut previous = 0;
        let mut chars = 0;
        return source
            .match_indices(needle)
            .map(|(byte, text)| {
                chars += source[previous..byte].chars().count();
                let start = chars;
                chars += text.chars().count();
                previous = byte + text.len();
                start..chars
            })
            .collect();
    }
    let mut folded = String::new();
    let mut boundaries = Vec::new();
    for (n, c) in source.chars().enumerate() {
        boundaries.push((folded.len(), n));
        folded.extend(c.to_lowercase());
    }
    boundaries.push((folded.len(), source.chars().count()));
    let needle = needle.to_lowercase();
    folded
        .match_indices(&needle)
        .filter_map(|(byte, text)| {
            let start = boundaries.binary_search_by_key(&byte, |b| b.0).ok()?;
            let end = boundaries.binary_search_by_key(&(byte + text.len()), |b| b.0).ok()?;
            Some(boundaries[start].1..boundaries[end].1)
        })
        .collect()
}

/// Optional whole-word filtering shared by script and notecard Find / Replace.
/// Character offsets, not UTF-8 byte offsets, match TextEdit's cursor model.
/// Underscores, punctuation and whitespace delimit words for Find / Replace.
pub(crate) fn matches_filtered(source: &str, needle: &str, insensitive: bool, whole_words: bool) -> Vec<Range<usize>> {
    let found = matches(source, needle, insensitive);
    if !whole_words {
        return found;
    }
    let chars: Vec<char> = source.chars().collect();
    let word_char = |c: char| c.is_alphanumeric();
    found
        .into_iter()
        .filter(|r| (r.start == 0 || !word_char(chars[r.start - 1])) && (r.end == chars.len() || !word_char(chars[r.end])))
        .collect()
}

pub(crate) fn next_match(ranges: &[Range<usize>], selection: Range<usize>, up: bool) -> Option<Range<usize>> {
    if up {
        ranges
            .iter()
            .rev()
            .find(|r| r.end <= selection.start)
            .or_else(|| ranges.last())
            .cloned()
    } else {
        ranges
            .iter()
            .find(|r| r.start >= selection.end && **r != selection)
            .or_else(|| ranges.first())
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bracket_at_the_caret_matches_its_pair_across_nesting() {
        let text = "default { if (a) { b(); } }";
        // caret after the outer `{` (char 9)
        assert_eq!(bracket_match(text, 9), Some((8, 26)));
        // caret before the `(`: the bracket after it
        let chars: Vec<char> = text.chars().collect();
        let at = chars.iter().position(|c| *c == '(').unwrap();
        assert_eq!(bracket_match(text, at), Some((at, at + 2)));
    }

    #[test]
    fn a_bracket_in_a_string_or_a_comment_is_not_matched() {
        let text = "llSay(0, \"(\"); // )\n/* ] */ x = 1;\n";
        // the `(` of llSay matches its own pair
        assert_eq!(bracket_match(text, 6), Some((5, 12)));
        // the `(` inside the string, the `)` of the line comment and the `]`
        // of the block comment are not brackets of the code
        assert_eq!(bracket_match(text, 11), None);
        assert_eq!(bracket_match(text, 19), None);
        assert_eq!(bracket_match(text, 24), None);
        // an unmatched bracket finds nothing
        assert_eq!(bracket_match("(a", 1), None);
        assert_eq!(bracket_match("a)", 1), None);
        // no bracket at the caret
        assert_eq!(bracket_match("abc", 1), None);
    }

    #[test]
    fn a_diagnostics_span_is_the_word_at_its_place_or_the_next_one() {
        let text = "default\n{\n    llSay(0, message);\n}\n";
        // a column inside `llSay` lights the whole word
        assert_eq!(error_span(text, 2, 5), 14..19);
        // a column on a space takes the next word
        assert_eq!(error_span(text, 2, 2), 14..19);
        // past the text of a line: its last character
        assert_eq!(error_span(text, 0, 40), 6..7);
        // an empty line points at nothing of it
        assert_eq!(error_span("a\n\nb\n", 1, 0), 2..2);
    }

    #[test]
    fn occurrences_leave_keywords_numbers_and_lone_words_alone() {
        let text = "integer alpha = 0;\nalpha = alpha + 1;\n";
        let (mine, found) = occurrences(text, 8, false).expect("alpha appears three times");
        assert_eq!(mine, 8..13);
        assert_eq!(found.len(), 3);
        // from the end of a word, it is that word
        let (mine, _) = occurrences(text, 13, false).expect("the caret at alpha's end");
        assert_eq!(mine, 8..13);
        // `integer` is a keyword, `0` a number, `x` alone
        assert_eq!(occurrences(text, 2, false), None);
        assert_eq!(occurrences("x = 0;\nx = 1;\n", 0, false), None);
        assert_eq!(occurrences("0 = 0;\n", 0, false), None);
        // a word that appears once has no other occurrence
        assert_eq!(occurrences("integer alpha = 0;\n", 8, false), None);
    }

    #[test]
    fn whole_words_exclude_substrings_but_accept_punctuation_delimiters() {
        let text = "This hi high hi_there hi-there (hi) \u{E9}hi hi\u{E9}";
        let found = matches_filtered(text, "hi", true, true);
        let substrings: Vec<String> = found
            .iter()
            .map(|range| text.chars().skip(range.start).take(range.len()).collect())
            .collect();
        assert_eq!(substrings, vec!["hi", "hi", "hi", "hi"]);
        // The original partial-word search still includes "hi" inside "This".
        assert!(matches_filtered(text, "hi", true, false).len() > found.len());
        assert_eq!(matches_filtered("HI hi Hi", "hi", false, true), vec![3..5]);
        assert_eq!(matches_filtered("HI hi Hi", "hi", true, true), vec![0..2, 3..5, 6..8]);
    }

    #[test]
    fn multiline_indent_keeps_backward_selection_and_excludes_the_final_line_at_column_zero() {
        let ctx = egui::Context::default();
        let id = Id::unique("lines");
        let original = "éあ\r\n  second\r\nthird";
        let mut source = original.to_owned();
        let end = original[..original.find("third").expect("third")].chars().count();
        select(&ctx, id, end..0, false);
        assert!(selection_command(&ctx, id, &mut source, SelectionCommand::Indent));
        assert_eq!(source, "    éあ\r\n      second\r\nthird");
        assert!(cursor(&ctx, id).primary.index < cursor(&ctx, id).secondary.index);
        assert!(selection_command(&ctx, id, &mut source, SelectionCommand::Outdent));
        assert_eq!(source, original);
        undo(&ctx, id, &mut source, false);
        assert_eq!(source, "    éあ\r\n      second\r\nthird");
        undo(&ctx, id, &mut source, false);
        assert_eq!(source, original);
    }

    #[test]
    fn outdent_handles_literal_tabs_short_indentation_and_unindented_code() {
        let ctx = egui::Context::default();
        let id = Id::unique("outdent");
        let mut source = "\tfirst\n  second\nthird\n".to_owned();
        let len = source.chars().count();
        select(&ctx, id, 0..len, false);
        selection_command(&ctx, id, &mut source, SelectionCommand::Outdent);
        assert_eq!(source, "first\nsecond\nthird\n");
    }

    #[test]
    fn line_comments_toggle_current_or_selected_lines_and_restore_spacing_exactly() {
        let ctx = egui::Context::default();
        let id = Id::unique("comments");
        let original = "    éあ\r\n\t// existing\r\n\r\nlast";
        let mut source = original.to_owned();
        let end = original[..original.find("last").expect("last")].chars().count();
        select(&ctx, id, 0..end, false);
        selection_command(&ctx, id, &mut source, SelectionCommand::LineComment);
        assert_eq!(source, "    // éあ\r\n\t// // existing\r\n\r\nlast");
        selection_command(&ctx, id, &mut source, SelectionCommand::LineComment);
        assert_eq!(source, original);
        let at = source.chars().count();
        select(&ctx, id, at..at, false);
        selection_command(&ctx, id, &mut source, SelectionCommand::LineComment);
        assert!(source.ends_with("// last"));
        selection_command(&ctx, id, &mut source, SelectionCommand::LineComment);
        assert_eq!(source, original);
    }

    #[test]
    fn indented_block_comments_toggle_at_the_caret_and_keep_surrounding_whitespace() {
        let ctx = egui::Context::default();
        let id = Id::unique("indented_comment");
        let mut source = "    /* code */ tail".to_owned();
        select(&ctx, id, 8..8, false);
        assert!(selection_command(&ctx, id, &mut source, SelectionCommand::BlockComment));
        assert_eq!(source, "     code  tail");
        let mut source = "\t/*\néあ\n*/\r\n".to_owned();
        let len = source.chars().count();
        select(&ctx, id, 0..len, false);
        assert!(selection_command(&ctx, id, &mut source, SelectionCommand::BlockComment));
        assert_eq!(source, "\t\néあ\n\r\n");
    }

    #[test]
    fn block_comment_wraps_selection_and_toggles_from_inside_without_nesting() {
        let ctx = egui::Context::default();
        let id = Id::unique("block");
        let original = "éあ\n  code;\nlast";
        let mut source = original.to_owned();
        select(&ctx, id, 0..10, false);
        selection_command(&ctx, id, &mut source, SelectionCommand::BlockComment);
        assert_eq!(source, "/*éあ\n  code;*/\nlast");
        selection_command(&ctx, id, &mut source, SelectionCommand::BlockComment);
        assert_eq!(source, original);
        let mut source = "/*\n  code;\n*/".into();
        select(&ctx, id, 5..5, false);
        selection_command(&ctx, id, &mut source, SelectionCommand::BlockComment);
        assert_eq!(source, "\n  code;\n");
        let mut source = "code; /* existing */ rest".to_owned();
        let len = source.chars().count();
        select(&ctx, id, 0..len, false);
        assert!(!selection_command(&ctx, id, &mut source, SelectionCommand::BlockComment));
        assert_eq!(source, "code; /* existing */ rest");
    }

    #[test]
    fn command_edits_share_native_undo_and_redo_and_reject_oversized_input() {
        let ctx = egui::Context::default();
        let id = Id::unique("command_edit");
        let mut source = "éあ start".to_owned();
        select(&ctx, id, 2..8, false);
        assert!(replace(&ctx, id, &mut source, 2..8, " finish"));
        assert_eq!(source, "éあ finish");
        undo(&ctx, id, &mut source, false);
        assert_eq!(source, "éあ start");
        assert_eq!(
            cursor(&ctx, id).as_sorted_char_range(),
            egui::text::CharIndex(2)..egui::text::CharIndex(8)
        );
        undo(&ctx, id, &mut source, true);
        assert_eq!(source, "éあ finish");
        let before = source.clone();
        assert!(!replace(&ctx, id, &mut source, 0..0, &"x".repeat(aurora_net::scripts::MAX_SOURCE)));
        assert_eq!(source, before);
    }
    #[test]
    fn search_ranges_wrap_and_handle_unicode_case_expansion() {
        let ranges = matches("Été été ÉTÉ", "été", true);
        assert_eq!(ranges, vec![0..3, 4..7, 8..11]);
        assert_eq!(next_match(&ranges, 8..11, false), Some(0..3));
        assert_eq!(next_match(&ranges, 0..3, true), Some(8..11));
        assert!(matches("İ", "i", true).is_empty());
        assert_eq!(matches("İ", "i\u{307}", true), vec![0..1]);
        assert!(matches("abc", "", true).is_empty());
    }
    #[test]
    fn navigation_clamps_lines_and_columns_and_parses_diagnostics() {
        assert_eq!(position("é\r\nあx", 4), (1, 1));
        assert_eq!(line_cursor("é\r\nあx", 1, 1), 4);
        assert_eq!(line_cursor("é\r\nあx", 0, 99), 1);
        assert_eq!(line_cursor("é\r\nあx", 99, 0), 5);
        assert_eq!(error_location("(3, 7) : ERROR : Syntax error"), Some((3, 7)));
        assert_eq!(error_location("network error"), None);
        assert_eq!(word("é llSay(0)", 4, false), "llSay");
        assert_eq!(word("x = ll.Say(0)", 8, true), "ll.Say");
    }
}
