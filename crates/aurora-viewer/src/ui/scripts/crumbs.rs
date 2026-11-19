//! The path of the caret: the text that opens each block it is in, outermost
//! first. An IDE's breadcrumb bar, as the Legacy Viewer's script editor has
//! not (Alchemy's Script Studio bar under the editor, ALScriptCrumbsBar):
//! Aurora shows it in the editor's own surface instead, one row.
//!
//! LSL blocks are found by their braces, the same scan `outline` uses, so a
//! brace in a comment or a string opens nothing; SLua has no braces, so its
//! path follows the outline (its top-level functions and globals).

use super::{editing, outline};

/// One step of the path: the text that opens a block, and the line it is on.
pub(super) struct Step {
    pub text: String,
    pub line: usize,
}

/// A block of the source: where its header starts and ends, and the braces that
/// open and close it, in char indices. The completion reads the header for a
/// block's parameters and the range for the names a block holds.
pub(super) struct Block {
    /// The text of the header that opens it (from its first word to the `{`).
    pub header: std::ops::Range<usize>,
    /// The index of the `{` that opens it.
    pub open: usize,
    /// The index of the `}` that closes it (the source's end when it is open).
    pub close: usize,
}

/// The path of the block that holds `line`.
pub(super) fn path(source: &str, lua: bool, line: usize) -> Vec<Step> {
    if lua { lua_path(source, line) } else { block_path(source, line) }
}

/// The blocks `line` is inside, outermost first (LSL).
fn block_path(source: &str, line: usize) -> Vec<Step> {
    let chars: Vec<char> = source.chars().collect();
    let code = editing::code_mask(&chars);
    let starts = line_starts(&chars);
    // the line's first character is the walk's end: what opens after it does
    // not hold the line
    let end = starts.get(line).copied().unwrap_or(chars.len());
    blocks(&chars, &code)
        .into_iter()
        .filter(|block| block.open < end && end <= block.close)
        .filter_map(|block| {
            let text = header_text(&chars, block.header.start, &code);
            (!text.is_empty()).then(|| Step {
                line: line_of(&starts, block.header.start),
                text,
            })
        })
        .collect()
}

/// Every block of the source, outermost first, with the header that opens it.
pub(super) fn blocks(chars: &[char], code: &[bool]) -> Vec<Block> {
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut found = Vec::new();
    for at in 0..chars.len() {
        if !code[at] {
            continue;
        }
        match chars[at] {
            '{' => {
                // the header starts at its first word, not at the blank before
                // it: that is the line it is on
                let mut first = header_start(chars, code, at);
                while first < at && chars[first].is_whitespace() {
                    first += 1;
                }
                open.push((first, at));
            }
            '}' => {
                if let Some((header, brace)) = open.pop() {
                    found.push(Block {
                        header: header..brace,
                        open: brace,
                        close: at,
                    });
                }
            }
            _ => {}
        }
    }
    // a block the source leaves open reaches its end
    for (header, brace) in open {
        found.push(Block {
            header: header..chars.len(),
            open: brace,
            close: chars.len(),
        });
    }
    found.sort_by_key(|block| block.open);
    found
}

/// SLua's path: the outline items whose block holds the line, outermost
/// first (no braces to follow).
fn lua_path(source: &str, line: usize) -> Vec<Step> {
    let items = outline::outline(source, true);
    // a global is a name, not a block to be inside
    let mut holding: Vec<&outline::Item> = items
        .iter()
        .filter(|item| item.kind != outline::Kind::Global && item.line <= line && line <= item.end)
        .collect();
    holding.dedup_by_key(|item| item.line);
    holding
        .into_iter()
        .map(|item| Step {
            text: item.name.clone(),
            line: item.line,
        })
        .collect()
}

/// Where the statement that opens a block starts: back from its `{` over the
/// blanks, then to the last `;`, `{` or `}` of the code.
fn header_start(chars: &[char], code: &[bool], brace: usize) -> usize {
    let mut at = brace;
    while at > 0 {
        let previous = at - 1;
        if chars[previous].is_whitespace() {
            at = previous;
        } else {
            break;
        }
    }
    let mut start = at;
    while start > 0 {
        let previous = start - 1;
        if !code[previous] {
            // a comment or a string between the statement and its brace
            start = previous;
            continue;
        }
        if matches!(chars[previous], ';' | '{' | '}') {
            break;
        }
        start = previous;
    }
    start
}

/// The text of a block's header, on one line, trimmed.
fn header_text(chars: &[char], start: usize, code: &[bool]) -> String {
    // a header is a statement: what a source leaves open does not run on
    const LONGEST: usize = 120;
    let mut text = String::new();
    for (at, ch) in chars.iter().enumerate().skip(start) {
        // the header ends at the brace: what opens the block is its opening
        if code.get(at).copied().unwrap_or(false) && *ch == '{' {
            break;
        }
        if text.len() >= LONGEST {
            break;
        }
        text.push(*ch);
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The line each character index starts on.
fn line_starts(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![0];
    for (at, ch) in chars.iter().enumerate() {
        if *ch == '\n' {
            starts.push(at + 1);
        }
    }
    starts
}

fn line_of(starts: &[usize], at: usize) -> usize {
    starts.partition_point(|start| *start <= at).saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(source: &str, line: usize) -> Vec<(String, usize)> {
        path(source, false, line).into_iter().map(|step| (step.text, step.line)).collect()
    }

    const NESTED: &str = "\
integer count = 0;

default
{
    touch_start(integer total)
    {
        if (count > 1)
        {
            llSay(0, \"more\");
        }
        else
        {
            llSay(0, \"less\");
        }
    }
}
";

    #[test]
    fn the_path_names_every_block_that_holds_the_line() {
        assert_eq!(
            steps(NESTED, 8),
            vec![
                ("default".to_owned(), 2),
                ("touch_start(integer total)".to_owned(), 4),
                ("if (count > 1)".to_owned(), 6),
            ]
        );
        // the else branch, whose brace follows the if's
        assert_eq!(
            steps(NESTED, 12),
            vec![
                ("default".to_owned(), 2),
                ("touch_start(integer total)".to_owned(), 4),
                ("else".to_owned(), 10),
            ]
        );
        // inside the event but between its blocks, then inside the state alone
        assert_eq!(steps(NESTED, 6).len(), 2);
        assert_eq!(steps(NESTED, 4).len(), 1);
        // a line outside every block has no path
        assert!(steps(NESTED, 1).is_empty());
        assert!(steps(NESTED, 16).is_empty());
    }

    #[test]
    fn a_brace_in_a_comment_or_a_string_opens_nothing() {
        let source = "\
default
{
    state_entry()
    {
        // } { a comment
        string text = \"}{ {\";
        llSay(0, text);
    }
}
";
        assert_eq!(steps(source, 7), vec![("default".to_owned(), 0), ("state_entry()".to_owned(), 2)]);
    }

    #[test]
    fn a_header_on_several_lines_reads_as_one_line() {
        let source = "\
default
{
    touch_start(integer total)
    {
        if (total
            > 2)
        {
        }
    }
}
";
        let path = steps(source, 7);
        assert_eq!(path.last().map(|(text, _)| text.as_str()), Some("if (total > 2)"));
        assert_eq!(path.last().map(|(_, line)| *line), Some(4), "the line it starts on");
    }

    #[test]
    fn an_slua_path_follows_the_functions_of_the_outline() {
        let source = "\
local count = 0

local function add(a, b)
    return a + b
end

function main()
    count = add(count, 1)
end
";
        let inside = path(source, true, 2);
        assert_eq!(inside.first().map(|step| step.text.as_str()), Some("add"));
        assert!(path(source, true, 0).is_empty(), "the top level holds no block");
    }
}
