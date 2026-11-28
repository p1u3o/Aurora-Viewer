//! What the editor itself says about a script, after the lints of Alchemy's
//! Script Studio (`allsllintpass.cpp`): a timer too fast, a sleep that stops
//! the script, a listener that hears everything, a sensor too often, an integer
//! past 32 bits, primitive-parameter calls that could be one, a string built in
//! a loop, a call repeated in an event. Read off the text, over its code only,
//! so a pattern in a comment or a string is not one.
//!
//! The checks that need the whole script follow the same source (the semantic
//! warnings of lslint, github.com/Makopo/lslint, public domain): a call's
//! arguments and an event's parameters against the language's catalog, a name
//! that starts like a built-in but is none (with the nearest one suggested), an
//! event handled twice in a state, an assignment where a comparison belongs, a
//! body that is only a semicolon, a switch without `default` or with a case
//! twice, and a change to the state the script is already in.
//!
//! They land in the Problems dock beside the simulator's own messages (and in
//! the object-wide check), the same rows, a click away from the line.

use super::Lang;
use super::crumbs;
use super::editing;

/// One word of what the editor says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Rule {
    FastTimer,
    SleepingCall,
    CostlyListen,
    FastSensor,
    IntegerPast32Bits,
    MergeablePrimParams,
    StringBuild,
    RepeatedCall {
        count: usize,
    },
    /// A call of a known function with the wrong number of arguments.
    FunctionArguments {
        name: String,
        expected: usize,
        given: usize,
    },
    /// An event handler with the wrong number of parameters.
    EventArguments {
        name: String,
        expected: usize,
        given: usize,
    },
    /// A call of a name that starts like a built-in, but is none, with the
    /// nearest built-in to suggest.
    UnknownFunction {
        name: String,
        suggestion: Option<String>,
    },
    /// The same event handled twice in one state.
    DuplicateEvent {
        name: String,
    },
    /// `if (x = 1)`: a comparison that assigns.
    AssignmentInCondition,
    /// `if (x);` or `while (x);`: a condition whose body is nothing.
    EmptyIf,
    EmptyWhile,
    /// A switch with no `default`.
    SwitchNoDefault,
    /// Two case labels with the same value.
    SwitchDuplicateCase {
        value: String,
    },
    /// `state default;` while already in `default`.
    StateAlreadyCurrent {
        name: String,
    },
    /// A Lua script calls an LSL name as LSL writes it (`llSay`), where Lua
    /// writes it under `ll.`.
    LuaLslCall {
        name: String,
        suggestion: Option<String>,
    },
    /// A `local` the script declares and never reads.
    LuaUnusedLocal {
        name: String,
    },
    /// An assignment to a name the script never declares `local`: a global.
    LuaGlobal {
        name: String,
    },
    /// A `+` with a string on its left: Lua joins strings with `..`.
    LuaStringPlus,
}

impl Rule {
    /// What the row says.
    pub(super) fn message(&self) -> String {
        match self {
            Rule::FastTimer => aurora_i18n::tr!("script-lint-fast-timer").to_string(),
            Rule::SleepingCall => aurora_i18n::tr!("script-lint-sleeping-call").to_string(),
            Rule::CostlyListen => aurora_i18n::tr!("script-lint-costly-listen").to_string(),
            Rule::FastSensor => aurora_i18n::tr!("script-lint-fast-sensor").to_string(),
            Rule::IntegerPast32Bits => aurora_i18n::tr!("script-lint-integer-past-32-bits").to_string(),
            Rule::MergeablePrimParams => aurora_i18n::tr!("script-lint-mergeable-prim-params").to_string(),
            Rule::StringBuild => aurora_i18n::tr!("script-lint-string-build").to_string(),
            Rule::RepeatedCall { count } => aurora_i18n::tr!("script-lint-repeated-call", count = *count as i64).to_string(),
            Rule::FunctionArguments { name, expected, given } => aurora_i18n::tr!(
                "script-lint-arguments",
                name = name.as_str(),
                expected = *expected as i64,
                given = *given as i64
            )
            .to_string(),
            Rule::EventArguments { name, expected, given } => aurora_i18n::tr!(
                "script-lint-event-arguments",
                event = name.as_str(),
                expected = *expected as i64,
                given = *given as i64
            )
            .to_string(),
            Rule::UnknownFunction { name, suggestion } => {
                let said = aurora_i18n::tr!("script-lint-unknown-function", name = name.as_str()).to_string();
                match suggestion {
                    Some(suggestion) => {
                        let hint = aurora_i18n::tr!("script-lint-did-you-mean", name = suggestion.as_str()).to_string();
                        format!("{said} {hint}")
                    }
                    None => said,
                }
            }
            Rule::DuplicateEvent { name } => aurora_i18n::tr!("script-lint-duplicate-event", event = name.as_str()).to_string(),
            Rule::AssignmentInCondition => aurora_i18n::tr!("script-lint-assignment-in-condition").to_string(),
            Rule::EmptyIf => aurora_i18n::tr!("script-lint-empty-if").to_string(),
            Rule::EmptyWhile => aurora_i18n::tr!("script-lint-empty-while").to_string(),
            Rule::SwitchNoDefault => aurora_i18n::tr!("script-lint-switch-no-default").to_string(),
            Rule::SwitchDuplicateCase { value } => {
                aurora_i18n::tr!("script-lint-switch-duplicate-case", value = value.as_str()).to_string()
            }
            Rule::StateAlreadyCurrent { name } => aurora_i18n::tr!("script-lint-state-current", state = name.as_str()).to_string(),
            Rule::LuaLslCall { name, suggestion } => match suggestion {
                Some(suggestion) => {
                    aurora_i18n::tr!("script-lint-lua-lsl-call", name = name.as_str(), suggestion = suggestion.as_str()).to_string()
                }
                None => aurora_i18n::tr!("script-lint-lua-lsl-call-hint", name = name.as_str()).to_string(),
            },
            Rule::LuaUnusedLocal { name } => aurora_i18n::tr!("script-lint-lua-unused-local", name = name.as_str()).to_string(),
            Rule::LuaGlobal { name } => aurora_i18n::tr!("script-lint-lua-global", name = name.as_str()).to_string(),
            Rule::LuaStringPlus => aurora_i18n::tr!("script-lint-lua-string-plus").to_string(),
        }
    }
}

/// One place the editor found something, on a line and a column of the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Lint {
    pub line: usize,
    pub column: usize,
    pub rule: Rule,
}

/// The least a timer may fire at, the least a sensor may repeat at.
const FAST: f64 = 0.1;
const OFTEN: f64 = 1.0;
/// How many times a call in an event has to repeat before it is worth a local.
const REPEATS: usize = 3;
/// The most rows one script gives.
const MOST: usize = 40;

/// Where each line of the text starts, 0 first: the line and column of a
/// finding, read in one step instead of walking the text from its start for
/// each one (which made the linter quadratic).
struct Lines(Vec<usize>);

impl Lines {
    fn new(chars: &[char]) -> Self {
        let mut starts = vec![0];
        starts.extend(chars.iter().enumerate().filter_map(|(at, ch)| (*ch == '\n').then_some(at + 1)));
        Self(starts)
    }

    /// The line `at` is on, from 0.
    fn line(&self, at: usize) -> usize {
        self.0.partition_point(|start| *start <= at).saturating_sub(1)
    }

    /// The line and the column of `at`, from 0.
    fn position(&self, at: usize) -> (usize, usize) {
        let line = self.line(at);
        (line, at - self.0[line])
    }
}

/// The lints of a script's text, in the order of the lines.
pub(super) fn lints(source: &str, outline: &[super::outline::Item]) -> Vec<Lint> {
    let chars: Vec<char> = source.chars().collect();
    let code = editing::code_mask(&chars);
    let starts = Lines::new(&chars);
    let mut found: Vec<Lint> = Vec::new();
    let mut offset = 0;
    let lines: Vec<&str> = source.split('\n').collect();
    for (number, line) in lines.iter().enumerate() {
        // the line's code, everything else blanked: a pattern in a comment or a
        // string is not code (the mask keeps one char for one, so a column of
        // the blank line is a column of the line)
        let length = line.chars().count();
        let masked: Vec<char> = chars[offset..(offset + length).min(chars.len())]
            .iter()
            .enumerate()
            .map(|(index, ch)| if code[offset + index] { *ch } else { ' ' })
            .collect();
        let text: String = masked.iter().collect();
        let original: Vec<char> = line.chars().collect();
        // the calls whose arguments matter
        for (at, _) in calls(&text, "llSetTimerEvent(") {
            if fast_timer(&call_arguments(&masked, "llSetTimerEvent(", at)) {
                found.push(Lint {
                    line: number,
                    column: at,
                    rule: Rule::FastTimer,
                });
            }
        }
        for (at, _) in calls(&text, "llSleep(") {
            found.push(Lint {
                line: number,
                column: at,
                rule: Rule::SleepingCall,
            });
        }
        for (at, _) in calls(&text, "llListen(") {
            if costly_listen(&call_arguments(&original, "llListen(", at)) {
                found.push(Lint {
                    line: number,
                    column: at,
                    rule: Rule::CostlyListen,
                });
            }
        }
        for (at, _) in calls(&text, "llSensorRepeat(") {
            if fast_sensor(&call_arguments(&masked, "llSensorRepeat(", at)) {
                found.push(Lint {
                    line: number,
                    column: at,
                    rule: Rule::FastSensor,
                });
            }
        }
        // an integer that does not fit in 32 bits
        for (at, value) in integers(&text) {
            if i64::from(i32::MIN) > value || value > i64::from(i32::MAX) {
                found.push(Lint {
                    line: number,
                    column: at,
                    rule: Rule::IntegerPast32Bits,
                });
            }
        }
        // primitive-parameter calls that could be one, on the line or the next
        let here = count_primitive_params(&text);
        let next = lines.get(number + 1).copied().unwrap_or_default();
        if here >= 2 || (here == 1 && count_primitive_params(next) >= 1) {
            found.push(Lint {
                line: number,
                column: 0,
                rule: Rule::MergeablePrimParams,
            });
        }
        offset += length + 1;
    }
    // a string built in a loop, and a call repeated in a block
    let blocks = crumbs::blocks(&chars, &code);
    for block in &blocks {
        let header: String = chars[block.header.clone()].iter().collect();
        let header = header.trim_start();
        let loops = header.starts_with("for") || header.starts_with("while");
        let inner: String = chars[block.open + 1..block.close].iter().collect();
        let first = starts.line(block.open + 1);
        if loops {
            for (index, text) in inner.split('\n').enumerate() {
                if builds_string(text) {
                    found.push(Lint {
                        line: first + index,
                        column: 0,
                        rule: Rule::StringBuild,
                    });
                }
            }
        }
        // calls repeated with the same arguments in one event or function
        let mut seen: Vec<(String, usize, usize)> = Vec::new();
        for (index, text) in inner.split('\n').enumerate() {
            let line = first + index;
            for (_, call) in call_texts(text) {
                if let Some(entry) = seen.iter_mut().find(|(known, ..)| *known == call) {
                    entry.2 += 1;
                } else {
                    seen.push((call, line, 1));
                }
            }
        }
        for (_call, line, count) in seen {
            if count >= REPEATS {
                found.push(Lint {
                    line,
                    column: 0,
                    rule: Rule::RepeatedCall { count },
                });
                break;
            }
        }
    }
    // the checks that need the whole script: a call's arguments, an event's
    // parameters, a condition, a switch, a state
    structural_lints(&chars, &code, &starts, outline, &mut found);
    found.sort_by_key(|lint| (lint.line, lint.column));
    found.dedup_by(|a, b| a.line == b.line && a.column == b.column && std::mem::discriminant(&a.rule) == std::mem::discriminant(&b.rule));
    found.truncate(MOST);
    found
}

/// What stands between the brackets of the call at `at` (`at` a char index),
/// in chars of the line itself.
fn call_arguments(chars: &[char], needle: &str, at: usize) -> String {
    let open = at + needle.chars().count();
    let mut depth = 1;
    let mut end = chars.len();
    for (index, ch) in chars.iter().enumerate().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = index;
                    break;
                }
            }
            _ => {}
        }
    }
    chars[open.min(chars.len())..end].iter().collect()
}

/// How many primitive-parameter calls a line makes.
fn count_primitive_params(text: &str) -> usize {
    [
        "llSetPrimitiveParams(",
        "llSetLinkPrimitiveParams(",
        "llSetLinkPrimitiveParamsFast(",
    ]
    .iter()
    .map(|needle| text.matches(needle).count())
    .sum()
}

/// The line a char index is on.
/// Each call of `needle` on a line, at its char column and with what stands
/// between its brackets.
fn calls(text: &str, needle: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(needle) {
        let at = from + at;
        let column = text[..at].chars().count();
        let open = at + needle.len();
        let mut depth = 1;
        let mut end = open;
        for (index, ch) in text[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + index;
                        break;
                    }
                }
                _ => {}
            }
        }
        found.push((column, text[open..end].to_owned()));
        from = end.max(at + needle.len());
    }
    found
}

/// The whole arguments of each call on a line, as `name(arguments)`.
fn call_texts(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("ll") {
        let at = from + at;
        let name: String = text[at..].chars().take_while(|ch| ch.is_alphanumeric() || *ch == '_').collect();
        let after = at + name.len();
        if name.len() > 2 && text[after..].starts_with('(') {
            let mut depth = 0;
            let mut end = after;
            for (index, ch) in text[after..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = after + index + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            found.push((at, text[at..end].to_owned()));
            from = end;
        } else {
            from = after.max(at + 1);
        }
    }
    found
}

/// `llSetTimerEvent(x)` with a timer faster than [`FAST`].
fn fast_timer(arguments: &str) -> bool {
    arguments.trim().parse::<f64>().is_ok_and(|seconds| seconds > 0.0 && seconds < FAST)
}

/// `llListen(channel, "", id, "")`: hears every speaker on the channel.
fn costly_listen(arguments: &str) -> bool {
    let args: Vec<&str> = arguments.split(',').map(str::trim).collect();
    args.len() >= 4 && args[1] == "\"\"" && args[3] == "\"\""
}

/// `llSensorRepeat(name, id, type, range, arc, rate)` with a rate faster
/// than [`OFTEN`].
fn fast_sensor(arguments: &str) -> bool {
    let args: Vec<&str> = arguments.split(',').map(str::trim).collect();
    args.len() >= 6 && args[5].parse::<f64>().is_ok_and(|rate| rate > 0.0 && rate < OFTEN)
}

/// The decimal integers of a line, with where they start.
fn integers(text: &str) -> Vec<(usize, i64)> {
    let mut found = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        if !chars[at].is_ascii_digit() || (at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_')) {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && chars[at].is_ascii_digit() {
            at += 1;
        }
        // a float, or a suffix of a longer word, is not an integer
        if chars.get(at).is_some_and(|ch| *ch == '.') || chars.get(start + 1).is_some_and(|ch| ch.is_alphabetic()) {
            continue;
        }
        if let Ok(value) = chars[start..at].iter().collect::<String>().parse::<i64>() {
            found.push((start, value));
        }
    }
    found
}

/// `name = name +`, `name +=`: a string grown a piece at a time.
fn builds_string(text: &str) -> bool {
    let Some((left, right)) = text.split_once('=') else {
        return false;
    };
    let name = left.trim_end_matches("+=").trim();
    if name.is_empty() || !name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
        return false;
    }
    let right = right.trim_start();
    if left.ends_with("+=") {
        return true;
    }
    right.starts_with(name) && right[name.len()..].trim_start().starts_with('+')
}

/// The lints that need more than one line: they read the whole code, with
/// what is not code blanked out, and the catalog of the language in force.
fn structural_lints(chars: &[char], code: &[bool], lines: &Lines, items: &[super::outline::Item], found: &mut Vec<Lint>) {
    let masked: Vec<char> = chars
        .iter()
        .zip(code)
        .map(|(ch, is_code)| if *is_code || *ch == '\n' { *ch } else { ' ' })
        .collect();
    // for counting a call's arguments: a string is one argument's worth of
    // text (its quotes kept, its commas not), a comment is nothing
    let countable = countable(chars, code);
    let declared: std::collections::HashSet<&str> = items
        .iter()
        .filter(|item| item.kind == super::outline::Kind::Function)
        .map(|item| item.name.as_str())
        .collect();
    let states: Vec<(&str, usize, usize)> = items
        .iter()
        .filter(|item| item.kind == super::outline::Kind::State)
        .map(|item| (item.name.as_str(), item.line, item.end))
        .collect();
    let catalog = super::syntax::catalog(Lang::Lsl);
    // a call of an `ll` function: its arguments against the catalog's signature
    for (name, open) in identifier_calls(&masked) {
        if !name.starts_with("ll") {
            continue;
        }
        let Some(close) = matching(&masked, open) else {
            continue;
        };
        let given = argument_count(&countable, open, close);
        let start = open.saturating_sub(name.chars().count());
        let (line, column) = lines.position(start);
        match catalog.entry(&name) {
            Some(entry) if entry.group == "functions" => {
                if let Some(expected) = signature_arguments(&entry.signature)
                    && expected != given
                {
                    found.push(Lint {
                        line,
                        column,
                        rule: Rule::FunctionArguments { name, expected, given },
                    });
                }
            }
            Some(_) => {}
            None if !declared.contains(name.as_str()) => {
                let suggestion = nearest_function(catalog, &name);
                found.push(Lint {
                    line,
                    column,
                    rule: Rule::UnknownFunction { name, suggestion },
                });
            }
            None => {}
        }
    }
    // an event handler: its parameters, and the same one twice in a state
    let mut handled: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for item in items.iter().filter(|item| item.kind == super::outline::Kind::Event) {
        let state = states
            .iter()
            .filter(|(_, first, last)| *first <= item.line && item.line <= *last)
            .max_by_key(|(_, first, _)| *first)
            .map(|(name, ..)| *name);
        let Some(start) = start_of_line(chars, item.line) else {
            continue;
        };
        let rest: String = chars[start..].iter().take_while(|ch| **ch != '\n' && **ch != '{').collect();
        let Some(found_at) = rest.find(&format!("{}(", item.name)) else {
            continue;
        };
        let at = found_at + item.name.chars().count();
        let open = start + at;
        let given = matching(&masked, open)
            .map(|close| argument_count(&masked, open, close))
            .unwrap_or(0);
        if let Some(entry) = catalog.entry(&item.name)
            && entry.group == "events"
            && let Some(expected) = signature_arguments(&entry.signature)
            && expected != given
        {
            found.push(Lint {
                line: item.line,
                column: found_at,
                rule: Rule::EventArguments {
                    name: item.name.clone(),
                    expected,
                    given,
                },
            });
        }
        if let Some(state) = state
            && !handled.insert((state.to_owned(), item.name.clone()))
        {
            found.push(Lint {
                line: item.line,
                column: found_at,
                rule: Rule::DuplicateEvent { name: item.name.clone() },
            });
        }
    }
    // `if (x = 1)`, `if (x);`, `while (x);`
    for at in keywords(&masked, "if") {
        condition_lints(&masked, lines, at, false, found);
    }
    for at in keywords(&masked, "while") {
        // the `while` of a do-while needs its semicolon, and has a body
        if !is_do_while(&masked, at) {
            condition_lints(&masked, lines, at, true, found);
        }
    }
    // a switch without a default, a case twice
    for at in keywords(&masked, "switch") {
        switch_lints(&masked, chars, lines, at, found);
    }
    // changing to the state the script is already in
    for at in keywords(&masked, "state") {
        let mut i = at + 5;
        while i < masked.len() && masked[i].is_whitespace() {
            i += 1;
        }
        let start = i;
        while i < masked.len() && is_ident(masked[i]) {
            i += 1;
        }
        if i == start {
            continue;
        }
        let name: String = masked[start..i].iter().collect();
        let mut j = i;
        while j < masked.len() && masked[j].is_whitespace() {
            j += 1;
        }
        if masked.get(j) != Some(&';') {
            continue;
        }
        let (line, column) = lines.position(at);
        if let Some((current, ..)) = states
            .iter()
            .filter(|(_, first, last)| *first <= line && line <= *last)
            .max_by_key(|(_, first, _)| *first)
            && *current == name
        {
            found.push(Lint {
                line,
                column,
                rule: Rule::StateAlreadyCurrent { name },
            });
        }
    }
}

/// `if (…)` or `while (…)` at `at`: an assignment where a comparison belongs,
/// and a body that is only a semicolon.
fn condition_lints(masked: &[char], lines: &Lines, at: usize, looping: bool, found: &mut Vec<Lint>) {
    let mut i = at + if looping { 5 } else { 2 };
    while i < masked.len() && masked[i].is_whitespace() {
        i += 1;
    }
    if masked.get(i) != Some(&'(') {
        return;
    }
    let Some(close) = matching(masked, i) else {
        return;
    };
    let (line, column) = lines.position(at);
    if has_assignment(&masked[i + 1..close]) {
        found.push(Lint {
            line,
            column,
            rule: Rule::AssignmentInCondition,
        });
    }
    let mut j = close + 1;
    while j < masked.len() && masked[j].is_whitespace() {
        j += 1;
    }
    if masked.get(j) == Some(&';') {
        found.push(Lint {
            line,
            column,
            rule: if looping { Rule::EmptyWhile } else { Rule::EmptyIf },
        });
    }
}

/// A `switch (…) { … }` at `at`: no `default`, or the same case twice.
fn switch_lints(masked: &[char], chars: &[char], lines: &Lines, at: usize, found: &mut Vec<Lint>) {
    let mut i = at + 6;
    while i < masked.len() && masked[i].is_whitespace() {
        i += 1;
    }
    if masked.get(i) != Some(&'(') {
        return;
    }
    let Some(close) = matching(masked, i) else {
        return;
    };
    let mut j = close + 1;
    while j < masked.len() && masked[j].is_whitespace() {
        j += 1;
    }
    if masked.get(j) != Some(&'{') {
        return;
    }
    let Some(end) = matching(masked, j) else {
        return;
    };
    let (line, column) = lines.position(at);
    let body = &masked[j + 1..end];
    let mut depth = 0i32;
    let mut values: Vec<String> = Vec::new();
    let mut has_default = false;
    let mut k = 0;
    while k < body.len() {
        match body[k] {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ if depth == 0 && word_at(body, k, "case") => {
                let start = k + 4;
                let mut d = 0i32;
                let mut colon = None;
                for (off, ch) in body[start..].iter().enumerate() {
                    match ch {
                        '(' | '[' | '{' => d += 1,
                        ')' | ']' | '}' => d -= 1,
                        ':' if d == 0 => {
                            colon = Some(start + off);
                            break;
                        }
                        _ => {}
                    }
                }
                let Some(colon) = colon else {
                    break;
                };
                // the value as it is written, strings and comments included
                let value: String = chars[j + 1 + start..j + 1 + colon].iter().collect();
                values.push(value.split_whitespace().collect::<Vec<_>>().join(" "));
                k = colon + 1;
                continue;
            }
            _ if depth == 0 && word_at(body, k, "default") => {
                has_default = true;
                k += 7;
                continue;
            }
            _ => {}
        }
        k += 1;
    }
    if !has_default {
        found.push(Lint {
            line,
            column,
            rule: Rule::SwitchNoDefault,
        });
    }
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if !seen.insert(value.clone()) {
            found.push(Lint {
                line,
                column,
                rule: Rule::SwitchDuplicateCase { value },
            });
        }
    }
}

/// Whether `chars` at `at` starts the word `word`, on its own.
fn word_at(chars: &[char], at: usize, word: &str) -> bool {
    let word: Vec<char> = word.chars().collect();
    chars.len() >= at + word.len()
        && chars[at..at + word.len()] == word[..]
        && (at == 0 || !is_ident(chars[at - 1]))
        && chars.get(at + word.len()).is_none_or(|ch| !is_ident(*ch))
}

/// Each `name(` of the code, with the index of its bracket.
fn identifier_calls(chars: &[char]) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && is_ident(chars[i]) {
                i += 1;
            }
            if chars.get(i) == Some(&'(') {
                found.push((chars[start..i].iter().collect(), i));
            }
        } else {
            i += 1;
        }
    }
    found
}

/// The index of the bracket that closes the one at `open`, if any.
fn matching(chars: &[char], open: usize) -> Option<usize> {
    let (opening, closing) = match chars.get(open)? {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        '{' => ('{', '}'),
        _ => return None,
    };
    let mut depth = 0i32;
    for (index, ch) in chars.iter().enumerate().skip(open) {
        if *ch == opening {
            depth += 1;
        } else if *ch == closing {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// The code in which a call's arguments are counted: what is not code blanked,
/// but a string kept as its two quotes (so `f(\"a,b\")` is one argument, and
/// not two, nor none).
fn countable(chars: &[char], code: &[bool]) -> Vec<char> {
    let mut out: Vec<char> = chars.iter().map(|ch| if *ch == '\n' { '\n' } else { ' ' }).collect();
    let mut i = 0;
    while i < chars.len() {
        if code[i] {
            out[i] = chars[i];
            i += 1;
        } else if matches!(chars[i], '"' | '\'') {
            let quote = chars[i];
            out[i] = quote;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == quote {
                    out[i] = quote;
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else {
            // a comment, or the rest of one
            i += 1;
        }
    }
    out
}

/// How many arguments stand between `open` and `close`: the commas at the
/// call's own depth, one argument for what is not empty.
fn argument_count(chars: &[char], open: usize, close: usize) -> usize {
    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut any = false;
    let mut i = open + 1;
    while i < close {
        let ch = chars[i];
        if !ch.is_whitespace() {
            any = true;
        }
        match ch {
            // a vector or a rotation literal: its commas are the literal's,
            // not the call's (`llSetText(t, <1, 1, 1>, 1.0)` gives three)
            '<' if starts_vector(&chars[i..close]) => {
                while i < close && chars[i] != '>' {
                    i += 1;
                }
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => commas += 1,
            _ => {}
        }
        i += 1;
    }
    if any { commas + 1 } else { 0 }
}

/// Does this `<` open a vector or a rotation literal? Its content is numbers
/// and commas only, where `a < b` is a comparison (`LLSetText`'s colour is a
/// literal; a comparison is not one).
fn starts_vector(chars: &[char]) -> bool {
    if chars.first() != Some(&'<') {
        return false;
    }
    let mut digits = false;
    for ch in chars.iter().skip(1).take(64) {
        match ch {
            '>' => return digits,
            c if c.is_ascii_digit() => digits = true,
            '.' | '-' | '+' | ',' | 'e' | 'E' | ' ' | '\t' | '\n' | '\r' => {}
            _ => return false,
        }
    }
    false
}

/// The arguments a catalog signature declares: `llSay(integer channel, string
/// msg)` takes two.
fn signature_arguments(signature: &str) -> Option<usize> {
    let open = signature.find('(')?;
    let close = signature.rfind(')')?;
    let inner = signature[open + 1..close].trim();
    Some(if inner.is_empty() { 0 } else { inner.split(',').count() })
}

/// The nearest `ll` function of the catalog, when a name is close to one.
fn nearest_function(catalog: &crate::lsl_keywords::Catalog, name: &str) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for entry in catalog.entries() {
        if entry.group != "functions" || !entry.name.starts_with("ll") {
            continue;
        }
        let distance = edit_distance(name, &entry.name);
        if distance > 2 {
            continue;
        }
        if best.is_none_or(|(known, _)| distance < known) {
            best = Some((distance, entry.name.as_str()));
        }
    }
    best.map(|(_, name)| name.to_owned())
}

/// The letters one name has to change, add or drop to become the other.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row.push((previous[j] + 1).min(row[j - 1] + 1).min(previous[j - 1] + cost));
        }
        previous = row;
    }
    previous[b.len()]
}

/// Whether a condition assigns rather than compares: a `=` that is neither
/// half of `==` nor part of `!=`, `<=`, `>=` or a compound assignment.
fn has_assignment(chars: &[char]) -> bool {
    for (index, ch) in chars.iter().enumerate() {
        if *ch != '=' {
            continue;
        }
        let previous = index.checked_sub(1).and_then(|at| chars.get(at)).copied();
        let next = chars.get(index + 1).copied();
        let joined = matches!(previous, Some('=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%'));
        if !joined && next != Some('=') {
            return true;
        }
    }
    false
}

/// The index of a line's first char, in the whole source.
fn start_of_line(chars: &[char], line: usize) -> Option<usize> {
    if line == 0 {
        return Some(0);
    }
    let mut seen = 0;
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '\n' {
            seen += 1;
            if seen == line {
                return Some(index + 1);
            }
        }
    }
    None
}

/// The line and column of a char index.
/// Each `word` of the code, on its own.
fn keywords(chars: &[char], word: &str) -> Vec<usize> {
    let word: Vec<char> = word.chars().collect();
    let mut found = Vec::new();
    if word.is_empty() || chars.len() < word.len() {
        return found;
    }
    for at in 0..=chars.len() - word.len() {
        if chars[at..at + word.len()] == word[..] && word_at(chars, at, &word.iter().collect::<String>()) {
            found.push(at);
        }
    }
    found
}

/// The `while` that ends a `do { … } while (…);`, which needs its semicolon.
fn is_do_while(chars: &[char], at: usize) -> bool {
    let mut i = at;
    loop {
        if i == 0 {
            return false;
        }
        i -= 1;
        if !chars[i].is_whitespace() {
            break;
        }
    }
    if chars[i] != '}' {
        return false;
    }
    let mut depth = 1i32;
    let mut open = None;
    while i > 0 {
        i -= 1;
        match chars[i] {
            '}' => depth += 1,
            '{' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(open) = open else {
        return false;
    };
    let mut end = open;
    loop {
        if end == 0 {
            return false;
        }
        end -= 1;
        if !chars[end].is_whitespace() {
            break;
        }
    }
    let mut start = end + 1;
    while start > 0 && is_ident(chars[start - 1]) {
        start -= 1;
    }
    chars[start..end + 1].iter().collect::<String>() == "do"
}

fn is_ident(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

/// The lints of a Lua script, over its code only: an LSL name called as LSL
/// writes it (`llSay`), a `local` never read, an assignment that makes a
/// global where a local was meant, and a `+` on a string, which Lua joins with
/// `..`. A word of a comment or a string is none of them.
pub(super) fn lua_lints(source: &str) -> Vec<Lint> {
    let chars: Vec<char> = source.chars().collect();
    let code = super::syntax::code_chars(source, Lang::Lua);
    let starts = Lines::new(&chars);
    let lua = crate::lsl_keywords::lua();
    let lsl = crate::lsl_keywords::current();
    let words = lua_words(&chars, &code);
    let mut found = Vec::new();
    let mut lint = |at: usize, rule: Rule| {
        let (line, column) = starts.position(at);
        found.push(Lint { line, column, rule });
    };
    // the script's own locals, and every name that is no global: Luau's two
    // globals, a local, a parameter, a loop's variable
    let mut locals: Vec<(usize, String)> = Vec::new();
    let mut held: std::collections::BTreeSet<String> = ["_G".to_owned(), "_VERSION".to_owned()].into();
    // LSL's name called as LSL writes it
    for (start, end, name) in &words {
        if !name.starts_with("ll")
            || name.len() < 3
            || lsl.entry(name).is_none()
            || lua.entry(name).is_some()
            || next_code(&chars, &code, *end).is_none_or(|at| chars[at] != '(')
        {
            continue;
        }
        let suggestion = format!("ll.{}", &name[2..]);
        let suggestion = lua.entry(&suggestion).is_some().then_some(suggestion);
        lint(
            *start,
            Rule::LuaLslCall {
                name: name.clone(),
                suggestion,
            },
        );
    }
    // the declarations: `local a, b = …`, `local function f(…)`, a function's
    // parameters and a `for`'s variables
    for (index, (_, end, text)) in words.iter().enumerate() {
        match text.as_str() {
            "local" => {
                let mut next = index + 1;
                if words.get(next).is_some_and(|(_, _, word)| word == "function") {
                    next += 1;
                }
                while let Some((name_at, name_end, name)) = words.get(next) {
                    locals.push((*name_at, name.clone()));
                    held.insert(name.clone());
                    let Some(comma) = next_code(&chars, &code, *name_end).filter(|at| chars[*at] == ',') else {
                        break;
                    };
                    next = words.iter().position(|(at, _, _)| *at > comma).unwrap_or(words.len());
                }
            }
            "function" => {
                // `function (x)` of an anonymous one, and the `(` after the
                // name of the others: `function f(x)`, `function M:run(x)`
                let mut after = *end;
                while let Some(next) = next_code(&chars, &code, after) {
                    if is_ident(chars[next]) || matches!(chars[next], ':' | '.') {
                        after = next + 1;
                        continue;
                    }
                    break;
                }
                let Some(open) = next_code(&chars, &code, after).filter(|at| chars[*at] == '(') else {
                    continue;
                };
                let mut depth = 0usize;
                let mut i = open;
                while i < chars.len() {
                    if code[i] {
                        match chars[i] {
                            '(' => depth += 1,
                            ')' => {
                                depth = depth.saturating_sub(1);
                                if depth == 0 {
                                    break;
                                }
                            }
                            ch if depth == 1 && is_ident(ch) && (i == 0 || !is_ident(chars[i - 1])) => {
                                let (_, name_end, name) =
                                    lua_words(&chars[i..], &code[i..]).into_iter().next().expect("a word at its start");
                                held.insert(name);
                                i += name_end;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    i += 1;
                }
            }
            "for" => {
                let mut i = *end;
                while let Some(next) = next_code(&chars, &code, i) {
                    if chars[next] == '=' || word_at(&chars, next, "in") {
                        break;
                    }
                    if !is_ident(chars[next]) {
                        i = next + 1;
                        continue;
                    }
                    let (_, name_end, name) = lua_words(&chars[next..], &code[next..])
                        .into_iter()
                        .next()
                        .expect("a word at its start");
                    held.insert(name);
                    i = next + name_end;
                }
            }
            _ => {}
        }
    }
    // a local the script declares and never reads
    for (at, name) in &locals {
        if name.starts_with('_') {
            continue;
        }
        if words.iter().filter(|(_, _, word)| word == name).count() == 1 {
            lint(*at, Rule::LuaUnusedLocal { name: name.clone() });
        }
    }
    // an assignment at the start of a statement that makes a global
    for (start, end, name) in &words {
        if held.contains(name) || brace_depth(&chars, &code, *start) > 0 {
            continue;
        }
        let Some(next) = next_code(&chars, &code, *end) else {
            continue;
        };
        let compound = matches!(chars[next], '+' | '-' | '*' | '/' | '%' | '^') && chars.get(next + 1) == Some(&'=');
        let plain = chars[next] == '=' && chars.get(next + 1) != Some(&'=');
        if (compound || plain) && starts_statement(&chars, &code, *start) {
            lint(*start, Rule::LuaGlobal { name: name.clone() });
        }
    }
    // a `+` with a string on its left
    for at in 0..chars.len() {
        if !code[at] || chars[at] != '+' {
            continue;
        }
        let previous = (0..at).rev().find(|index| !chars[*index].is_whitespace());
        if previous.is_some_and(|index| !code[index] && matches!(chars[index], '"' | '\'' | '`' | ']')) {
            lint(at, Rule::LuaStringPlus);
        }
    }
    found.sort_by_key(|lint| (lint.line, lint.column));
    found.truncate(MOST);
    found
}

/// The words of the code, as (start, end, text), in the order of the source.
fn lua_words(chars: &[char], code: &[bool]) -> Vec<(usize, usize, String)> {
    let mut words = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let boundary = at == 0 || !code[at - 1] || !is_ident(chars[at - 1]);
        if code[at] && is_ident(chars[at]) && boundary {
            let mut end = at + 1;
            while end < chars.len() && code[end] && is_ident(chars[end]) {
                end += 1;
            }
            words.push((at, end, chars[at..end].iter().collect()));
            at = end;
            continue;
        }
        at += 1;
    }
    words
}

/// The next char of the code after `at`.
fn next_code(chars: &[char], code: &[bool], at: usize) -> Option<usize> {
    (at..chars.len()).find(|index| code[*index] && !chars[*index].is_whitespace())
}

/// How many table braces hold a name at `at`: a `name =` inside one is a key,
/// not an assignment.
fn brace_depth(chars: &[char], code: &[bool], at: usize) -> usize {
    let mut depth = 0usize;
    for index in 0..at.min(chars.len()) {
        if code[index] {
            match chars[index] {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    depth
}

/// Whether a name at `at` starts a statement: after a line, a `;`, or the
/// `then`, `do` or `else` that opens one.
fn starts_statement(chars: &[char], code: &[bool], at: usize) -> bool {
    let Some(previous) = (0..at).rev().find(|index| !chars[*index].is_whitespace()) else {
        return true;
    };
    if chars[previous] == ';' || chars[previous + 1..at].contains(&'\n') {
        return true;
    }
    if !code[previous] {
        return false;
    }
    let mut start = previous;
    while start > 0 && code[start - 1] && is_ident(chars[start - 1]) {
        start -= 1;
    }
    matches!(&chars[start..=previous], ['t', 'h', 'e', 'n'] | ['d', 'o'] | ['e', 'l', 's', 'e'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(source: &str) -> Vec<(usize, Rule)> {
        let outline = crate::ui::scripts::outline::outline(source, Lang::Lsl);
        lints(source, &outline).into_iter().map(|lint| (lint.line, lint.rule)).collect()
    }

    fn lua_rules(source: &str) -> Vec<(usize, Rule)> {
        lua_lints(source).into_iter().map(|lint| (lint.line, lint.rule)).collect()
    }

    /// Lua's own lints: an LSL name called as LSL writes it, a `local` never
    /// read, an assignment that makes a global, and a string joined with `+`.
    /// A table's key is no global, a parameter or a loop's variable is none
    /// either, and no word of a comment or a string is one.
    #[test]
    fn lua_lints_name_its_own_mistakes() {
        assert_eq!(
            lua_rules("local function f()\n    llSay(0, \"hi\")\n    f()\nend\nf()\n"),
            vec![(
                1,
                Rule::LuaLslCall {
                    name: "llSay".into(),
                    suggestion: Some("ll.Say".into())
                }
            )]
        );
        // an LSL name the SLua catalog does not have keeps the hint alone
        assert_eq!(
            lua_rules("llSetTimerEvent(1.0)\n"),
            vec![(
                0,
                Rule::LuaLslCall {
                    name: "llSetTimerEvent".into(),
                    suggestion: None
                }
            )]
        );
        assert_eq!(
            lua_rules("local unused = 1\n"),
            vec![(0, Rule::LuaUnusedLocal { name: "unused".into() })]
        );
        // a global written in a block, not a table's key, not a parameter
        assert_eq!(
            lua_rules("local function f(x)\n    x = 2\n    count = 1\nend\nf(1)\n"),
            vec![(2, Rule::LuaGlobal { name: "count".into() })]
        );
        assert!(
            lua_rules("local t = {\n    a = 1,\n}\nt.a = 2\n").is_empty(),
            "{:?}",
            lua_rules("local t = {\n    a = 1,\n}\nt.a = 2\n")
        );
        assert!(lua_rules("for i = 1, 3 do\n    i = i + 1\nend\n").is_empty());
        // `+` on a string, and `..` as it should be
        assert_eq!(lua_rules("local s = \"a\" + name\nll.Say(0, s)\n"), vec![(0, Rule::LuaStringPlus)]);
        assert!(lua_rules("local s = \"a\" .. name\nll.Say(0, s)\n").is_empty());
        // a word in a comment or a string is none of them
        assert!(
            lua_rules("local s = \"llSay(0)\"\nll.Say(0, s)\n-- count = 1 ; \"a\" + b\n").is_empty(),
            "{:?}",
            lua_rules("local s = \"llSay(0)\"\nll.Say(0, s)\n-- count = 1 ; \"a\" + b\n")
        );
        // LSL's own rules never read a Lua script as LSL
        assert!(
            lua_rules("local t = ll.GetTime()\nlocal s = \"x\" .. t\nll.Say(0, s)\n").is_empty(),
            "{:?}",
            lua_rules("local t = ll.GetTime()\nlocal s = \"x\" .. t\nll.Say(0, s)\n")
        );
    }

    #[test]
    fn a_timer_and_a_sensor_too_fast_are_named() {
        assert_eq!(
            rules("default { state_entry() { llSetTimerEvent(0.05); } }"),
            vec![(0, Rule::FastTimer)]
        );
        assert!(rules("default { state_entry() { llSetTimerEvent(1.0); } }").is_empty());
        assert_eq!(
            rules("default { state_entry() { llSensorRepeat(\"a\", NULL_KEY, AGENT, 10.0, PI, 0.5); } }"),
            vec![(0, Rule::FastSensor)]
        );
    }

    #[test]
    fn a_sleep_a_broad_listen_and_a_big_integer_are_named() {
        assert_eq!(rules("default { state_entry() { llSleep(0.5); } }"), vec![(0, Rule::SleepingCall)]);
        assert_eq!(
            rules("default { state_entry() { llListen(0, \"\", NULL_KEY, \"\"); } }"),
            vec![(0, Rule::CostlyListen)]
        );
        assert!(rules("default { state_entry() { llListen(0, \"Only\", NULL_KEY, \"\"); } }").is_empty());
        assert_eq!(rules("integer big = 5000000000;"), vec![(0, Rule::IntegerPast32Bits)]);
        assert!(rules("integer ok = 2147483647;").is_empty());
        assert!(rules("float f = 5000000000.5;").is_empty(), "a float is not one");
    }

    #[test]
    fn two_calls_in_a_row_and_a_string_built_in_a_loop_are_named() {
        assert_eq!(
            rules("default { touch_start(integer n) { llSetPrimitiveParams([]); llSetPrimitiveParams([]); } }"),
            vec![(0, Rule::MergeablePrimParams)]
        );
        let loop_source = "default { state_entry() { integer i; for (i = 0; i < 3; ++i) { text = text + \"x\"; } } }";
        assert_eq!(rules(loop_source), vec![(0, Rule::StringBuild)]);
    }

    #[test]
    fn a_call_repeated_in_an_event_is_named_with_its_count() {
        let source = "default { state_entry() { llGetTime(); llGetTime(); llGetTime(); } }";
        assert_eq!(rules(source), vec![(0, Rule::RepeatedCall { count: 3 })]);
        assert!(rules("default { state_entry() { llGetTime(); llGetTime(); } }").is_empty());
    }

    #[test]
    fn a_pattern_in_a_comment_or_a_string_is_not_a_lint() {
        assert!(rules("// llSleep(0.5); llSetTimerEvent(0.01);\nstring s = \"llSleep(1)\";\n").is_empty());
    }
    /// The checks that read the whole script: a call's arguments, an event's
    /// parameters, a condition, a switch, a state.
    #[test]
    fn a_call_of_a_known_function_needs_its_arguments() {
        assert_eq!(
            rules("default { state_entry() { llSay(0); } }"),
            vec![(
                0,
                Rule::FunctionArguments {
                    name: "llSay".into(),
                    expected: 2,
                    given: 1
                }
            )]
        );
        assert!(rules("default { state_entry() { llSay(0, \"hi\"); } }").is_empty());
        assert!(
            rules("default { state_entry() { llSetPrimitiveParams([]); } }").is_empty(),
            "a list is one argument"
        );
        // a string alone is one argument, and a vector literal's commas are
        // its own
        assert!(rules("default { state_entry() { llOwnerSay(\"hi\"); } }").is_empty());
        assert!(rules("default { state_entry() { llSetText(\"hi\", <1, 1, 1>, 1.0); } }").is_empty());
        assert!(rules("default { state_entry() { llSetText(\"hi\", ZERO_VECTOR, 1.0); } }").is_empty());
    }

    #[test]
    fn an_unknown_ll_name_suggests_the_nearest_one() {
        let found = rules("default { state_entry() { llSayy(0, \"hi\"); } }");
        assert_eq!(found.len(), 1, "{found:?}");
        match &found[0].1 {
            Rule::UnknownFunction { name, suggestion } => {
                assert_eq!(name, "llSayy");
                assert_eq!(suggestion.as_deref(), Some("llSay"));
            }
            other => panic!("{other:?}"),
        }
        // a function of the script itself is no unknown built-in
        assert!(
            rules("integer llMine() { return 1; }\ndefault { state_entry() { llMine(); } }\n").is_empty(),
            "the script's own ll name"
        );
    }

    #[test]
    fn an_event_needs_its_parameters_and_is_handled_once() {
        assert_eq!(
            rules("default\n{\n    touch_start()\n    {\n    }\n}\n"),
            vec![(
                2,
                Rule::EventArguments {
                    name: "touch_start".into(),
                    expected: 1,
                    given: 0
                }
            )]
        );
        let found = rules("default\n{\n    state_entry() { }\n    state_entry() { }\n}\n");
        assert!(
            found
                .iter()
                .any(|(line, rule)| *line == 3 && matches!(rule, Rule::DuplicateEvent { name } if name == "state_entry")),
            "{found:?}"
        );
        // the same event in two states is fine
        assert!(
            rules("default { state_entry() { } }\nstate other { state_entry() { } }\n")
                .iter()
                .all(|(_, rule)| !matches!(rule, Rule::DuplicateEvent { .. }))
        );
    }

    #[test]
    fn an_assignment_in_a_condition_is_named() {
        assert_eq!(
            rules("default { state_entry() { integer x; if (x = 1) { } } }"),
            vec![(0, Rule::AssignmentInCondition)]
        );
        assert!(rules("default { state_entry() { integer x; if (x == 1) { } } }").is_empty());
        assert!(rules("default { state_entry() { integer x; if (x != 1) { } } }").is_empty());
        assert!(rules("default { state_entry() { integer x; if (x <= 1) { } } }").is_empty());
    }

    #[test]
    fn an_empty_body_is_named_but_a_do_while_is_its_own() {
        assert_eq!(rules("default { state_entry() { if (1); } }"), vec![(0, Rule::EmptyIf)]);
        assert_eq!(rules("default { state_entry() { while (1); } }"), vec![(0, Rule::EmptyWhile)]);
        // do { … } while (x); has its body, and needs the semicolon
        assert!(rules("default { state_entry() { do { } while (1); } }").is_empty());
    }

    #[test]
    fn a_switch_needs_a_default_and_one_case_per_value() {
        let found = rules("default { state_entry() { switch (1) { case 1: break; case 1: break; } } }");
        assert!(found.iter().any(|(_, rule)| matches!(rule, Rule::SwitchNoDefault)), "{found:?}");
        assert!(
            found
                .iter()
                .any(|(_, rule)| matches!(rule, Rule::SwitchDuplicateCase { value } if value == "1")),
            "{found:?}"
        );
        assert!(rules("default { state_entry() { switch (1) { case 1: break; default: break; } } }").is_empty());
    }

    #[test]
    fn a_state_already_entered_is_named() {
        assert_eq!(
            rules("default\n{\n    state_entry()\n    {\n        state default;\n    }\n}\n"),
            vec![(4, Rule::StateAlreadyCurrent { name: "default".into() })]
        );
        assert!(rules("default\n{\n    state_entry()\n    {\n        state other;\n    }\n}\n").is_empty());
    }
}
