//! LLScriptEdCore::processKeywords / setHelpPage (Firestorm, LGPL 2.1).
//! Signatures and help come from the LSL keywords in use (the bundled
//! keywords_lsl_default.xml or the region's), or SLua's for a Lua script
//! (`crate::lsl_keywords`).

pub(super) use crate::lsl_keywords::Entry;

pub(super) fn all(lua: bool) -> &'static [Entry] {
    super::syntax::catalog(lua).entries()
}

pub(super) fn get(name: &str, lua: bool) -> Option<&'static Entry> {
    super::syntax::catalog(lua).entry(name)
}

/// The entry the pointer documents, when it is over code: the word it is on
/// and the catalog's entry for it. A name inside a comment or a string is not
/// the keyword, and documents nothing.
pub(super) fn hover(source: &str, at: usize, lua: bool) -> Option<&'static Entry> {
    let chars: Vec<char> = source.chars().collect();
    let at = at.min(chars.len());
    let is_word = |c: char| c == '_' || c.is_alphanumeric() || (lua && c == '.');
    let start = chars[..at].iter().rposition(|c| !is_word(*c)).map_or(0, |i| i + 1);
    let end = chars[at..].iter().position(|c| !is_word(*c)).map_or(chars.len(), |i| at + i);
    if start == end {
        return None;
    }
    let code = super::editing::code_mask(&chars);
    if !(start..end).all(|i| code.get(i).copied().unwrap_or(false)) {
        return None;
    }
    let name: String = chars[start..end].iter().collect();
    get(&name, lua)
}

/// build_script_help_url (indra/newview/llpreviewscript.cpp): `ll.` names
/// open the LSL wiki page of the same function (`ll.Say` → `llSay`), the
/// Luau libraries luau.org, any other SLua word the Lua portal.
pub(super) fn url(name: &str, lua: bool) -> String {
    // Keywords contain only identifier characters and dots; never interpolate source text.
    let known = get(name, lua).is_some();
    if !lua || name.starts_with("ll.") {
        return if known {
            format!("https://wiki.secondlife.com/wiki/{}", name.replacen("ll.", "ll", usize::from(lua)))
        } else if lua {
            "https://wiki.secondlife.com/wiki/Lua_Alpha".into()
        } else {
            "https://wiki.secondlife.com/wiki/LSL_Portal".into()
        };
    }
    const LIBRARIES: [&str; 9] = [
        "bit32.",
        "buffer.",
        "coroutine.",
        "debug.",
        "math.",
        "os.",
        "string.",
        "table.",
        "utf8.",
    ];
    if known && LIBRARIES.iter().any(|l| name.starts_with(l)) {
        "https://luau.org/library".into()
    } else {
        "https://wiki.secondlife.com/wiki/Lua_Alpha".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_is_sorted_complete_and_contains_signatures() {
        assert!(all(false).len() > 1100);
        assert!(all(false).windows(2).all(|w| w[0].name < w[1].name));
        let say = get("llSay", false).expect("native function");
        assert_eq!(say.signature, "void llSay(integer Channel, string Text)");
        assert!(!say.tooltip.is_empty());
        assert_eq!(get("NULL_KEY", false).expect("constant").data_type, "string");
        assert_eq!(url("user supplied/../text", false), "https://wiki.secondlife.com/wiki/LSL_Portal");
        assert_eq!(url("llSay", false), "https://wiki.secondlife.com/wiki/llSay");
    }

    #[test]
    fn a_word_documents_itself_over_code_and_nothing_in_a_comment_or_a_string() {
        let source = "default { state_entry() { llSay(0, \"llSetTimerEvent\"); } } // llSay";
        // the pointer on a word's first letter
        let on = |needle: &str| {
            let byte = source.find(needle).expect("the word");
            source[..byte].chars().count()
        };
        assert_eq!(hover(source, on("llSay(0"), false).map(|entry| entry.name.as_str()), Some("llSay"));
        let in_string = source.find("\"llSetTimerEvent").expect("the string") + 1;
        assert!(
            hover(source, source[..in_string].chars().count(), false).is_none(),
            "a name in a string documents nothing"
        );
        let in_comment = source.find("// llSay").expect("the comment") + 3;
        assert!(
            hover(source, source[..in_comment].chars().count(), false).is_none(),
            "nor a name in a comment"
        );
        // a constant, and a word no catalog knows
        assert!(hover("llSay(ACTIVE, \"\")", 7, false).is_some());
        assert!(hover("llSay(unknownWord, \"\")", 7, false).is_none());
    }

    #[test]
    fn slua_help_opens_the_lsl_page_luau_or_the_lua_portal() {
        assert!(get("ll.Say", true).is_some());
        assert!(get("llSay", true).is_none());
        assert_eq!(url("ll.Say", true), "https://wiki.secondlife.com/wiki/llSay");
        assert_eq!(url("string.format", true), "https://luau.org/library");
        assert_eq!(url("ll.NotAFunction", true), "https://wiki.secondlife.com/wiki/Lua_Alpha");
        assert_eq!(url("", true), "https://wiki.secondlife.com/wiki/Lua_Alpha");
        assert_eq!(url("local", true), "https://wiki.secondlife.com/wiki/Lua_Alpha");
    }
}
