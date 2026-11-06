//! Lua keywords of the script editor: the bundled catalog generated from
//! Second Life's SLua definitions
//! (https://raw.githubusercontent.com/secondlife/lsl-definitions/refs/heads/main/generated/lua_keywords_pretty.xml,
//! via `scripts/generate-lua-keywords.py`).
//!
//! The upstream file is already curated: LuaJIT leftovers (`getfenv`,
//! `setfenv`, `loadstring`), local-only debug helpers (`collectgarbage`,
//! `graphheap`, `graphuserheap`) and anything else the simulator does not
//! run are absent, and the LSL twins live under their `ll.` names next to
//! the Lua libraries. The highlighter looks identifiers up one by one, so
//! `ll.Say` and `string.format` contribute their member names here; the
//! dotted forms themselves can never match and are dropped by the script.
//! A bare member name never outranks grammar: controls beat constants,
//! which beat types, events, then functions.
use crate::lsl_keywords::Group;
use std::collections::HashMap;
use std::sync::OnceLock;

pub fn word(name: &str) -> Option<Group> {
    bundled().get(name).copied()
}

fn bundled() -> &'static HashMap<String, Group> {
    static BUNDLED: OnceLock<HashMap<String, Group>> = OnceLock::new();
    BUNDLED.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Words {
            controls: Vec<String>,
            types: Vec<String>,
            events: Vec<String>,
            functions: Vec<String>,
            constants: Vec<String>,
        }
        // Invariant: this JSON file is a bundled, tested build-time asset, not input.
        let catalog: Words = serde_json::from_str(include_str!("../assets/lua/keywords.json")).expect("valid bundled Lua keyword catalog");
        let mut words = HashMap::new();
        for (names, group) in [
            (catalog.functions, Group::Function),
            (catalog.events, Group::Event),
            (catalog.types, Group::Type),
            (catalog.constants, Group::Constant),
            (catalog.controls, Group::Control),
        ] {
            words.extend(names.into_iter().map(|name| (name, group)));
        }
        words
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_cover_the_language_and_skip_what_the_simulator_lacks() {
        for (name, group) in [
            ("while", Group::Control),
            ("continue", Group::Control),
            ("nil", Group::Constant),
            ("pi", Group::Constant),
            ("vector", Group::Type),
            ("LLEvents", Group::Type),
            ("touch_start", Group::Event),
            ("print", Group::Function),
            ("Say", Group::Function),
            ("format", Group::Function),
            ("type", Group::Function),
        ] {
            assert_eq!(word(name), Some(group), "{name}");
        }
        // LuaJIT leftovers, local-only debug helpers and plain Lua that
        // SLua drops stay uncolored variables.
        for name in ["getfenv", "setfenv", "loadstring", "collectgarbage", "graphheap", "goto"] {
            assert_eq!(word(name), None, "{name}");
        }
        // A bare member name never outranks grammar or constants.
        assert_eq!(word("remove"), Some(Group::Constant));
    }
}
