//! IEEE 1800-2017 §9.4.5 intra-assignment delay canonicalization.
//!
//! The parser consumes and DISCARDS intra-assignment timing controls
//! (`skip_intra_assignment_timing` in sv-parser's statements.rs), so a
//! `lhs = #d rhs;` delay can never reach the simulator through the AST.
//! Until the parser models it, this pre-parse pass rewrites the source text
//!
//! ```text
//! lhs  = #d rhs;   ->   lhs  = $__xz_intra_delay(d, rhs);
//! lhs <= #d rhs;   ->   lhs <= $__xz_intra_delay(d, rhs);
//! ```
//!
//! and the simulator implements §9.4.5 for the marker call: evaluate the RHS
//! immediately, suspend the process `d` time units, then assign the saved
//! value. The rewrite preserves every byte of whitespace (line numbers do not
//! shift). Intra-assignment EVENT controls (`= @(...)`, `= repeat(n) @(...)`)
//! keep the parser's existing discard behavior, as do min:typ:max delays.
//! Files pulled in via `include are preprocessed inside xezim-core and are
//! not seen by this pass.

/// Marker system-function name the simulator recognizes (never user-visible).
pub const INTRA_DELAY_MARKER: &str = "$__xz_intra_delay";

/// Skip whitespace and comments starting at `i`; returns the next index of
/// significant text.
fn skip_ws_comments(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else {
            return i;
        }
    }
}

/// Index just past a `"..."` string literal starting at `i` (which must be
/// the opening quote).
fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// Find the `;` terminating the statement, starting at `i`, at zero
/// paren/bracket/brace depth, skipping strings and comments.
fn find_stmt_semi(b: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 0i32;
    while i < b.len() {
        match b[i] {
            b'/' if i + 1 < b.len() && (b[i + 1] == b'/' || b[i + 1] == b'*') => {
                i = skip_ws_comments(b, i);
            }
            b'"' => i = skip_string(b, i),
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
                i += 1;
            }
            b';' if depth == 0 => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// End of the `\`define` line starting at (or before) `i`: the first newline
/// not escaped by a trailing `\` continuation.
fn define_line_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() {
        if b[i] == b'\n' && (i == 0 || b[i - 1] != b'\\') {
            return i;
        }
        i += 1;
    }
    b.len()
}

/// Given `i` at the `#` of an intra-assignment delay, return
/// `(delay_end, semi)` where `src[i+1..delay_end]` is the delay expression
/// text (paren-wrapped or a single literal/identifier token) and `semi`
/// indexes the statement's terminating `;`. `None` means "leave unchanged".
fn extract_delay_and_rhs(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let j = skip_ws_comments(b, i + 1);
    let delay_end;
    if j < b.len() && b[j] == b'(' {
        let mut k = j + 1;
        let mut depth = 1i32;
        let mut top_colon = false;
        while k < b.len() && depth > 0 {
            match b[k] {
                b'"' => {
                    k = skip_string(b, k);
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                // min:typ:max delay `#(1:2:3)` — not an expression; keep the
                // parser's discard behavior.
                b':' if depth == 1 => top_colon = true,
                _ => {}
            }
            k += 1;
        }
        if depth != 0 || top_colon {
            return None;
        }
        delay_end = k;
    } else {
        // `#5`, `#1.5ns`, `#delay_id` — one literal/identifier token.
        let mut k = j;
        while k < b.len()
            && (b[k].is_ascii_alphanumeric() || matches!(b[k], b'_' | b'$' | b'.'))
        {
            k += 1;
        }
        if k == j {
            return None;
        }
        delay_end = k;
    }
    let semi = find_stmt_semi(b, delay_end)?;
    // Empty RHS (`x = #1;` is not an intra-assignment) — leave unchanged.
    if b[delay_end..semi].iter().all(|c| c.is_ascii_whitespace()) {
        return None;
    }
    Some((delay_end, semi))
}

/// Rewrite `= #d rhs;` / `<= #d rhs;` into the `$__xz_intra_delay(d, rhs)`
/// marker form (see module docs). Returns the input unchanged when no
/// intra-assignment delay is present.
pub fn rewrite_intra_assignment_delays(src: &str) -> String {
    if !src.contains('#') {
        return src.to_string();
    }
    let b = src.as_bytes();
    let n = b.len();
    let mut out = String::with_capacity(n + 64);
    let mut i = 0usize;
    // Last three significant chars (outside comments/strings), newest first.
    let (mut p1, mut p2, mut p3) = (b' ', b' ', b' ');
    let push_sig = |p1: &mut u8, p2: &mut u8, p3: &mut u8, c: u8| {
        *p3 = *p2;
        *p2 = *p1;
        *p1 = c;
    };
    // While inside a `define body, a rewrite must find its `;` before the
    // (continuation-aware) end of the define line — the use site may supply
    // the semicolon, and scanning past the define would corrupt the source.
    let mut define_end: Option<usize> = None;
    while i < n {
        if define_end.is_some_and(|e| i >= e) {
            define_end = None;
        }
        let c = b[i];
        if c >= 0x80 {
            // Copy a multi-byte UTF-8 char whole (byte-wise push would
            // re-encode it as Latin-1 and corrupt it).
            let len = match c {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                _ => 4,
            }
            .min(n - i);
            out.push_str(&src[i..i + len]);
            i += len;
            push_sig(&mut p1, &mut p2, &mut p3, c);
            continue;
        }
        if c == b'/' && i + 1 < n && (b[i + 1] == b'/' || b[i + 1] == b'*') {
            let end = skip_ws_comments(b, i);
            out.push_str(&src[i..end]);
            i = end;
            continue;
        }
        if c == b'"' {
            let end = skip_string(b, i);
            out.push_str(&src[i..end]);
            i = end;
            push_sig(&mut p1, &mut p2, &mut p3, b'"');
            continue;
        }
        if c == b'`' && src[i..].starts_with("`define") {
            define_end = Some(define_line_end(b, i));
        }
        if c == b'#' && (i + 1 >= n || b[i + 1] != b'#') {
            // Trigger only directly after a plain `=` (blocking) or `<=`
            // (nonblocking) assignment operator — the only legal SV contexts
            // where `#` follows `=`. Compound (`+=`) and comparison
            // (`==`, `<=` as relational can't precede `#`) forms excluded.
            let blocking = p1 == b'='
                && !matches!(
                    p2,
                    b'=' | b'!'
                        | b'<'
                        | b'>'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'&'
                        | b'|'
                        | b'^'
                        | b'~'
                );
            let nba = p1 == b'=' && p2 == b'<' && p3 != b'<';
            if blocking || nba {
                if let Some((delay_end, semi)) = extract_delay_and_rhs(b, i) {
                    if define_end.map_or(true, |e| semi < e) {
                        out.push_str(INTRA_DELAY_MARKER);
                        out.push('(');
                        out.push_str(&src[i + 1..delay_end]);
                        out.push(',');
                        out.push_str(&src[delay_end..semi]);
                        out.push(')');
                        i = semi; // main loop copies the `;`
                        continue;
                    }
                }
            }
        }
        out.push(c as char);
        if !c.is_ascii_whitespace() {
            push_sig(&mut p1, &mut p2, &mut p3, c);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_blocking_and_nba() {
        let s = "v = #2 5;\nx <= #(D*2) w + 1;\n";
        let r = rewrite_intra_assignment_delays(s);
        assert_eq!(
            r,
            "v = $__xz_intra_delay(2, 5);\nx <= $__xz_intra_delay((D*2), w + 1);\n"
        );
    }

    #[test]
    fn leaves_non_intra_untouched() {
        for s in [
            "if (a <= 3) b = 1;\n",
            "#5 v = 1;\n",
            "a = b ## 2;\n",
            "x = #(1:2:3) y;\n", // min:typ:max — parser keeps discarding
            "s = \"= #2 5;\";\n",
            "// v = #2 5;\n",
        ] {
            assert_eq!(rewrite_intra_assignment_delays(s), s);
        }
    }
}
