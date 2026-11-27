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
//! shift). A min:typ:max delay `#(1:2:3)` is copied as written; the parser
//! reads the parenthesized triple as its typical value.
//! Files pulled in via `include are preprocessed inside xezim-core and are
//! not seen by this pass.

/// Marker system-function name the simulator recognizes (never user-visible).
pub const INTRA_DELAY_MARKER: &str = "$__xz_intra_delay";

/// Marker for §9.4.5 intra-assignment EVENT controls:
/// `lhs = repeat(n) @(posedge clk) rhs;` / `lhs = @(ev) rhs;` become
/// `$__xz_intra_ev(n, <edge>, <sig>, rhs)` where `<edge>` is 0 (any),
/// 1 (posedge) or 2 (negedge). Multi-term event lists keep the parser's
/// discard behavior.
pub const INTRA_EVENT_MARKER: &str = "$__xz_intra_ev";

/// Marker for §14.16 intra-assignment CYCLE delay on a clocking-block drive:
/// `cb.out <= ##N rhs;` becomes `$__xz_intra_cycle(N, rhs)`. The simulator
/// defers the drive by N clocking events of that block.
pub const INTRA_CYCLE_MARKER: &str = "$__xz_intra_cycle";

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

/// IEEE 1800-2023 §5.7.2 and §9.4.5: a bare delay can be a real literal.
/// Extend the scan across an adjacent exponent sign only after a whole
/// numeric mantissa and exponent letter, so a sign on the RHS stays there.
fn extend_signed_real_delay(b: &[u8], start: usize, end: usize) -> usize {
    if !matches!(b.get(end), Some(b'+' | b'-'))
        || !b.get(end + 1).is_some_and(u8::is_ascii_digit)
    {
        return end;
    }
    let [first, mantissa @ .., b'e' | b'E'] = &b[start..end] else {
        return end;
    };
    if !first.is_ascii_digit() {
        return end;
    }

    let is_decimal = |c: &u8| c.is_ascii_digit() || *c == b'_';
    let integer_len = mantissa.iter().take_while(|c| is_decimal(c)).count();
    match &mantissa[integer_len..] {
        [] => {}
        [b'.', digit, rest @ ..] if digit.is_ascii_digit() && rest.iter().all(is_decimal) => {}
        _ => return end,
    }
    // IEEE 1800-2023 §5.8 and A.8.7: an adjacent unit name starts the RHS.
    let mut k = end + 2;
    while b.get(k).is_some_and(is_decimal) {
        k += 1;
    }
    k
}

/// Given `i` at the first `#` of an intra-assignment delay, return
/// `(delay_end, semi)`. The delay expression spans from the end of `#` or
/// `##` to `delay_end`, and `semi` indexes the terminating `;`.
/// `None` means "leave unchanged".
fn extract_delay_and_rhs(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let cycle_delay = b.get(i + 1) == Some(&b'#');
    let j = skip_ws_comments(b, i + 1 + usize::from(cycle_delay));
    let delay_end;
    if j < b.len() && b[j] == b'(' {
        let mut k = j + 1;
        let mut depth = 1i32;
        while k < b.len() && depth > 0 {
            match b[k] {
                b'"' => {
                    k = skip_string(b, k);
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            k += 1;
        }
        if depth != 0 {
            return None;
        }
        delay_end = k;
    } else {
        // `#5`, `#1.5ns`, `#delay_id` — one literal/identifier token.
        let mut k = j;
        while k < b.len() && (b[k].is_ascii_alphanumeric() || matches!(b[k], b'_' | b'$' | b'.')) {
            k += 1;
        }
        if k == j {
            return None;
        }
        delay_end = if cycle_delay {
            k
        } else {
            extend_signed_real_delay(b, j, k)
        };
    }
    let semi = find_stmt_semi(b, delay_end)?;
    // Empty RHS (`x = #1;` is not an intra-assignment) — leave unchanged.
    if b[delay_end..semi].iter().all(|c| c.is_ascii_whitespace()) {
        return None;
    }
    Some((delay_end, semi))
}

/// Given `i` at the `@` of an intra-assignment event control, return
/// `(edge, sig_text, rhs_start, semi)`. `None` -> leave unchanged
/// (multi-term lists, empty RHS).
fn extract_event_and_rhs(b: &[u8], src: &str, i: usize) -> Option<(u8, String, usize, usize)> {
    let j = skip_ws_comments(b, i + 1);
    if j >= b.len() {
        return None;
    }
    // §9.4.2 `@ hierarchical_event_identifier` without parentheses.
    if b[j] != b'(' {
        if !(b[j].is_ascii_alphabetic() || b[j] == b'_') {
            return None;
        }
        let mut k = j;
        while k < b.len() && (b[k].is_ascii_alphanumeric() || matches!(b[k], b'_' | b'$' | b'.')) {
            k += 1;
        }
        let semi = find_stmt_semi(b, k)?;
        if b[k..semi].iter().all(|c| c.is_ascii_whitespace()) {
            return None;
        }
        return Some((0, src[j..k].to_string(), k, semi));
    }
    let mut k = j + 1;
    let mut depth = 1i32;
    while k < b.len() && depth > 0 {
        match b[k] {
            b'"' => {
                k = skip_string(b, k);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        k += 1;
    }
    if depth != 0 {
        return None;
    }
    let ev = src[j + 1..k - 1].trim();
    // Multi-term event lists: keep discard behavior.
    if ev.contains(" or ") || ev.contains(',') {
        return None;
    }
    let (edge, sig) = if let Some(r) = ev.strip_prefix("posedge ") {
        (1u8, r.trim())
    } else if let Some(r) = ev.strip_prefix("negedge ") {
        (2u8, r.trim())
    } else {
        (0u8, ev)
    };
    if sig.is_empty()
        || !sig
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | b'.' | b'[' | b']'))
    {
        return None;
    }
    let semi = find_stmt_semi(b, k)?;
    if b[k..semi].iter().all(|c| c.is_ascii_whitespace()) {
        return None;
    }
    Some((edge, sig.to_string(), k, semi))
}

/// Rewrite `= #d rhs;` / `<= #d rhs;` into the `$__xz_intra_delay(d, rhs)`
/// marker form (see module docs). Returns `Cow::Borrowed(src)` with zero
/// allocations when no intra-assignment delay is present (#721).
pub fn rewrite_intra_assignment_delays_cow(src: &str) -> std::borrow::Cow<'_, str> {
    if !src.contains('#') && !src.contains('@') {
        return std::borrow::Cow::Borrowed(src);
    }
    let b = src.as_bytes();
    let n = b.len();
    let mut out: Option<String> = None;
    let mut copied = 0usize;
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
            let len = match c {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                _ => 4,
            }
            .min(n - i);
            i += len;
            push_sig(&mut p1, &mut p2, &mut p3, c);
            continue;
        }
        if c == b'/' && i + 1 < n && (b[i + 1] == b'/' || b[i + 1] == b'*') {
            i = skip_ws_comments(b, i);
            continue;
        }
        if c == b'"' {
            i = skip_string(b, i);
            push_sig(&mut p1, &mut p2, &mut p3, b'"');
            continue;
        }
        if c == b'`' && src[i..].starts_with("`define") {
            define_end = Some(define_line_end(b, i));
        }
        // `= @(ev) rhs;` / `= repeat(n) @(ev) rhs;` intra-assignment event
        // controls (§9.4.5). Same assignment-operator gating as `#` below.
        if c == b'@' || (c == b'r' && src[i..].starts_with("repeat")) {
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
                let mut count_txt = String::from("1");
                let mut at_pos = i;
                let mut ok = true;
                if c == b'r' {
                    // repeat ( count ) @
                    let mut k = skip_ws_comments(b, i + 6);
                    if k < n && b[k] == b'(' {
                        let mut depth = 1i32;
                        let start = k + 1;
                        k += 1;
                        while k < n && depth > 0 {
                            match b[k] {
                                b'(' => depth += 1,
                                b')' => depth -= 1,
                                _ => {}
                            }
                            k += 1;
                        }
                        if depth == 0 {
                            count_txt = src[start..k - 1].trim().to_string();
                            let a = skip_ws_comments(b, k);
                            if a < n && b[a] == b'@' {
                                at_pos = a;
                            } else {
                                ok = false;
                            }
                        } else {
                            ok = false;
                        }
                    } else {
                        ok = false;
                    }
                }
                if ok {
                    if let Some((edge, sig, rhs_start, semi)) =
                        extract_event_and_rhs(b, src, at_pos)
                    {
                        if define_end.is_none_or(|e| semi < e) {
                            let buf = out.get_or_insert_with(|| String::with_capacity(n + 64));
                            buf.push_str(&src[copied..i]);
                            buf.push_str(INTRA_EVENT_MARKER);
                            buf.push('(');
                            buf.push_str(&count_txt);
                            buf.push(',');
                            buf.push_str(&edge.to_string());
                            buf.push(',');
                            buf.push_str(&sig);
                            buf.push(',');
                            buf.push_str(&src[rhs_start..semi]);
                            buf.push(')');
                            copied = semi;
                            i = semi; // main loop copies the `;`
                            continue;
                        }
                    }
                }
            }
        }
        if c == b'#' && i + 1 < n && b[i + 1] == b'#' {
            // §14.16 cycle-delay intra-assignment `lhs <= ##N rhs;` — same
            // trigger rule as the `#delay` form below (directly after `=`/`<=`).
            let blocking_c = p1 == b'='
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
            let nba_c = p1 == b'=' && p2 == b'<' && p3 != b'<';
            if blocking_c || nba_c {
                if let Some((cnt_end, semi)) = extract_delay_and_rhs(b, i) {
                    if define_end.is_none_or(|e| semi < e) {
                        let buf = out.get_or_insert_with(|| String::with_capacity(n + 64));
                        buf.push_str(&src[copied..i]);
                        buf.push_str(INTRA_CYCLE_MARKER);
                        buf.push('(');
                        buf.push_str(&src[i + 2..cnt_end]);
                        buf.push(',');
                        buf.push_str(&src[cnt_end..semi]);
                        buf.push(')');
                        copied = semi;
                        i = semi;
                        continue;
                    }
                }
            }
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
                    if define_end.is_none_or(|e| semi < e) {
                        let buf = out.get_or_insert_with(|| String::with_capacity(n + 64));
                        buf.push_str(&src[copied..i]);
                        buf.push_str(INTRA_DELAY_MARKER);
                        buf.push('(');
                        buf.push_str(&src[i + 1..delay_end]);
                        buf.push(',');
                        buf.push_str(&src[delay_end..semi]);
                        buf.push(')');
                        copied = semi;
                        i = semi; // main loop copies the `;`
                        continue;
                    }
                }
            }
        }
        if !c.is_ascii_whitespace() {
            push_sig(&mut p1, &mut p2, &mut p3, c);
        }
        i += 1;
    }
    match out {
        Some(mut buf) => {
            buf.push_str(&src[copied..]);
            std::borrow::Cow::Owned(buf)
        }
        None => std::borrow::Cow::Borrowed(src),
    }
}

/// Rewrite `= #d rhs;` / `<= #d rhs;` into the `$__xz_intra_delay(d, rhs)`
/// marker form (see module docs). Returns the input unchanged when no
/// intra-assignment delay is present.
pub fn rewrite_intra_assignment_delays(src: &str) -> String {
    rewrite_intra_assignment_delays_cow(src).into_owned()
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
    fn rewrites_signed_real_delays() {
        let src = "x = #1.5e-1 7;\nx <= #1_0.5_E+0_1 8;\nx = #1_e-1 9;\n";
        assert_eq!(
            rewrite_intra_assignment_delays(src),
            concat!(
                "x = $__xz_intra_delay(1.5e-1, 7);\n",
                "x <= $__xz_intra_delay(1_0.5_E+0_1, 8);\n",
                "x = $__xz_intra_delay(1_e-1, 9);\n",
            )
        );
    }

    #[test]
    fn signed_real_delay_token_boundaries() {
        for (delay, rhs) in [
            ("#1.5e-1", "ns + 7;"),
            ("#1.5e-1", "ns;"),
            ("#1.5e-1", "s;"),
            ("#1.5e-1", "s * 2;"),
            ("#1.5e-1", "s[0];"),
            ("#1.5e-1", "s.a;"),
            ("#1.5e-1_", "s + 3;"),
            ("#1e-1", "ns + 7;"),
            ("#1.5e-1", "s_var;"),
            ("#1.5e-1", "s$var;"),
            ("#1.5e-1", "nsvar;"),
            ("#e", "-1;"),
            ("#E", "-1;"),
            ("#d_e", "-1;"),
            ("#e", "+1;"),
            ("#1", "-1;"),
            ("#1", " -v;"),
            ("##1", " - 1;"),
            // Malformed exponent and cycle-count fragments retain their old split.
            ("#1e", " - 1;"),
            ("#1e", "-v;"),
            ("#1e", "-_1 7;"),
            ("#1._5e", "-1 7;"),
            ("#1.e", "-1 7;"),
            ("#1.2.3e", "-1 7;"),
            ("##1e", "-1 7;"),
        ] {
            let input = format!("{delay}{rhs}");
            let (end, semi) = extract_delay_and_rhs(input.as_bytes(), 0).unwrap();
            assert_eq!((end, semi), (delay.len(), input.len() - 1), "{input}");
        }
    }

    #[test]
    fn rewrites_min_typ_max_delay() {
        let r = rewrite_intra_assignment_delays("v = # (2:10:17) 4'h5;\n");
        assert_eq!(r, "v = $__xz_intra_delay( (2:10:17), 4'h5);\n");
    }

    #[test]
    fn rewrites_bare_event_identifier() {
        let s = "v = @ ev 4'h5;\nw = repeat (5) @top.ev 1'b1 && 1'b1;\n";
        let r = rewrite_intra_assignment_delays(s);
        assert_eq!(
            r,
            "v = $__xz_intra_ev(1,0,ev, 4'h5);\nw = $__xz_intra_ev(5,0,top.ev, 1'b1 && 1'b1);\n"
        );
    }

    #[test]
    fn leaves_non_intra_untouched() {
        for s in [
            "if (a <= 3) b = 1;\n",
            "#5 v = 1;\n",
            "a = b ## 2;\n",
            "s = \"= #2 5;\";\n",
            "// v = #2 5;\n",
        ] {
            assert_eq!(rewrite_intra_assignment_delays(s), s);
        }
    }
}
