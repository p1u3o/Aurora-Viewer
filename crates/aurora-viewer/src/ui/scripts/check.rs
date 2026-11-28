//! What the viewer itself can see in a script, without the simulator's
//! compiler: a `{` that is never closed, a `}` that closes nothing, a string
//! or a comment left open. A script's own Problems runs it over the script in
//! front, and the object-wide pass over every script of a linkset, the ones no
//! tab holds read from their prim (`ui/scripts.rs`). It reads the text as LSL
//! or as Lua (`lua`): Lua's comments are `--` and `--[[ ]]`, its strings also
//! take `[[ ]]`, and its braces only hold tables -- the words that open and
//! close Lua's blocks are left to the server's compiler.
//!
//! The lints are beside it, in `lints.rs`.

use super::Lang;

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
    /// A Lua block (`function`, `do`, `if`, `repeat`) the source never closes.
    OpenLuaBlock { line: usize, word: &'static str },
    /// An `end` or an `until` with no Lua block open.
    StrayLuaClose { line: usize, word: &'static str },
}

impl Finding {
    /// The line it is on, from 0.
    pub(super) fn line(self) -> usize {
        match self {
            Finding::StrayClose { line }
            | Finding::OpenBlock { line }
            | Finding::OpenString { line }
            | Finding::OpenComment { line }
            | Finding::OpenLuaBlock { line, .. }
            | Finding::StrayLuaClose { line, .. } => line,
        }
    }

    /// What the row says.
    pub(super) fn message(self) -> String {
        match self {
            Finding::StrayClose { .. } => aurora_i18n::tr!("script-check-stray-close").to_string(),
            Finding::OpenBlock { .. } => aurora_i18n::tr!("script-check-open-block").to_string(),
            Finding::OpenString { .. } => aurora_i18n::tr!("script-check-open-string").to_string(),
            Finding::OpenComment { .. } => aurora_i18n::tr!("script-check-open-comment").to_string(),
            Finding::OpenLuaBlock { word, .. } => aurora_i18n::tr!("script-check-open-lua-block", word = word.to_string()).to_string(),
            Finding::StrayLuaClose { word, .. } => aurora_i18n::tr!("script-check-stray-lua-close", word = word.to_string()).to_string(),
        }
    }
}

/// The findings of `source`, in the order of the text, at most `MOST`: the
/// braces a table never closes, a string or a comment left open, and, for Lua,
/// the words that open a block (`function`, `do`, `if`, `repeat`) against the
/// `end` or the `until` that closes it. `lua` reads the text as Lua, whose
/// comments are `--` and `--[[ ]]` and whose strings also take `[[ ]]`; its
/// braces only hold tables.
pub(super) fn structure(source: &str, lang: Lang) -> Vec<Finding> {
    let lua = lang.is_lua();
    /// A source with more than this many unclosed blocks is past reading.
    const MOST: usize = 4;
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut blocks: Vec<usize> = Vec::new();
    let mut lua_blocks: Vec<(&'static str, usize)> = Vec::new();
    let mut line = 0;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '\n' => line += 1,
            // Lua: `--` to the end of its line, `--[[ ]]` (any `=` level) over
            // as many lines as it takes
            '-' if lua && chars.get(index + 1) == Some(&'-') => {
                let opened = line;
                match long_bracket(&chars, index + 2) {
                    Some((end, closed)) => {
                        line += newlines(&chars, index, end);
                        index = end;
                        if !closed && found.len() < MOST {
                            found.push(Finding::OpenComment { line: opened });
                        }
                    }
                    None => index = line_end(&chars, index),
                }
                continue;
            }
            // Lua: a long string, `[[ ... ]]` (any `=` level)
            '[' if lua => {
                let opened = line;
                if let Some((end, closed)) = long_bracket(&chars, index) {
                    line += newlines(&chars, index, end);
                    index = end;
                    if !closed && found.len() < MOST {
                        found.push(Finding::OpenString { line: opened });
                    }
                    continue;
                }
            }
            '/' if !lua && chars.get(index + 1) == Some(&'/') => {
                index = line_end(&chars, index);
                continue;
            }
            '/' if !lua && chars.get(index + 1) == Some(&'*') => {
                let opened = line;
                let (end, closed) = block_comment_end(&chars, index + 2);
                line += newlines(&chars, index, end);
                index = end;
                if !closed && found.len() < MOST {
                    found.push(Finding::OpenComment { line: opened });
                }
                continue;
            }
            '"' | '\'' => {
                let opened = line;
                let (end, closed) = string_end(&chars, index);
                line += newlines(&chars, index, end);
                index = end;
                if !closed && found.len() < MOST {
                    found.push(Finding::OpenString { line: opened });
                }
                continue;
            }
            // a word: in Lua, a block opens or closes on one
            word if lua && (word == '_' || word.is_alphabetic()) => {
                let start = index;
                while index < chars.len() && (chars[index] == '_' || chars[index].is_alphanumeric()) {
                    index += 1;
                }
                let text: String = chars[start..index].iter().collect();
                if let Some(block) = lua_opener(&text) {
                    lua_blocks.push((block, line));
                } else if let Some(close) = lua_closer(&text)
                    && lua_blocks.pop().is_none()
                    && found.len() < MOST
                {
                    found.push(Finding::StrayLuaClose { line, word: close });
                }
                // past the word, so its letters are not read again
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
    for (block, opened) in lua_blocks.into_iter().rev().take(MOST) {
        found.push(Finding::OpenLuaBlock { line: opened, word: block });
    }
    found
}

/// The Lua block a word opens, if it opens one. `for` and `while` open through
/// their own `do`, and `elseif` is a branch of its `if`, not another block.
fn lua_opener(word: &str) -> Option<&'static str> {
    match word {
        "function" => Some("function"),
        "do" => Some("do"),
        "if" => Some("if"),
        "repeat" => Some("repeat"),
        _ => None,
    }
}

/// The Lua word that closes a block: `end` for most, `until` for a `repeat`.
fn lua_closer(word: &str) -> Option<&'static str> {
    match word {
        "end" => Some("end"),
        "until" => Some("until"),
        _ => None,
    }
}

/// Where the line that holds `at` ends: the index of its `\n`, or the end.
fn line_end(chars: &[char], at: usize) -> usize {
    chars[at..].iter().position(|c| *c == '\n').map_or(chars.len(), |i| at + i)
}

/// Where a `/*` comment whose body starts at `at` closes, and whether it does.
fn block_comment_end(chars: &[char], at: usize) -> (usize, bool) {
    let mut index = at;
    while index < chars.len() {
        if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
            return (index + 2, true);
        }
        index += 1;
    }
    (chars.len(), false)
}

/// Where the quoted string that opens at `at` closes, and whether it does: its
/// own quote, a `\` escaping the next character (LSL and Lua alike).
fn string_end(chars: &[char], at: usize) -> (usize, bool) {
    let quote = chars[at];
    let mut index = at + 1;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == quote {
            return (index + 1, true);
        }
        index += 1;
    }
    (chars.len(), false)
}

/// The newlines a span holds, for the lines that follow it.
fn newlines(chars: &[char], from: usize, to: usize) -> usize {
    chars[from..to.min(chars.len())].iter().filter(|c| **c == '\n').count()
}

/// A Lua long bracket at `at` -- `[[`, `[=[`, or more `=` between -- and where
/// it ends: the index just after its `]]` (or its like), and whether that was
/// found. `None` when no bracket opens here, so that `a[1]` stays code.
fn long_bracket(chars: &[char], at: usize) -> Option<(usize, bool)> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let mut level = 0;
    let mut i = at + 1;
    while chars.get(i) == Some(&'=') {
        level += 1;
        i += 1;
    }
    if chars.get(i) != Some(&'[') {
        return None;
    }
    i += 1;
    while i < chars.len() {
        if chars[i] == ']' {
            let mut seen = 0;
            let mut end = i + 1;
            while seen < level && chars.get(end) == Some(&'=') {
                seen += 1;
                end += 1;
            }
            if seen == level && chars.get(end) == Some(&']') {
                return Some((end + 1, true));
            }
        }
        i += 1;
    }
    Some((chars.len(), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_in_order_has_nothing_to_say() {
        assert!(
            structure(
                "default\n{\n    state_entry()\n    {\n        llSay(0, \"a { } // not one\");\n    }\n}\n",
                Lang::Lsl
            )
            .is_empty()
        );
        assert!(structure("// { and } and \"\ninteger a = 0;\n", Lang::Lsl).is_empty());
    }

    #[test]
    fn an_unclosed_block_is_named_at_the_line_it_opens() {
        let found = structure("default\n{\n    state_entry()\n    {\n    }\n", Lang::Lsl);
        assert_eq!(found, vec![Finding::OpenBlock { line: 1 }]);
    }

    #[test]
    fn a_close_with_no_block_is_named_where_it_stands() {
        assert_eq!(structure("integer a = 0;\n}\n", Lang::Lsl), vec![Finding::StrayClose { line: 1 }]);
    }

    #[test]
    fn a_string_or_a_comment_left_open_is_named() {
        assert_eq!(structure("llSay(0, \"no end\n", Lang::Lsl), vec![Finding::OpenString { line: 0 }]);
        assert_eq!(structure("/* no end\n", Lang::Lsl), vec![Finding::OpenComment { line: 0 }]);
    }

    /// Lua reads its own comments and strings: an apostrophe or a brace in a
    /// `--` comment is neither, `--[[ ]]` and `[[ ]]` run over lines, and a
    /// table's braces are still counted.
    #[test]
    fn lua_comments_and_long_strings_are_not_code() {
        let source = "-- it's fine: { and \" and }
function f()
    local t = { a = 1 }
    print([[ { \" ]])  -- [[ another } ]]
end
";
        assert!(structure(source, Lang::Lua).is_empty(), "{:?}", structure(source, Lang::Lua));
        // the same text as LSL: the apostrophe opens a string that never ends
        assert_eq!(structure("-- it's fine\n", Lang::Lsl), vec![Finding::OpenString { line: 0 }]);
        // a table or a block left open, and a `}` closing none
        assert_eq!(
            structure("local t = {\n  a = 1,\n", Lang::Lua),
            vec![Finding::OpenBlock { line: 0 }]
        );
        assert_eq!(
            structure("end\n}\n", Lang::Lua),
            vec![Finding::StrayLuaClose { line: 0, word: "end" }, Finding::StrayClose { line: 1 }]
        );
    }

    #[test]
    fn a_lua_long_bracket_left_open_is_named_and_its_newlines_are_counted() {
        assert_eq!(
            structure("print(\"one\")\nprint([[a\nb\n", Lang::Lua),
            vec![Finding::OpenString { line: 1 }]
        );
        assert_eq!(structure("--[[ a\nb\n", Lang::Lua), vec![Finding::OpenComment { line: 0 }]);
        // the line after a long string is its own, as the source counts it
        assert_eq!(
            structure("local s = [[a\nb]]\nlocal t = {\n", Lang::Lua),
            vec![Finding::OpenBlock { line: 2 }]
        );
    }

    /// Lua blocks: the words that open them against the `end` or the `until`
    /// that closes them; `for` and `while` open through their own `do`, and
    /// `elseif` stays in its `if`.
    #[test]
    fn lua_blocks_open_and_close_on_their_words() {
        let balanced = "local function f(x)\n  if x then\n    for i = 1, 3 do\n      while i < 2 do i = i + 1 end\n    end\n  elseif x == 2 then\n    repeat x = x - 1 until x == 0\n  end\nend\n";
        assert!(structure(balanced, Lang::Lua).is_empty(), "{:?}", structure(balanced, Lang::Lua));
        assert_eq!(
            structure("function f()\n  do\n    print(1)\nend\n", Lang::Lua),
            vec![Finding::OpenLuaBlock { line: 0, word: "function" }],
            "the end closes the do"
        );
        assert_eq!(
            structure("repeat\n  x = 1\n", Lang::Lua),
            vec![Finding::OpenLuaBlock { line: 0, word: "repeat" }]
        );
        assert_eq!(
            structure("if a then\n  b()\nend\nend\n", Lang::Lua),
            vec![Finding::StrayLuaClose { line: 3, word: "end" }]
        );
        assert_eq!(
            structure("until x\n", Lang::Lua),
            vec![Finding::StrayLuaClose { line: 0, word: "until" }]
        );
        // a word in a comment or a string is not one, and the declarations
        // that are not blocks (`for` before its `do`) are left alone
        assert!(
            structure(
                "-- function and do\nlocal s = 'if repeat end'\nfor i = 1, 2 do print(i) end\n",
                Lang::Lua
            )
            .is_empty(),
            "{:?}",
            structure(
                "-- function and do\nlocal s = 'if repeat end'\nfor i = 1, 2 do print(i) end\n",
                Lang::Lua
            )
        );
    }

    /// A long bracket has its level of `=`s: `[=[` closes at `]=]` only.
    #[test]
    fn a_lua_long_bracket_closes_at_its_own_level() {
        assert!(structure("local s = [=[ ]] ]=]\n", Lang::Lua).is_empty());
        assert_eq!(structure("local s = [=[ ]]\n", Lang::Lua), vec![Finding::OpenString { line: 0 }]);
        assert_eq!(structure("a[1] = 2\n", Lang::Lua), vec![]);
    }
}
