//! What the call under the caret takes (signature help, as an IDE's): the
//! parameters of the function being called, and the one the caret is filling.
//! Read off the catalog's signature (`void llSay(integer Channel, string
//! Text)`), over the code only, so a call in a comment is not one.

use super::{editing, reference};

/// A call the caret is inside: the function's name, its parameters as the
/// catalog writes them, and the one the caret is in (the first, from 0).
pub(super) struct Signature {
    pub name: String,
    pub params: Vec<String>,
    /// The parameter the caret is in, within the parameters.
    pub at: usize,
}

/// The call whose parentheses hold the caret, when the catalog knows the name.
pub(super) fn at_caret(source: &str, caret: usize, lua: bool) -> Option<Signature> {
    let chars: Vec<char> = source.chars().collect();
    let code = editing::code_mask(&chars);
    let caret = caret.min(chars.len());
    // the bracket that opens the call: the nearest one not closed before the
    // caret
    let mut depth = 0i32;
    let mut open = None;
    for index in (0..caret).rev() {
        if !code[index] {
            continue;
        }
        match chars[index] {
            ')' => depth += 1,
            '(' if depth == 0 => {
                open = Some(index);
                break;
            }
            '(' => depth -= 1,
            _ => {}
        }
    }
    let open = open?;
    let mut start = open;
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    let name: String = chars[start..open].iter().collect();
    if name.is_empty() {
        return None;
    }
    let entry = reference::get(&name, lua)?;
    let params = parameters(&entry.signature);
    if params.is_empty() {
        return None;
    }
    // the commas of this call before the caret, at its own depth
    let mut at = 0usize;
    let mut depth = 0i32;
    for index in open + 1..caret {
        if !code[index] {
            continue;
        }
        match chars[index] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => at += 1,
            _ => {}
        }
    }
    Some(Signature {
        name,
        at: at.min(params.len() - 1),
        params,
    })
}

/// The parameters of a signature, as written between its outermost brackets.
fn parameters(signature: &str) -> Vec<String> {
    let Some(open) = signature.find('(') else {
        return Vec::new();
    };
    let Some(close) = signature.rfind(')') else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    let mut params = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in signature[open + 1..close].chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                let param = current.trim();
                if !param.is_empty() {
                    params.push(param.to_owned());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let param = current.trim();
    if !param.is_empty() {
        params.push(param.to_owned());
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_lists_its_parameters() {
        assert_eq!(
            parameters("void llSay(integer Channel, string Text)"),
            ["integer Channel", "string Text"]
        );
        assert_eq!(parameters("void llSleep(float sec)"), ["float sec"]);
        assert_eq!(
            parameters("list llParseString2List(string src, list separators, list excludes)"),
            ["string src", "list separators", "list excludes"]
        );
        assert!(parameters("integer llGetTime()").is_empty(), "no parameters");
    }

    #[test]
    fn the_caret_in_a_call_names_the_parameter_it_is_in() {
        let source = "default { state_entry() { llSay(0, \"hi\"); } }";
        let at = source.find("0, ").unwrap() + 2;
        let signature = at_caret(source, at, false).expect("a call");
        assert_eq!(signature.name, "llSay");
        assert_eq!(signature.at, 1, "the second parameter");
        assert_eq!(signature.params.len(), 2);
        // the first parameter, right after the bracket
        let at = source.find("(0").unwrap() + 1;
        assert_eq!(at_caret(source, at, false).map(|signature| signature.at), Some(0));
        // and outside any call, nothing
        assert!(at_caret("default { state_entry() { llSay(0); } }", 3, false).is_none());
    }

    #[test]
    fn a_call_in_a_comment_or_a_string_is_not_one() {
        let source = "// llSay(0, \"hi\")\nstring s = \"llSay(1, 2)\";\n";
        assert!(at_caret(source, 12, false).is_none(), "in a comment");
        assert!(at_caret(source, 40, false).is_none(), "in a string");
    }

    #[test]
    fn the_calls_own_commas_count_and_no_others() {
        let source = "llSetLinkPrimitiveParamsFast([1, 2], 0)";
        let at = source.rfind('0').unwrap();
        let signature = at_caret(source, at, false).expect("a call");
        assert_eq!(signature.at, 1, "the second argument: the lists' commas are not its own");
    }
}
