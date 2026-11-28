//! Finding a word in the scripts of one window, and where a word is declared:
//! the dock's Search tab, and Ctrl+click's go-to-definition across the scripts
//! of a linkset. Both scan the text as it is edited (not the processed view),
//! through the same matching and outline passes as the rest of the editor.

use super::Lang;
use super::editing;
use super::outline::{self, Kind};
use aurora_net::scripts::ScriptTarget;

/// One hit of the dock's Search tab.
pub(super) struct Hit {
    /// The script's place in the window's list of documents, when it has one:
    /// a script searched for the object scope has a target only.
    pub document: Option<usize>,
    /// The script it is in.
    pub target: ScriptTarget,
    pub name: String,
    /// The line and the column it starts at, counted as the source is.
    pub line: usize,
    pub column: usize,
    /// The document's first line number: 0 for LSL, 1 for SLua.
    pub first: usize,
    /// The line as it reads, trimmed, for the row.
    pub text: String,
}

/// How many hits the dock keeps: past this, the query is too broad to read.
pub(super) const MAX_HITS: usize = 200;

/// The hits of `query` in the window's scripts (`(document, target, name,
/// source)`), in the window's order and the text's, with the case and
/// whole-word options the tab keeps, at most [`MAX_HITS`]. `document` is none
/// for a script the object scope read out of its prim. A query of one
/// character is not one.
pub(super) fn find(
    scripts: &[(Option<usize>, ScriptTarget, &str, &str, Lang)],
    query: &str,
    insensitive: bool,
    whole_words: bool,
) -> Vec<Hit> {
    let needle = query.trim();
    if needle.chars().count() < 2 {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (document, target, name, source, lang) in scripts {
        for (line, text) in source.lines().enumerate() {
            for range in editing::matches_filtered(text, needle, insensitive, whole_words) {
                hits.push(Hit {
                    document: *document,
                    target: *target,
                    name: (*name).to_owned(),
                    line,
                    column: range.start,
                    first: usize::from(lang.is_lua()),
                    text: text.trim().to_owned(),
                });
                if hits.len() >= MAX_HITS {
                    return hits;
                }
            }
        }
    }
    hits
}

/// One place a name is used, for the References tab: the whole name in code,
/// not in a comment or a string, ready to be renamed.
pub(super) struct Reference {
    /// The script's place in the window's list of documents.
    pub document: usize,
    pub name: String,
    pub line: usize,
    pub column: usize,
    /// The document's first line number: 0 for LSL, 1 for SLua.
    pub first: usize,
    /// The line as it reads, trimmed, for the row.
    pub text: String,
    /// The name in the document's own characters, to rename it.
    pub range: std::ops::Range<usize>,
}

/// Every use of `word` in the window's scripts (`(document, name, source,
/// lua)`), the whole name once each, case-sensitively, in the code only: an
/// occurrence in a comment or a string is not a reference to rename, and a
/// longer name that holds the word is not one either.
pub(super) fn references(scripts: &[(usize, &str, &str, Lang)], word: &str) -> Vec<Reference> {
    let needle = word.trim();
    let name = needle.chars().all(|ch| ch == '_' || ch.is_alphanumeric());
    if needle.chars().count() < 2 || !name {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (document, script, source, lang) in scripts {
        let code = super::syntax::code_chars(source, *lang);
        let mut start = 0;
        for (line, text) in source.lines().enumerate() {
            for range in editing::matches_filtered(text, needle, false, true) {
                let at = start + range.start..start + range.end;
                if !at.clone().all(|index| code.get(index).copied().unwrap_or(false)) {
                    continue;
                }
                found.push(Reference {
                    document: *document,
                    name: (*script).to_owned(),
                    line,
                    column: range.start,
                    first: usize::from(lang.is_lua()),
                    text: text.trim().to_owned(),
                    range: at,
                });
                if found.len() >= MAX_HITS {
                    return found;
                }
            }
            start += text.chars().count() + 1;
        }
    }
    found
}

/// Where `word` is declared among the window's scripts (`(document, source,
/// lua)`): the document's place in the list, the line, and the column of its
/// name. The script the caret is in is searched first, then the others in the
/// window's order; only the outlines' states, events, functions and globals
/// count, so a keyword or a built-in name has no declaration.
pub(super) fn definition(scripts: &[(usize, &str, Lang)], word: &str, current: usize) -> Option<(usize, usize, usize)> {
    if word.is_empty() {
        return None;
    }
    let mut order: Vec<&(usize, &str, Lang)> = scripts.iter().collect();
    order.sort_by_key(|(document, ..)| *document != current);
    for (document, source, lang) in order {
        for item in outline::outline(source, *lang) {
            if item.name != word || !matches!(item.kind, Kind::State | Kind::Event | Kind::Function | Kind::Global) {
                continue;
            }
            let column = source
                .lines()
                .nth(item.line)
                .and_then(|line| line.find(word).map(|byte| line[..byte].chars().count()))
                .unwrap_or(0);
            return Some((*document, item.line, column));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_query_lists_its_hits_in_the_windows_and_the_texts_order() {
        let door = "integer alpha = 0;\nllSay(0, \"alpha\");\n";
        let panel = "// alpha twice: alpha\n";
        let panel_target = ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(1),
        };
        let door_target = ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(2),
        };
        let scripts = [
            (Some(0usize), panel_target, "Panel Script", panel, Lang::Lsl),
            (Some(1), door_target, "Door Script", door, Lang::Lsl),
        ];
        let hits = find(&scripts, "alpha", true, false);
        let rows: Vec<(Option<usize>, usize, usize)> = hits.iter().map(|hit| (hit.document, hit.line, hit.column)).collect();
        assert_eq!(rows, vec![(Some(0), 0, 3), (Some(0), 0, 16), (Some(1), 0, 8), (Some(1), 1, 10)]);
        assert_eq!(hits[0].name, "Panel Script");
        assert_eq!(hits[2].text, "integer alpha = 0;");
        // case-insensitive, and trimmed
        assert_eq!(find(&scripts, "  ALPHA ", true, false).len(), 4);
        // one character is not a query
        assert!(find(&scripts, "a", true, false).is_empty());
    }

    #[test]
    fn the_hits_stop_at_the_limit() {
        let source = "x\n".repeat(MAX_HITS + 10);
        let target = ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(4),
        };
        let scripts = [(None, target, "Loop", source.as_str(), Lang::Lsl)];
        assert_eq!(find(&scripts, "x", true, false).len(), 0, "one letter is not a query");
        let source = "xx\n".repeat(MAX_HITS + 10);
        let target = ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(3),
        };
        assert_eq!(
            find(&[(None, target, "Loop", source.as_str(), Lang::Lsl)], "xx", true, false).len(),
            MAX_HITS
        );
    }

    #[test]
    fn a_reference_is_the_whole_name_in_code_only() {
        let door = "integer count = 0;\n// count\nllSay(0, \"count\");\ncount = counter + 1;\n";
        let panel = "count = 2;\n";
        let scripts = [(0usize, "Door Script", door, Lang::Lsl), (1, "Panel Script", panel, Lang::Lsl)];
        let found = references(&scripts, "count");
        let places: Vec<(usize, usize, usize)> = found.iter().map(|hit| (hit.document, hit.line, hit.column)).collect();
        assert_eq!(
            places,
            vec![(0, 0, 8), (0, 3, 0), (1, 0, 0)],
            "the comment, the string, the longer name"
        );
        assert_eq!(found[0].range, 8..13, "the name in the document's characters");
        assert_eq!(found[1].range, 47..52, "on its own line");
        assert_eq!(found[1].text, "count = counter + 1;");
        assert_eq!(found[2].name, "Panel Script");
        // a word that is not one, or one letter, has no reference
        assert!(references(&scripts, "cou").is_empty());
        assert!(references(&scripts, "c").is_empty());
    }

    #[test]
    fn a_definition_is_searched_in_the_caret_script_first_then_the_others() {
        let panel = "default\n{\n    state_entry()\n    {\n        helper(1);\n    }\n}\n";
        let door = "integer helper(integer n)\n{\n    return n;\n}\n";
        let scripts = [(0usize, panel, Lang::Lsl), (1, door, Lang::Lsl)];
        // declared in the door's script: found there
        assert_eq!(definition(&scripts, "helper", 0), Some((1, 0, 8)));
        // a state and an event resolve too
        assert_eq!(definition(&scripts, "default", 1), Some((0, 0, 0)));
        assert_eq!(definition(&scripts, "state_entry", 1), Some((0, 2, 4)));
        // a keyword, a built-in or an unknown name has no declaration
        assert_eq!(definition(&scripts, "integer", 0), None);
        assert_eq!(definition(&scripts, "llSay", 0), None);
        assert_eq!(definition(&scripts, "", 0), None);
    }

    #[test]
    fn the_caret_script_wins_when_both_declare_the_name() {
        let panel = "integer count = 0;\n";
        let door = "integer count = 1;\n";
        let scripts = [(0usize, panel, Lang::Lsl), (1, door, Lang::Lsl)];
        assert_eq!(definition(&scripts, "count", 0), Some((0, 0, 8)));
        assert_eq!(definition(&scripts, "count", 1), Some((1, 0, 8)));
    }
}
