//! The outline of a script: the states, events, functions and globals it
//! declares, in the source's order, with the line each one starts on. A simple
//! scan of the lines that hold code (comments and strings are blanked out), as
//! an IDE's outline view: `default` and `state name` at the top level, the
//! events inside a state, the functions and the globals of the top level
//! (LSL's declarations start with their type, SLua's with `function` or a
//! name).

use super::editing;

/// What an outline item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    State,
    Event,
    Function,
    Global,
}

/// One item of the outline.
pub(super) struct Item {
    pub kind: Kind,
    pub name: String,
    /// The line it starts on, counted from 0 as the source is.
    pub line: usize,
}

/// The kinds of block the scan follows: a state's body holds events, a
/// function's body holds statements.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    State,
    Function,
    Other,
}

/// The types an LSL global or function starts with.
const TYPES: [&str; 7] = ["integer", "float", "string", "key", "vector", "rotation", "list"];

/// The outline of `source`.
pub(super) fn outline(source: &str, lua: bool) -> Vec<Item> {
    let chars: Vec<char> = source.chars().collect();
    let code = editing::code_mask(&chars);
    // the same lines with what is not code blanked out, so that a brace or a
    // word in a comment or a string is not one of the code's
    let mut text = String::with_capacity(source.len());
    for (ch, is_code) in chars.iter().zip(&code) {
        text.push(if *is_code || *ch == '\n' { *ch } else { ' ' });
    }
    let mut items = Vec::new();
    let mut stack: Vec<Block> = Vec::new();
    let mut pending: Option<Block> = None;
    for (index, line) in text.lines().enumerate() {
        let block = stack.last().copied();
        let trimmed = line.trim_start();
        let found = match block {
            // SLua's blocks end with `end`, not a brace: its top level is what
            // starts at the left margin, its bodies are indented
            None if lua => (trimmed.len() == line.len()).then(|| lua_top(trimmed)).flatten(),
            None => lsl_top(trimmed),
            Some(Block::State) => call_name(trimmed).map(|name| (Kind::Event, name)),
            Some(Block::Function | Block::Other) => None,
        };
        if let Some((kind, name)) = found {
            items.push(Item {
                kind,
                name: name.to_owned(),
                line: index,
            });
            // a brace opening on a later line belongs to it
            pending = Some(match kind {
                Kind::State => Block::State,
                Kind::Function => Block::Function,
                _ => Block::Other,
            });
        }
        for ch in line.chars() {
            match ch {
                '{' => stack.push(pending.take().unwrap_or(Block::Other)),
                '}' => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    items
}

fn is_ident(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

fn ident(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_ident) && !text.starts_with(|c: char| c.is_ascii_digit())
}

/// `name(` at the start of a line: an event of a state.
fn call_name(line: &str) -> Option<&str> {
    let name = line.split('(').next()?;
    let name = name.trim_end();
    if ident(name) { Some(name) } else { None }
}

/// A declaration of the top level of an LSL script.
fn lsl_top(line: &str) -> Option<(Kind, &str)> {
    let mut words = line.split_whitespace();
    let first = words.next()?;
    if first == "default" {
        return Some((Kind::State, "default"));
    }
    if first == "state" {
        let name = words.next()?;
        return ident(name).then_some((Kind::State, name));
    }
    let second = words.next();
    if TYPES.contains(&first) {
        let second = second?;
        // `integer add(integer a, integer b)`: a function with a return type
        if second.contains('(') {
            return call_name(second).map(|name| (Kind::Function, name));
        }
        // `integer count = 0;`: a global, named before its value
        let name = second.trim_end_matches([';', ',']).split('=').next()?.trim_end();
        return ident(name).then_some((Kind::Global, name));
    }
    // `add(integer a, integer b)`: a function of the top level
    if first.contains('(') {
        return call_name(first).map(|name| (Kind::Function, name));
    }
    None
}

/// A declaration of the top level of an SLua script.
fn lua_top(line: &str) -> Option<(Kind, &str)> {
    let rest = line.strip_prefix("local ").map_or(line, |rest| rest.trim_start());
    if let Some(name) = rest.strip_prefix("function ") {
        let name = name.split(|ch: char| !is_ident(ch)).next()?;
        return ident(name).then_some((Kind::Function, name));
    }
    let (name, value) = rest.split_once('=')?;
    let name = name.trim_end();
    if !ident(name) {
        return None;
    }
    if value.trim_start().starts_with("function") {
        Some((Kind::Function, name))
    } else {
        Some((Kind::Global, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(items: &[Item]) -> Vec<(Kind, &str, usize)> {
        items.iter().map(|item| (item.kind, item.name.as_str(), item.line)).collect()
    }

    #[test]
    fn an_lsl_script_lists_its_globals_functions_states_and_events() {
        let source = "\
// a comment { with a brace
integer count = 0;
string message = \"a string {\";

integer add(integer a, integer b)
{
    return a + b;
}

default
{
    state_entry()
    {
        llSay(0, message);
    }
    touch_start(integer total)
    {
        count = add(count, 1);
    }
}

state busy
{
    state_entry() { }
}
";
        let items = outline(source, false);
        assert_eq!(
            named(&items),
            vec![
                (Kind::Global, "count", 1),
                (Kind::Global, "message", 2),
                (Kind::Function, "add", 4),
                (Kind::State, "default", 9),
                (Kind::Event, "state_entry", 11),
                (Kind::Event, "touch_start", 15),
                (Kind::State, "busy", 21),
                (Kind::Event, "state_entry", 23),
            ]
        );
    }

    #[test]
    fn an_slua_script_lists_its_functions_and_globals() {
        let source = "\
-- a comment
local count = 0
local function add(a, b)
    return a + b
end

function main()
    count = add(count, 1)
end

handler = function(event)
end
";
        let items = outline(source, true);
        assert_eq!(
            named(&items),
            vec![
                (Kind::Global, "count", 1),
                (Kind::Function, "add", 2),
                (Kind::Function, "main", 6),
                (Kind::Function, "handler", 10),
            ]
        );
    }

    #[test]
    fn a_brace_in_a_comment_or_a_string_does_not_open_a_block() {
        // the string and the comment would otherwise hide the event
        let source = "\
default
{
    // }
    state_entry()
    {
        llSay(0, \"}{ {(\");
    }
    touch_start(integer n) { }
}
";
        let items = outline(source, false);
        assert_eq!(
            named(&items),
            vec![
                (Kind::State, "default", 0),
                (Kind::Event, "state_entry", 3),
                (Kind::Event, "touch_start", 7)
            ]
        );
    }
}
