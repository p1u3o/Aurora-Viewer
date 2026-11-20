//! What the viewer itself can see in a script, without the simulator's
//! compiler: a block that is never closed, a `}` that closes nothing, a string
//! or a block comment left open. A script's own Problems runs it over the
//! script in front, and the object-wide pass over every script of a linkset,
//! the ones no tab holds read from their prim (`ui/scripts.rs`).
//!
//! Alchemy's Script Studio lints are beside it, in `lints.rs`.

/// One thing wrong with a script's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Finding {
    /// A `}` with no block open.
    StrayClose { line: usize },
    /// A `{` that the source never closes.
    OpenBlock { line: usize },
    /// A string that the source never closes.
    OpenString { line: usize },
    /// A `/*` that the source never closes.
    OpenComment { line: usize },
}

impl Finding {
    /// The line it is on, from 0.
    pub(super) fn line(self) -> usize {
        match self {
            Finding::StrayClose { line } | Finding::OpenBlock { line } | Finding::OpenString { line } | Finding::OpenComment { line } => {
                line
            }
        }
    }

    /// What the row says.
    pub(super) fn message(self) -> String {
        match self {
            Finding::StrayClose { .. } => aurora_i18n::tr!("script-check-stray-close").to_string(),
            Finding::OpenBlock { .. } => aurora_i18n::tr!("script-check-open-block").to_string(),
            Finding::OpenString { .. } => aurora_i18n::tr!("script-check-open-string").to_string(),
            Finding::OpenComment { .. } => aurora_i18n::tr!("script-check-open-comment").to_string(),
        }
    }
}

/// The findings of `source`, in the order of the text, at most `MOST`.
pub(super) fn structure(source: &str) -> Vec<Finding> {
    /// A source with more than this many unclosed blocks is past reading.
    const MOST: usize = 4;
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut blocks: Vec<usize> = Vec::new();
    let mut line = 0;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '\n' => line += 1,
            '/' if chars.get(index + 1) == Some(&'/') => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                continue;
            }
            '/' if chars.get(index + 1) == Some(&'*') => {
                let opened = line;
                index += 2;
                let mut closed = false;
                while index < chars.len() {
                    if chars[index] == '\n' {
                        line += 1;
                    }
                    if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                        index += 2;
                        closed = true;
                        break;
                    }
                    index += 1;
                }
                if !closed && found.len() < MOST {
                    found.push(Finding::OpenComment { line: opened });
                }
                continue;
            }
            '"' | '\'' => {
                let quote = ch;
                let opened = line;
                index += 1;
                let mut closed = false;
                while index < chars.len() {
                    if chars[index] == '\n' {
                        line += 1;
                    }
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == quote {
                        index += 1;
                        closed = true;
                        break;
                    }
                    index += 1;
                }
                if !closed && found.len() < MOST {
                    found.push(Finding::OpenString { line: opened });
                }
                continue;
            }
            '{' => blocks.push(line),
            '}' if blocks.pop().is_none() && found.len() < MOST => {
                found.push(Finding::StrayClose { line });
            }
            _ => {}
        }
        index += 1;
    }
    for opened in blocks.into_iter().rev().take(MOST) {
        found.push(Finding::OpenBlock { line: opened });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_in_order_has_nothing_to_say() {
        assert!(structure("default\n{\n    state_entry()\n    {\n        llSay(0, \"a { } // not one\");\n    }\n}\n").is_empty());
        assert!(structure("// { and } and \"\ninteger a = 0;\n").is_empty());
    }

    #[test]
    fn an_unclosed_block_is_named_at_the_line_it_opens() {
        let found = structure("default\n{\n    state_entry()\n    {\n    }\n");
        assert_eq!(found, vec![Finding::OpenBlock { line: 1 }]);
    }

    #[test]
    fn a_close_with_no_block_is_named_where_it_stands() {
        assert_eq!(structure("integer a = 0;\n}\n"), vec![Finding::StrayClose { line: 1 }]);
    }

    #[test]
    fn a_string_or_a_comment_left_open_is_named() {
        assert_eq!(structure("llSay(0, \"no end\n"), vec![Finding::OpenString { line: 0 }]);
        assert_eq!(structure("/* no end\n"), vec![Finding::OpenComment { line: 0 }]);
    }
}
