//! What the editor itself says about a script, after the lints of Alchemy's
//! Script Studio (`allsllintpass.cpp`): a timer too fast, a sleep that stops
//! the script, a listener that hears everything, a sensor too often, an integer
//! past 32 bits, primitive-parameter calls that could be one, a string built in
//! a loop, a call repeated in an event. Read off the text, over its code only,
//! so a pattern in a comment or a string is not one.
//!
//! They land in the Problems dock beside the simulator's own messages (and in
//! the object-wide check), the same rows, a click away from the line.

use super::crumbs;
use super::editing;

/// One word of what the editor says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rule {
    FastTimer,
    SleepingCall,
    CostlyListen,
    FastSensor,
    IntegerPast32Bits,
    MergeablePrimParams,
    StringBuild,
    RepeatedCall { count: usize },
}

impl Rule {
    /// What the row says.
    pub(super) fn message(self) -> String {
        match self {
            Rule::FastTimer => aurora_i18n::tr!("script-lint-fast-timer").to_string(),
            Rule::SleepingCall => aurora_i18n::tr!("script-lint-sleeping-call").to_string(),
            Rule::CostlyListen => aurora_i18n::tr!("script-lint-costly-listen").to_string(),
            Rule::FastSensor => aurora_i18n::tr!("script-lint-fast-sensor").to_string(),
            Rule::IntegerPast32Bits => aurora_i18n::tr!("script-lint-integer-past-32-bits").to_string(),
            Rule::MergeablePrimParams => aurora_i18n::tr!("script-lint-mergeable-prim-params").to_string(),
            Rule::StringBuild => aurora_i18n::tr!("script-lint-string-build").to_string(),
            Rule::RepeatedCall { count } => aurora_i18n::tr!("script-lint-repeated-call", count = count as i64).to_string(),
        }
    }
}

/// One place the editor found something, on a line and a column of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// The lints of a script's text, in the order of the lines.
pub(super) fn lints(source: &str) -> Vec<Lint> {
    let chars: Vec<char> = source.chars().collect();
    let code = editing::code_mask(&chars);
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
        let first = line_of(&chars, block.open + 1);
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
fn line_of(chars: &[char], at: usize) -> usize {
    chars[..at.min(chars.len())].iter().filter(|ch| **ch == '\n').count()
}

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

/// `llSensorRepeat(name, id, type, range, rate)` with a rate faster than
/// [`OFTEN`].
fn fast_sensor(arguments: &str) -> bool {
    let args: Vec<&str> = arguments.split(',').map(str::trim).collect();
    args.len() >= 5 && args[4].parse::<f64>().is_ok_and(|rate| rate > 0.0 && rate < OFTEN)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(source: &str) -> Vec<(usize, Rule)> {
        lints(source).into_iter().map(|lint| (lint.line, lint.rule)).collect()
    }

    #[test]
    fn a_timer_and_a_sensor_too_fast_are_named() {
        assert_eq!(
            rules("default { state_entry() { llSetTimerEvent(0.05); } }"),
            vec![(0, Rule::FastTimer)]
        );
        assert!(rules("default { state_entry() { llSetTimerEvent(1.0); } }").is_empty());
        assert_eq!(
            rules("default { state_entry() { llSensorRepeat(\"a\", NULL_KEY, AGENT, 10.0, 0.5); } }"),
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
}
