//! Finding a word in the scripts of one window, and where a word is declared:
//! the dock's Search tab, and Ctrl+click's go-to-definition across the scripts
//! of a linkset. Both scan the text as it is edited (not the processed view),
//! through the same matching and outline passes as the rest of the editor.

use super::editing;
use super::outline::{self, Kind};

/// One hit of the dock's Search tab.
pub(super) struct Hit {
    /// The script's place in the window's list of documents.
    pub document: usize,
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

/// The hits of `query` in the window's scripts (`(document, name, source)`),
/// in the window's order and the text's, case-insensitively, at most
/// [`MAX_HITS`]. A query of one character is not one.
pub(super) fn find(scripts: &[(usize, &str, &str, bool)], query: &str) -> Vec<Hit> {
    let needle = query.trim();
    if needle.chars().count() < 2 {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (document, name, source, lua) in scripts {
        for (line, text) in source.lines().enumerate() {
            for range in editing::matches_filtered(text, needle, true, false) {
                hits.push(Hit {
                    document: *document,
                    name: (*name).to_owned(),
                    line,
                    column: range.start,
                    first: usize::from(*lua),
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

/// Where `word` is declared among the window's scripts (`(document, source,
/// lua)`): the document's place in the list, the line, and the column of its
/// name. The script the caret is in is searched first, then the others in the
/// window's order; only the outlines' states, events, functions and globals
/// count, so a keyword or a built-in name has no declaration.
pub(super) fn definition(scripts: &[(usize, &str, bool)], word: &str, current: usize) -> Option<(usize, usize, usize)> {
    if word.is_empty() {
        return None;
    }
    let mut order: Vec<&(usize, &str, bool)> = scripts.iter().collect();
    order.sort_by_key(|(document, ..)| *document != current);
    for (document, source, lua) in order {
        for item in outline::outline(source, *lua) {
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

    #[test]
    fn a_query_lists_its_hits_in_the_windows_and_the_texts_order() {
        let door = "integer alpha = 0;\nllSay(0, \"alpha\");\n";
        let panel = "// alpha twice: alpha\n";
        let scripts = [(0usize, "Panel Script", panel, false), (1, "Door Script", door, false)];
        let hits = find(&scripts, "alpha");
        let rows: Vec<(usize, usize, usize)> = hits.iter().map(|hit| (hit.document, hit.line, hit.column)).collect();
        assert_eq!(rows, vec![(0, 0, 3), (0, 0, 16), (1, 0, 8), (1, 1, 10)]);
        assert_eq!(hits[0].name, "Panel Script");
        assert_eq!(hits[2].text, "integer alpha = 0;");
        // case-insensitive, and trimmed
        assert_eq!(find(&scripts, "  ALPHA ").len(), 4);
        // one character is not a query
        assert!(find(&scripts, "a").is_empty());
    }

    #[test]
    fn the_hits_stop_at_the_limit() {
        let source = "x\n".repeat(MAX_HITS + 10);
        let scripts = [(0usize, "Loop", source.as_str(), false)];
        assert_eq!(find(&scripts, "x").len(), 0, "one letter is not a query");
        let source = "xx\n".repeat(MAX_HITS + 10);
        assert_eq!(find(&[(0, "Loop", source.as_str(), false)], "xx").len(), MAX_HITS);
    }

    #[test]
    fn a_definition_is_searched_in_the_caret_script_first_then_the_others() {
        let panel = "default\n{\n    state_entry()\n    {\n        helper(1);\n    }\n}\n";
        let door = "integer helper(integer n)\n{\n    return n;\n}\n";
        let scripts = [(0usize, panel, false), (1, door, false)];
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
        let scripts = [(0usize, panel, false), (1, door, false)];
        assert_eq!(definition(&scripts, "count", 0), Some((0, 0, 8)));
        assert_eq!(definition(&scripts, "count", 1), Some((1, 0, 8)));
    }
}
