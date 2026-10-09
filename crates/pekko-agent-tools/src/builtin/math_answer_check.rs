//! Grade a model's math answer against a ground truth.
//!
//! Ported from `reasoning_from_scratch/ch03.py` of "Build a Reasoning Model
//! (From Scratch)" (Sebastian Raschka, Apache License 2.0):
//! <https://github.com/rasbt/reasoning-from-scratch>
//!
//! Extraction, normalization and part splitting follow the original closely.
//! The original's final step compares expressions symbolically with SymPy
//! (`simplify(gt - pred) == 0`); Rust has no comparable CAS, so
//! [`equality_check`] substitutes exact string comparison followed by numeric
//! evaluation. See that function for what this does and does not cover.

use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolContext, ToolDefinition, ToolError, ToolOutput};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// ── Patterns ────────────────────────────────────────────────────────────────

static RE_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-?(?:\d+/\d+|\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)").unwrap()
});
/// Chat special tokens such as `<|im_start|>`.
static RE_SPECIAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<\|[^>]+?\|>").unwrap());
static RE_MC_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z]\s*[.:]\s*(.+)$").unwrap());
static RE_DEG_BRACED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\^\s*\{\s*\\circ\s*\}").unwrap());
static RE_DEG_BARE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\^\s*\\circ").unwrap());
static RE_TEXT_WRAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\\text\{(.+?)\}$").unwrap());
/// Whitespace touching an arithmetic operator or bracket. The comma is
/// deliberately absent: an operator already separates its operands, so
/// dropping space around one cannot merge two tokens, whereas `-2, 1` and
/// `-2,1` are a comma list that SymPy refuses to parse — treating those as
/// equal would claim an equivalence the reference rejects.
static RE_WS_AROUND_OP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*([+\-*/^()=<>|])\s*").unwrap());

/// Two numbers separated only by whitespace, e.g. `1 2`. SymPy refuses to
/// parse that, so the evaluator must not silently join them into `12`.
/// Deliberately limited to digits: `3 sqrt(5)` is implicit multiplication,
/// which SymPy does evaluate, so that must still be allowed through.
static RE_OPERAND_GAP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[0-9]\s+[0-9]").unwrap());

fn superscript_digit(c: char) -> Option<char> {
    Some(match c {
        '⁰' => '0', '¹' => '1', '²' => '2', '³' => '3', '⁴' => '4',
        '⁵' => '5', '⁶' => '6', '⁷' => '7', '⁸' => '8', '⁹' => '9',
        '⁺' => '+', '⁻' => '-', '⁽' => '(', '⁾' => ')',
        _ => return None,
    })
}

/// Remove parentheses that wrap the whole expression, repeatedly.
///
/// SymPy reads them as grouping, so `(c)` and `c` are the same symbol. Only
/// a paren whose match is the final character is stripped, which leaves
/// `(a+5)(b+2)` and `(3)/(4)` untouched.
fn strip_redundant_parens(s: &str) -> String {
    let mut cur = s;
    loop {
        let bytes = cur.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'(' || bytes[bytes.len() - 1] != b')' {
            break;
        }
        let mut depth = 0usize;
        let mut matches_last = false;
        for (i, c) in cur.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        matches_last = i == cur.len() - 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !matches_last {
            break;
        }
        cur = &cur[1..cur.len() - 1];
    }
    cur.to_string()
}

/// Drop whitespace adjacent to an operator so `6 + 9i` and `6+9i` compare
/// equal. Whitespace between two operands is left alone: `1 2` must not
/// become `12`, nor `sin x` become `sinx`.
fn squeeze_operator_whitespace(s: &str) -> String {
    RE_WS_AROUND_OP.replace_all(s, "$1").into_owned()
}

/// The spelling-insensitive form used for string comparison: operator spacing
/// squeezed out and wrapping parentheses removed.
fn canonical_form(s: &str) -> String {
    strip_redundant_parens(&squeeze_operator_whitespace(s))
}

// ── Extraction ──────────────────────────────────────────────────────────────

/// Content of the last `\boxed{...}`, honouring brace nesting.
///
/// Returns `None` when there is no `\boxed`, no opening brace follows it, or
/// the braces never balance.
pub fn get_last_boxed(text: &str) -> Option<String> {
    const MARKER: &str = r"\boxed";
    let bytes = text.as_bytes();

    // rfind is a backward byte scan and lands on a char boundary because
    // MARKER is ASCII. Everything below walks bytes from there, so a long
    // response is never decoded or copied just to locate its last answer.
    let start = text.rfind(MARKER)?;
    let mut i = start + MARKER.len();

    // Whitespace may be non-ASCII, so decode — but only the few chars here.
    for c in text[i..].chars() {
        if c.is_whitespace() {
            i += c.len_utf8();
        } else {
            break;
        }
    }

    if bytes.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let content_start = i;

    // `{` and `}` are ASCII, which UTF-8 never produces as a continuation
    // byte, so counting depth over raw bytes cannot mis-fire inside a
    // multi-byte character.
    let mut depth = 1usize;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }

    // Both ends sit just inside ASCII braces, so these are char boundaries.
    Some(text[content_start..i - 1].to_string())
}

/// What to do when the text contains no `\boxed{...}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// Last number, else the whole text.
    #[default]
    NumberThenFull,
    /// Last number, else empty.
    NumberOnly,
    /// Empty.
    None,
}

/// The answer the model settled on: the last boxed expression, or a fallback.
pub fn extract_final_candidate(text: &str, fallback: Fallback) -> String {
    if text.is_empty() {
        return String::new();
    }

    if let Some(boxed) = get_last_boxed(text.trim()) {
        if !boxed.is_empty() {
            return boxed.trim().trim_matches(|c| c == '$' || c == ' ').to_string();
        }
    }

    match fallback {
        Fallback::NumberThenFull | Fallback::NumberOnly => match last_number(text) {
            Some(m) => m.to_string(),
            None if fallback == Fallback::NumberThenFull => text.to_string(),
            None => String::new(),
        },
        Fallback::None => String::new(),
    }
}

/// Could this byte be part of a `RE_NUMBER` match?
fn is_number_byte(c: u8) -> bool {
    c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'/' | b'e' | b'E')
}

/// The last `RE_NUMBER` match, without enumerating the earlier ones.
///
/// `find_iter(text).last()` walks every number in the response to report the
/// trailing one, which dominates extraction for the ~39% of outputs that carry
/// no `\boxed{...}`. Two facts make a bounded search give the same answer:
/// a lone digit is itself a match, so the final match must contain the last
/// digit; and a match is built only from the bytes above, so it cannot reach
/// back across one that is not. Searching back to the nearest such byte
/// therefore starts where no earlier match can still be open, and the scan
/// over that suffix yields exactly the match the full pass would.
fn last_number(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    // Continuation bytes are >= 0x80, so this cannot land inside a character.
    let last_digit = b.iter().rposition(u8::is_ascii_digit)?;
    let mut start = last_digit;
    while start > 0 && is_number_byte(b[start - 1]) {
        start -= 1;
    }
    // Every byte stepped over is ASCII, so `start` is a character boundary.
    RE_NUMBER.find_iter(&text[start..]).last().map(|m| m.as_str())
}

// ── Normalization ───────────────────────────────────────────────────────────

/// Canonicalize an answer string so two spellings of the same value compare
/// equal: strips chat tokens, math delimiters and degree markers, rewrites
/// `\frac`/`\sqrt`/superscripts into plain arithmetic, and lowercases.
///
/// The original applies ~20 regex substitutions in sequence, each scanning and
/// reallocating the whole string. This folds the local rewrites into one
/// character pass. Three things cannot move into it, because the reference's
/// ordering is observable:
///
/// * the multiple-choice and `\text{...}` unwraps are anchored to the whole
///   string, and degree removal runs between them, so `\text{x}^\circ`
///   unwraps while `\text{x}b` does not;
/// * the mixed-number and thousands rules apply to the *rewritten* text, so
///   they run afterwards over the (much shorter) output.
///
/// Each of those is guarded by a cheap `contains` check, so the common case —
/// a short expression with no chat token, label or degree marker — reaches the
/// main pass directly.
pub fn normalize_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // ── anchored pre-steps, each skipped unless its marker is present ──
    let stripped;
    let mut s = if text.contains("<|") {
        stripped = RE_SPECIAL.replace_all(text, "").into_owned();
        stripped.trim()
    } else {
        text.trim()
    };

    // "c. 3" -> "3"
    let mc;
    if s.as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic()) {
        if let Some(c) = RE_MC_LABEL.captures(s) {
            mc = c[1].to_string();
            s = &mc;
        }
    }

    let degreeless;
    if s.contains("circ") || s.contains('°') {
        let t = RE_DEG_BRACED.replace_all(s, "");
        let t = RE_DEG_BARE.replace_all(&t, "");
        degreeless = t.replace('°', "");
        s = &degreeless;
    }

    // Unwrap \text{...} only when it wraps the entire string.
    let unwrapped;
    if s.starts_with("\\text{") && s.ends_with('}') {
        if let Some(c) = RE_TEXT_WRAP.captures(s) {
            unwrapped = c[1].to_string();
            s = &unwrapped;
        }
    }

    // ── the single pass ──
    let mut out = String::with_capacity(s.len() + 8);
    normalize_body(s, &mut out);

    // ── fractions and leftover braces ──
    let out = if out.contains('{') || out.contains('}') || out.contains("frac") {
        apply_fractions(&out)
    } else {
        out
    };

    // ── digit grouping, over the rewritten text ──
    let out = if out.as_bytes().iter().any(|c| *c == b',' || c.is_ascii_whitespace()) {
        fix_digit_groups(&out)
    } else {
        out
    };

    let t = out.trim();
    if t.len() == out.len() {
        out
    } else {
        t.to_string()
    }
}

/// Can a unicode superscript attach to this character as its base?
fn is_superscript_base(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ')' | ']' | '}')
}

/// The character ending at byte `i`.
fn char_before(s: &str, i: usize) -> Option<char> {
    s[..i].chars().next_back()
}

/// Walk `s` once, appending the canonical form to `out`.
///
/// Recurses into `\frac` and `\sqrt` arguments, writing into the same buffer,
/// so nothing is allocated per group.
fn normalize_body(s: &str, out: &mut String) {
    let b = s.as_bytes();
    let mut i = 0;

    while i < b.len() {
        let c = b[i];

        // multi-byte: superscripts, degree sign, multiplication dots
        if c >= 0x80 {
            let ch = s[i..].chars().next().unwrap();
            if let Some(d) = superscript_digit(ch) {
                // The reference inserts ** once per run, when a base precedes it.
                if char_before(s, i).is_some_and(is_superscript_base) {
                    out.push_str("**");
                }
                out.push(d);
                i += ch.len_utf8();
                while let Some(next) = s[i..].chars().next() {
                    match superscript_digit(next) {
                        Some(d2) => {
                            out.push(d2);
                            i += next.len_utf8();
                        }
                        None => break,
                    }
                }
            } else if ch == '°' {
                i += ch.len_utf8();
            } else if ch == '\u{00B7}' || ch == '\u{00D7}' {
                out.push('*');
                i += ch.len_utf8();
            } else {
                out.extend(ch.to_lowercase());
                i += ch.len_utf8();
            }
            continue;
        }

        match c {
            b'\\' => match handle_command(s, i, out) {
                Some(adv) => i += adv,
                None => {
                    out.push('\\');
                    i += 1;
                }
            },
            // ^{\circ} and ^\circ are gone by now, so any caret is an exponent.
            b'^' => {
                out.push_str("**");
                i += 1;
            }
            // \% became %, and both % and $ are dropped. Braces survive this
            // pass: the fraction rule needs them, and it runs next.
            b'$' | b'%' => i += 1,
            _ => {
                out.push(c.to_ascii_lowercase() as char);
                i += 1;
            }
        }
    }
}

/// Skip ASCII and unicode whitespace from `i`, returning the new offset.
fn skip_ws(s: &str, mut i: usize) -> usize {
    while let Some(c) = s[i..].chars().next() {
        if c.is_whitespace() {
            i += c.len_utf8();
        } else {
            break;
        }
    }
    i
}

/// A `{...}` group whose body may not contain braces, as `[^{}]+` requires.
fn flat_group(s: &str, i: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'{') {
        return None;
    }
    let body_start = i + 1;
    let mut j = body_start;
    while j < b.len() && b[j] != b'{' && b[j] != b'}' {
        j += 1;
    }
    if j == body_start || b.get(j) != Some(&b'}') {
        return None; // empty, nested, or unterminated
    }
    Some((&s[body_start..j], j + 1))
}

/// A `{...}` group whose body may contain `{`, as `[^}]*` allows, and may be empty.
fn loose_group(s: &str, i: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'{') {
        return None;
    }
    let body_start = i + 1;
    let mut j = body_start;
    while j < b.len() && b[j] != b'}' {
        j += 1;
    }
    if b.get(j) != Some(&b'}') {
        return None;
    }
    Some((&s[body_start..j], j + 1))
}

/// A bare argument: one or more characters that are not a backslash, brace or space.
fn bare_arg(s: &str, i: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    let mut j = i;
    while j < b.len() {
        let c = b[j];
        if c == b'\\' || c == b'{' || c == b'}' || c.is_ascii_whitespace() {
            break;
        }
        j += 1;
    }
    if j == i {
        None
    } else {
        Some((&s[i..j], j))
    }
}

/// The argument of `\sqrt`: a braced group first, as the reference tries,
/// then a bare token.
fn sqrt_arg(s: &str, after: usize) -> Option<(&str, usize)> {
    let p = skip_ws(s, after);
    if let Some(found) = loose_group(s, p) {
        return Some(found);
    }
    if p == after {
        return None; // the bare form requires whitespace
    }
    bare_arg(s, p)
}

/// Rewrite one LaTeX command at `i`, returning the bytes consumed.
///
/// `None` means this is not a command the reference rewrites, so the caller
/// emits the backslash literally — matching the original, which leaves an
/// unrecognised command in place.
fn handle_command(s: &str, i: usize, out: &mut String) -> Option<usize> {
    let rest = &s[i + 1..];

    // Spacing commands and math delimiters: dropped outright.
    for lit in ["left", "right"] {
        if let Some(tail) = rest.strip_prefix(lit) {
            let after = s.len() - tail.len();
            return Some(skip_ws(s, after) - i);
        }
    }
    for lit in [",", "!", ";", ":", "(", ")", "[", "]", "%"] {
        if rest.starts_with(lit) {
            return Some(1 + lit.len());
        }
    }
    if rest.starts_with("cdot") {
        out.push('*');
        return Some(1 + 4);
    }

    // The fraction rules run after this pass, because SymPy's ordering is
    // observable: \sqrt{21} loses its braces first, which is the only reason
    // \frac{\sqrt{21}}{5} can match a rule whose groups reject braces.
    // \dfrac and \tfrac are folded into \frac here, as the reference does.
    for lit in ["dfrac", "tfrac"] {
        if rest.starts_with(lit) {
            out.push_str("\\frac");
            return Some(1 + lit.len());
        }
    }

    if rest.starts_with("sqrt") {
        let after = i + 1 + 4;
        // \sqrt{a} takes priority over \sqrt a
        if let Some((body, end)) = sqrt_arg(s, after) {
            out.push_str("sqrt(");
            normalize_body(body, out);
            out.push(')');
            return Some(end - i);
        }
        out.push_str("\\sqrt");
        return Some(1 + 4);
    }

    None
}

/// Rewrite `\frac` and drop any remaining braces.
///
/// Scans left to right and, on a `\frac` whose arguments do not match,
/// advances one byte and keeps looking — the same way a single `re.sub` pass
/// behaves, which is why `\frac{\frac{1}{2}}{3}` rewrites the inner fraction
/// and leaves the outer one alone.
fn apply_fractions(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;

    while i < b.len() {
        if b[i] == b'{' || b[i] == b'}' {
            i += 1;
            continue;
        }
        if b[i] == b'\\' && s[i + 1..].starts_with("frac") {
            let after = i + 1 + 4;
            let braced = (|| {
                let p = skip_ws(s, after);
                let (g1, p) = flat_group(s, p)?;
                let p = skip_ws(s, p);
                let (g2, p) = flat_group(s, p)?;
                Some((g1, g2, p))
            })();
            let bare = || {
                let p = skip_ws(s, after);
                if p == after {
                    return None; // the bare form requires whitespace
                }
                let (g1, p) = bare_arg(s, p)?;
                let q = skip_ws(s, p);
                if q == p {
                    return None;
                }
                let (g2, p) = bare_arg(s, q)?;
                Some((g1, g2, p))
            };
            if let Some((g1, g2, end)) = braced.or_else(bare) {
                out.push('(');
                out.push_str(g1);
                out.push_str(")/(");
                out.push_str(g2);
                out.push(')');
                i = end;
                continue;
            }
        }
        // Copy one character, not one byte, to keep UTF-8 intact.
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Apply the two rules that read the rewritten text: a mixed number becomes a
/// sum, and thousands separators are dropped.
fn fix_digit_groups(s: &str) -> String {
    let b = s.as_bytes();
    // Only ASCII is inserted or removed, so the bytes stay valid UTF-8.
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;

    while i < b.len() {
        let c = b[i];
        let prev_is_digit = out.last().is_some_and(u8::is_ascii_digit);

        // "1,234" -> "1234", but only before exactly three digits.
        if c == b',' && prev_is_digit {
            let d = &b[i + 1..];
            let three = d.len() >= 3 && d[..3].iter().all(u8::is_ascii_digit);
            if three && (d.len() == 3 || !d[3].is_ascii_digit()) {
                i += 1;
                continue;
            }
        }

        // "1 1/2" -> "1+1/2"
        if c.is_ascii_whitespace() && prev_is_digit {
            let ws_end = {
                let mut j = i;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                j
            };
            let num_end = {
                let mut j = ws_end;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                j
            };
            if num_end > ws_end && b.get(num_end) == Some(&b'/') {
                let den_end = {
                    let mut j = num_end + 1;
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    j
                };
                if den_end > num_end + 1 {
                    out.push(b'+');
                    i = ws_end;
                    continue;
                }
            }
        }

        out.push(c);
        i += 1;
    }

    String::from_utf8(out).expect("only ASCII was added or removed")
}

/// Split `"(a, b)"` into its items so tuple answers compare element-wise.
/// Anything else is returned as a single part.
pub fn split_into_parts(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = text.chars().collect();
    if chars.len() >= 2 {
        let first = chars[0];
        let last = chars[chars.len() - 1];
        let inner: String = chars[1..chars.len() - 1].iter().collect();
        if matches!(first, '(' | '[') && matches!(last, ')' | ']') && inner.contains(',') {
            let items: Vec<String> = inner.split(',').map(|p| p.trim().to_string()).collect();
            if items.iter().all(|p| !p.is_empty()) {
                return items;
            }
        }
    }
    vec![text.to_string()]
}

// ── Equivalence ─────────────────────────────────────────────────────────────

/// Are two normalized expressions the same answer?
///
/// Exact string equality first, then the same comparison on a canonical form
/// (operator spacing squeezed, wrapping parentheses dropped), then numeric
/// evaluation of both sides.
///
/// This replaces the original's SymPy step and is deliberately narrower.
/// Covered: integers, decimals, fractions, `sqrt`, powers, parentheses and
/// implicit multiplication of numerics — so `1/2` matches `0.5` and `2+3`
/// matches `5`. Not covered: any expression with a free variable or an exact
/// symbol such as `pi`, where SymPy would still prove `2x+3x == 5x`. Those
/// match only when the normalized strings are identical.
///
/// The asymmetry is deliberate: relative to the SymPy reference this can miss
/// an equivalence, but it never reports one that does not hold.
pub fn equality_check(gt: &str, pred: &str) -> bool {
    if gt == pred {
        return true;
    }
    // SymPy's parser ignores whitespace around operators and treats a wrapping
    // paren as grouping; normalize_text preserves both, so retry the string
    // comparison on a canonical form before giving up on strings.
    if canonical_form(gt) == canonical_form(pred) {
        return true;
    }
    match (eval_expr(gt), eval_expr(pred)) {
        (Some(a), Some(b)) => values_equal(a, b),
        _ => false,
    }
}

/// Do two evaluated expressions denote the same number?
///
/// Exact f64 comparison, with no tolerance. A tolerance would be unsound
/// here: SymPy reads a decimal literal as exact, so `0.3333333333` is not
/// `1/3` however many digits it carries, and any window wide enough to
/// bridge that gap grades a truncated answer as correct. Measured against
/// the reference, exact comparison loses nothing — two spellings of one
/// value agree to the last bit (`sqrt(8)/2` and `sqrt(2)`, `(3)/(4)` and
/// `0.75`, even `1/3+1/3+1/3` and `1`).
fn values_equal(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && a == b
}

/// Decide whether `pred_text` answers the question as well as `gt_text`.
///
/// Both sides are extracted-and-normalized, split into parts, and compared
/// element-wise; every part must match and the part counts must agree.
pub fn grade_answer(pred_text: &str, gt_text: &str) -> bool {
    let gt_parts = split_into_parts(&normalize_text(gt_text));
    let pred_parts = split_into_parts(&normalize_text(pred_text));

    if gt_parts.is_empty() || pred_parts.is_empty() || gt_parts.len() != pred_parts.len() {
        return false;
    }
    gt_parts
        .iter()
        .zip(pred_parts.iter())
        .all(|(g, p)| equality_check(g, p))
}

// ── Numeric expression evaluator ────────────────────────────────────────────
//
// Recursive descent over the subset of arithmetic that normalize_text emits.
// Returns None for anything it cannot evaluate — notably free variables —
// which callers treat as "not comparable" rather than "not equal".

/// Guard against pathological input, mirroring the original's length check.
const MAX_EXPR_LEN: usize = 2000;

fn eval_expr(s: &str) -> Option<f64> {
    if s.is_empty() || s.len() > MAX_EXPR_LEN {
        return None;
    }
    // Whitespace is dropped below, which would turn `1 2` into `12`. SymPy
    // treats that as a parse error rather than a number, so refuse it here
    // instead of inventing a value.
    if RE_OPERAND_GAP.is_match(s) {
        return None;
    }
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut p = Parser { chars: &chars, pos: 0 };
    let v = p.expr()?;
    if p.pos == p.chars.len() && v.is_finite() {
        Some(v)
    } else {
        None
    }
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, s: &str) -> bool {
        let n = s.chars().count();
        if self.pos + n <= self.chars.len()
            && self.chars[self.pos..self.pos + n].iter().copied().eq(s.chars())
        {
            self.pos += n;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<f64> {
        let mut acc = self.term()?;
        loop {
            if self.eat('+') {
                acc += self.term()?;
            } else if self.eat('-') {
                acc -= self.term()?;
            } else {
                return Some(acc);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut acc = self.unary()?;
        loop {
            // `**` is exponentiation, so only a lone `*` multiplies.
            if self.peek() == Some('*') && self.chars.get(self.pos + 1) != Some(&'*') {
                self.pos += 1;
                acc *= self.unary()?;
            } else if self.eat('/') {
                let d = self.unary()?;
                if d == 0.0 {
                    return None;
                }
                acc /= d;
            } else if self.starts_implicit_factor() {
                acc *= self.unary()?;
            } else {
                return Some(acc);
            }
        }
    }

    /// `2(3+4)` and `2sqrt(2)` multiply without an operator, as SymPy's
    /// implicit-multiplication transformation allows.
    fn starts_implicit_factor(&self) -> bool {
        matches!(self.peek(), Some('(')) || self.looks_like_name()
    }

    fn looks_like_name(&self) -> bool {
        matches!(self.peek(), Some(c) if c.is_ascii_alphabetic())
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat_str("**") {
            // Right associative: 2**3**2 == 2**9.
            let exp = self.unary()?;
            let r = base.powf(exp);
            return if r.is_finite() { Some(r) } else { None };
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            let v = self.expr()?;
            if !self.eat(')') {
                return None;
            }
            return Some(v);
        }

        if self.eat_str("sqrt(") {
            let v = self.expr()?;
            if !self.eat(')') || v < 0.0 {
                return None;
            }
            return Some(v.sqrt());
        }

        // `pi` is deliberately NOT evaluated to a float. SymPy treats it as an
        // exact symbol, so `simplify(pi - 3.14159...) != 0`; evaluating it here
        // would grade a truncated decimal as correct when the reference does
        // not. Falling through leaves it to string equality, matching SymPy.
        if self.looks_like_name() {
            // A free variable: not numerically evaluable.
            return None;
        }

        self.number()
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.eat('.') {
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if self.pos == start {
            return None;
        }
        // Exponent, but only when digits actually follow.
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save = self.pos;
            self.pos += 1;
            if self.peek() == Some('+') || self.peek() == Some('-') {
                self.pos += 1;
            }
            if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }
}

// ── Tool ────────────────────────────────────────────────────────────────────

/// Checks a reasoning model's final answer against a known-correct one.
pub struct MathAnswerCheckTool;

#[derive(Debug, Deserialize)]
pub struct MathAnswerCheckInput {
    /// Raw model output; `\boxed{...}` is extracted from it.
    pub prediction: String,
    /// The known-correct answer.
    pub ground_truth: String,
    #[serde(default)]
    pub fallback: Fallback,
    /// Skip extraction and treat `prediction` as the final answer already.
    #[serde(default)]
    pub prediction_is_final: bool,
}

#[derive(Debug, Serialize)]
pub struct MathAnswerCheckResult {
    pub matched: bool,
    pub extracted: String,
    pub normalized_prediction: String,
    pub normalized_ground_truth: String,
    pub parts_compared: usize,
    /// How the verdict was reached: `exact`, `numeric`, `part_count_mismatch`
    /// or `not_equivalent`.
    pub method: String,
}

#[async_trait]
impl Tool for MathAnswerCheckTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "math_answer_check".to_string(),
            description: "수학 답안이 정답과 일치하는지 검증합니다. \
                          모델 출력에서 \\boxed{...}를 추출해 정규화한 뒤, \
                          문자열 일치와 수치 평가로 동등성을 판정합니다. \
                          자유 변수가 포함된 기호식은 문자열이 같을 때만 일치로 봅니다."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prediction": {
                        "type": "string",
                        "description": "모델의 원본 출력. \\boxed{...}가 있으면 그것을 최종 답안으로 사용합니다."
                    },
                    "ground_truth": {
                        "type": "string",
                        "description": "정답 문자열 (예: 42, \\frac{3}{4}, (1,2))"
                    },
                    "fallback": {
                        "type": "string",
                        "enum": ["number_then_full", "number_only", "none"],
                        "description": "\\boxed{...}가 없을 때의 추출 방식",
                        "default": "number_then_full"
                    },
                    "prediction_is_final": {
                        "type": "boolean",
                        "description": "true면 추출을 건너뛰고 prediction을 이미 최종 답안으로 취급합니다.",
                        "default": false
                    }
                },
                "required": ["prediction", "ground_truth"],
                "additionalProperties": false
            }),
            required_permissions: vec![],
            timeout_ms: 1000,
            idempotent: true,
        }
    }

    fn validate_input(&self, input: &serde_json::Value) -> Result<(), ToolError> {
        let obj = input
            .as_object()
            .ok_or_else(|| ToolError::ValidationFailed("input must be an object".into()))?;
        for key in ["prediction", "ground_truth"] {
            match obj.get(key) {
                Some(serde_json::Value::String(_)) => {}
                Some(_) => {
                    return Err(ToolError::ValidationFailed(format!("'{key}' must be a string")))
                }
                None => return Err(ToolError::ValidationFailed(format!("'{key}' is required"))),
            }
        }
        Ok(())
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let input: MathAnswerCheckInput = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(e.to_string()))?;

        let extracted = if input.prediction_is_final {
            input.prediction.clone()
        } else {
            extract_final_candidate(&input.prediction, input.fallback)
        };

        let norm_pred = normalize_text(&extracted);
        let norm_gt = normalize_text(&input.ground_truth);
        let gt_parts = split_into_parts(&norm_gt);
        let pred_parts = split_into_parts(&norm_pred);

        let (matched, method) = if gt_parts.is_empty()
            || pred_parts.is_empty()
            || gt_parts.len() != pred_parts.len()
        {
            (false, "part_count_mismatch")
        } else if gt_parts == pred_parts {
            (true, "exact")
        } else if gt_parts
            .iter()
            .zip(pred_parts.iter())
            .all(|(g, p)| equality_check(g, p))
        {
            (true, "numeric")
        } else {
            (false, "not_equivalent")
        };

        let result = MathAnswerCheckResult {
            matched,
            extracted,
            normalized_prediction: norm_pred,
            normalized_ground_truth: norm_gt,
            parts_compared: gt_parts.len(),
            method: method.to_string(),
        };

        Ok(ToolOutput::success(
            serde_json::to_value(result).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?,
        ))
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────
//
// The case tables mirror `tests/test_ch03.py` in the source repository, so a
// regression here means the port drifted from the original's behaviour.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_boxed_handles_nesting_and_malformed_input() {
        let cases: &[(&str, Option<&str>)] = &[
            (r"foo \boxed{42}", Some("42")),
            (r"\boxed{a} bla \boxed{b+c}", Some("b+c")),
            (r"noise \boxed   {  x^2 } end", Some("  x^2 ")),
            (r"\boxed{outer {inner} ok}", Some("outer {inner} ok")),
            (r"nothing here", None),
            (r"\boxed  not_brace", None),
            (r"\boxed{unbalanced", None),
        ];
        for (text, expected) in cases {
            assert_eq!(
                get_last_boxed(text).as_deref(),
                *expected,
                "input: {text:?}"
            );
        }
    }

    #[test]
    fn extracts_the_final_answer() {
        let cases: &[(&str, &str)] = &[
            ("Steps...\n\\boxed{3/4}\nDone.", "3/4"),
            (r"Reasoning...\boxed{\frac{3}{4}}", r"\frac{3}{4}"),
            (r"Compute...\boxed{\sqrt{2}}", r"\sqrt{2}"),
            (r"Tuple case...\boxed{(1,2)}", "(1,2)"),
            (r"Earlier box \boxed{1/3}, later \boxed{2/3}", "2/3"),
            (r"Noisy \boxed   {  x^2 } trailing text", "x^2"),
            (r"Nested braces \boxed{outer {inner} ok}", "outer {inner} ok"),
            (r"In math mode: $ \boxed{ \dfrac{14}{3} } $", r"\dfrac{14}{3}"),
            // No \boxed: fall back to the last number, else the whole text.
            ("Some steps...\nFinal Answer: 14/3", "14/3"),
            ("All done. 1 \nFinal 2 answer: ", "2"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                extract_final_candidate(input, Fallback::NumberThenFull),
                *expected,
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn bounded_last_number_matches_a_full_scan() {
        // The bounded search must agree with enumerating every match, which is
        // what the reference does. These are the shapes where a naive scan
        // backwards from the end would disagree.
        let cases: &[(&str, Option<&str>)] = &[
            ("answer: 42", Some("42")),
            ("1 then 2 then 3", Some("3")),
            // A trailing exponent must not be read as a bare number.
            ("value 1.5e10", Some("1.5e10")),
            ("value 1.5E-10", Some("1.5E-10")),
            // A fraction is one match, not two.
            ("ratio 3/4", Some("3/4")),
            ("a 1/2 b 5/6", Some("5/6")),
            // A minus sign belongs to the following number only when free.
            ("x-1", Some("-1")),
            ("9-1", Some("-1")),
            ("-7", Some("-7")),
            // A letter that is also an exponent marker must not confuse it.
            ("the5", Some("5")),
            ("1e5e7", Some("7")),
            // Trailing non-numeric text is skipped.
            ("12345 units", Some("12345")),
            ("3.25.", Some("3.25")),
            ("no digits here", None),
            ("", None),
        ];
        for (text, expected) in cases {
            assert_eq!(last_number(text), *expected, "input: {text:?}");
            // ...and the bounded result is the full scan's result.
            assert_eq!(
                last_number(text),
                RE_NUMBER.find_iter(text).last().map(|m| m.as_str()),
                "bounded search disagreed with the full scan on {text:?}"
            );
        }
    }

    #[test]
    fn bounded_search_agrees_on_long_responses() {
        // A realistic shape: a long chain of reasoning, numbers throughout,
        // no \boxed{...} at the end.
        let mut text = String::new();
        for i in 0..400 {
            text.push_str(&format!("step {i}: carry {}/{} then ", i + 1, i + 2));
        }
        text.push_str("so the value is -12.5e3 at last");
        assert_eq!(
            last_number(&text),
            RE_NUMBER.find_iter(&text).last().map(|m| m.as_str())
        );
        assert_eq!(last_number(&text), Some("-12.5e3"));
    }

    #[test]
    fn normalizes_latex_and_spacing() {
        let cases: &[(&str, &str)] = &[
            ("  3/4  ", "3/4"),
            ("\n\t  (1, 2)  \t", "(1, 2)"),
            ("$2/3$", "2/3"),
            (r"\( 2/3 \)", "2/3"),
            (r"\left(1,\,2\right)", "(1,2)"),
            (r"\frac{3}{4}", "(3)/(4)"),
            (r"\dfrac{14}{3}", "(14)/(3)"),
            ("(3)/(4)", "(3)/(4)"),
            ("c. 3", "3"),
            ("b: 2", "2"),
            (r"\sqrt{2}", "sqrt(2)"),
            (r"{x}", "x"),
            (r"{ (1, 2) }", "(1, 2)"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_text(raw), *expected, "input: {raw:?}");
        }
    }

    #[test]
    fn lookbehind_replacements_match_the_original() {
        // The regex crate has no lookbehind; these are the two patterns that
        // had to be restructured.
        assert_eq!(normalize_text("1 1/2"), "1+1/2");
        assert_eq!(normalize_text("1,234"), "1234");
        assert_eq!(normalize_text("1,234,567"), "1234567");
        // A comma that is not a thousands separator must survive.
        assert_eq!(normalize_text("(1,2)"), "(1,2)");
    }

    #[test]
    fn splits_only_real_tuples() {
        assert_eq!(split_into_parts("(1, 2)"), vec!["1", "2"]);
        assert_eq!(split_into_parts("[3,4]"), vec!["3", "4"]);
        assert_eq!(split_into_parts("(1,)"), vec!["(1,)"]); // empty item
        assert_eq!(split_into_parts("42"), vec!["42"]);
        assert!(split_into_parts("").is_empty());
    }

    #[test]
    fn numeric_equivalence_without_a_cas() {
        assert!(grade_answer("1/2", "0.5"));
        assert!(grade_answer("2+3", "5"));
        assert!(grade_answer(r"\frac{3}{4}", "0.75"));
        assert!(grade_answer(r"\sqrt{9}", "3"));
        assert!(grade_answer("2(3+4)", "14"));
        assert!(grade_answer("sqrt(8)/2", "sqrt(2)"));
        assert!(grade_answer("1,234", "1234"));
        assert!(grade_answer("-1/2", "-0.5"));
        assert!(grade_answer("(1,2)", "(1, 2)"));

        assert!(!grade_answer("(1,2)", "(2,1)"));
        assert!(!grade_answer("5", "6"));

        // A decimal literal is exact to SymPy, so a truncated one is a
        // different number however many digits it carries.
        assert!(!grade_answer("1/3", "0.333"));
        assert!(!grade_answer("0.3333333333", "1/3"));
        assert!(!grade_answer("0.33333333333333", "1/3"));
        // ...until it is the full f64 expansion, which SymPy also accepts.
        assert!(grade_answer("0.3333333333333333", "1/3"));
        // Exactly representable decimals match.
        assert!(grade_answer("0.5", "1/2"));
        assert!(grade_answer("0.75", r"\frac{3}{4}"));
        assert!(grade_answer("2.0", "2"));
        // Different spellings of one value agree to the last bit.
        assert!(grade_answer("sqrt(8)/2", "sqrt(2)"));
        assert!(grade_answer("1/3+1/3+1/3", "1"));
    }

    #[test]
    fn whitespace_around_operators_is_ignored() {
        // SymPy's parser ignores this spacing, so these are real equivalences
        // the string comparison would otherwise miss. All have free variables
        // or an imaginary unit, so numeric evaluation cannot rescue them.
        assert!(grade_answer("6 + 9i", "6+9i"));
        assert!(grade_answer("2k + 2", "2k+2"));
        assert!(grade_answer("x^3 + 3x - 6", "x^3+3x-6"));
        assert!(grade_answer("6r^2 -4r -24", "6r^2-4r-24"));
        assert!(grade_answer("(a + 5)(b + 2)", "(a+5)(b+2)"));
        assert!(grade_answer(r"137\frac{1}{2}", r"137 \frac{1}{2}"));
    }

    #[test]
    fn wrapping_parentheses_are_grouping() {
        // SymPy parses (c) as the symbol c, so a multiple-choice answer
        // spelled either way is the same answer.
        assert!(grade_answer("C", r"\text{(C)}"));
        assert!(grade_answer("E", r"\text{(E)}"));
        assert!(grade_answer("(x)", "x"));
        assert!(grade_answer("((x))", "x"));

        // Only a paren matching the final character is redundant.
        assert_eq!(strip_redundant_parens("(a+5)(b+2)"), "(a+5)(b+2)");
        assert_eq!(strip_redundant_parens("(3)/(4)"), "(3)/(4)");
        assert_eq!(strip_redundant_parens("sqrt(2)"), "sqrt(2)");
        assert_eq!(strip_redundant_parens("(a+(b))"), "a+(b)");
        assert_eq!(strip_redundant_parens("(x"), "(x");

        // Distinct answers must stay distinct.
        assert!(!grade_answer("C", r"\text{(D)}"));
        assert!(!grade_answer("(a+5)(b+2)", "(a+5)(b+3)"));
    }

    #[test]
    fn whitespace_between_operands_is_significant() {
        // No operator separates these, so squeezing would merge two tokens
        // into one and claim an equivalence the reference rejects.
        assert!(!grade_answer("1 2", "12"));
        assert!(!grade_answer("sin x", "sinx"));
        // A bare comma list is not a tuple (no brackets) and SymPy will not
        // parse it, so the reference says these differ.
        assert!(!grade_answer("-2, 1", "-2,1"));
    }

    #[test]
    fn free_variables_fall_back_to_string_equality() {
        // Identical spellings still match.
        assert!(grade_answer("2x", "2x"));
        // These need a CAS; the original proves them, this port does not.
        assert!(!grade_answer("2x+3x", "5x"));
        assert!(!grade_answer("2*x", "x*2"));
        // pi is an exact symbol, matching SymPy's behaviour.
        assert!(!grade_answer("pi", "3.141592653589793"));
        assert!(grade_answer("pi", "pi"));
    }

    #[test]
    fn rejects_unparseable_and_oversized_input() {
        assert_eq!(eval_expr("sqrt("), None);
        assert_eq!(eval_expr("??"), None);
        assert_eq!(eval_expr("2**"), None);
        assert_eq!(eval_expr(""), None);
        assert_eq!(eval_expr(&"1".repeat(MAX_EXPR_LEN + 1)), None);
        assert_eq!(eval_expr("1/0"), None);
        // `1 2` must not be read as 12.
        assert_eq!(eval_expr("1 2"), None);
        assert_eq!(eval_expr("12"), Some(12.0));
        // ...but implicit multiplication across a space still evaluates.
        assert!(eval_expr("3 sqrt(4)").is_some_and(|v| (v - 6.0).abs() < 1e-9));
    }

    #[tokio::test]
    async fn tool_reports_how_it_decided() {
        let tool = MathAnswerCheckTool;
        let ctx = ToolContext {
            tenant_id: "t".into(),
            user_id: "u".into(),
            session_id: uuid::Uuid::nil(),
            credentials: Default::default(),
            timeout: std::time::Duration::from_secs(1),
        };

        let out = tool
            .execute(
                serde_json::json!({
                    "prediction": r"Thinking...\boxed{\frac{1}{2}}",
                    "ground_truth": "0.5"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert_eq!(out.content["matched"], true);
        assert_eq!(out.content["method"], "numeric");
        assert_eq!(out.content["extracted"], r"\frac{1}{2}");

        let out = tool
            .execute(
                serde_json::json!({"prediction": r"\boxed{7}", "ground_truth": "7"}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(out.content["method"], "exact");

        let out = tool
            .execute(
                serde_json::json!({"prediction": r"\boxed{(1,2)}", "ground_truth": "5"}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(out.content["matched"], false);
        assert_eq!(out.content["method"], "part_count_mismatch");

        // Missing field is a validation error, not a panic.
        assert!(tool
            .validate_input(&serde_json::json!({"prediction": "x"}))
            .is_err());
    }
}
