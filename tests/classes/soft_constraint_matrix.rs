//! §18.5.14: a soft constraint holds unless it conflicts with a hard one (or
//! a higher-priority soft one), whatever its shape. Each case below is run
//! twice: as written, which the joint solver (`rand_csp`) takes, and with an
//! unrelated `b == a / 8` added, which that solver does not model, so the
//! trial loop's repair passes solve it. The trial loop used to:
//!   * force a soft item nested in a `foreach` like a hard one, so it and a
//!     conflicting hard item (`q[0] != -1`) undid each other every pass and
//!     every trial was rejected (randomize() returned 0);
//!   * skip every soft item on a variable some hard item names
//!     (`x >= -1; soft x == -1;` lost the soft value although it holds);
//!   * re-pick an element for `!=` over the whole word, ignoring its
//!     `inside` domain, and repair a signed element's `>=` / `<` over an
//!     unsigned domain (`q[i] >= -1` rejected a legal -1);
//!   * never fall back to a lower-priority soft item when the winning one
//!     conflicts with a hard item.

use xezim::simulate;

/// (name, class declarations, randomize call, check). `C`, `S` and `B` are
/// renamed per case; `DIV` marks where the solver-defeating item goes.
const CASES: &[(&str, &str, &str, &str)] = &[
    (
        "scalar_compat",
        "class C; rand int x; constraint h { x inside {[-1:15]}; } constraint s { soft x == -1; } DIV endclass",
        "o.randomize()",
        "o.x == -1",
    ),
    (
        "scalar_conflict",
        "class C; rand int x; constraint h { x inside {[-1:15]}; x != -1; } constraint s { soft x == -1; } DIV endclass",
        "o.randomize()",
        "o.x inside {[0:15]}",
    ),
    (
        "elem_compat",
        "class C; rand int q[2]; constraint h { q[0] inside {[-1:15]}; } constraint s { soft q[0] == -1; } DIV endclass",
        "o.randomize()",
        "o.q[0] == -1",
    ),
    (
        "elem_conflict",
        "class C; rand int q[2]; constraint h { q[0] inside {[-1:15]}; q[0] != -1; } constraint s { soft q[0] == -1; } DIV endclass",
        "o.randomize()",
        "o.q[0] inside {[0:15]}",
    ),
    (
        "foreach_compat",
        "class C; rand int q[2]; constraint h { foreach (q[i]) q[i] inside {[-1:15]}; } constraint s { foreach (q[i]) soft q[i] == -1; } DIV endclass",
        "o.randomize()",
        "o.q[0] == -1 && o.q[1] == -1",
    ),
    (
        "foreach_conflict",
        "class C; rand int q[2]; constraint h { foreach (q[i]) { q[i] inside {[-1:15]}; q[i] != -1; } } constraint s { foreach (q[i]) soft q[i] == -1; } DIV endclass",
        "o.randomize()",
        "o.q[0] inside {[0:15]} && o.q[1] inside {[0:15]}",
    ),
    (
        "if_soft_compat",
        "class C; bit en = 1; rand int x; constraint h { x inside {[-1:15]}; } constraint s { if (en) soft x == -1; } DIV endclass",
        "o.randomize()",
        "o.x == -1",
    ),
    (
        "if_soft_conflict",
        "class C; bit en = 1; rand int x; constraint h { x inside {[-1:15]}; x != -1; } constraint s { if (en) soft x == -1; } DIV endclass",
        "o.randomize()",
        "o.x inside {[0:15]}",
    ),
    (
        "if_foreach_compat",
        "class C; bit en = 1; rand int q[2]; constraint h { if (en) { foreach (q[i]) q[i] inside {[-1:15]}; } } constraint s { foreach (q[i]) soft q[i] == -1; } DIV endclass",
        "o.randomize()",
        "o.q[0] == -1 && o.q[1] == -1",
    ),
    (
        "if_foreach_conflict",
        "class C; bit en = 1; rand int q[2]; constraint h { if (en) { foreach (q[i]) { q[i] inside {[-1:15]}; q[i] != -1; } } } constraint s { foreach (q[i]) soft q[i] == -1; } DIV endclass",
        "o.randomize()",
        "o.q[0] inside {[0:15]} && o.q[1] inside {[0:15]}",
    ),
    (
        "inline_scalar",
        "class C; rand int x; constraint h { x inside {[-1:15]}; } constraint s { soft x == -1; } DIV endclass",
        "o.randomize() with { x != -1; }",
        "o.x inside {[0:15]}",
    ),
    (
        "inline_foreach",
        "class C; rand int q[2]; constraint h { foreach (q[i]) q[i] inside {[-1:15]}; } constraint s { foreach (q[i]) soft q[i] == -1; } DIV endclass",
        "o.randomize() with { q[0] != -1; }",
        "o.q[0] inside {[0:15]} && o.q[1] == -1",
    ),
    (
        "sub_compat",
        "class S; rand int x; constraint h { x inside {[-1:15]}; } constraint s { soft x == -1; } DIV endclass class C; rand S c; function new(); c = new(); endfunction endclass",
        "o.randomize()",
        "o.c.x == -1",
    ),
    (
        "sub_conflict",
        "class S; rand int x; constraint h { x inside {[-1:15]}; } constraint s { soft x == -1; } DIV endclass class C; rand S c; constraint k { c.x != -1; } function new(); c = new(); endfunction endclass",
        "o.randomize()",
        "o.c.x inside {[0:15]}",
    ),
    (
        "sub_array_conflict",
        "class S; rand int q[2]; constraint h { foreach (q[i]) q[i] inside {[-1:15]}; } constraint s { foreach (q[i]) soft q[i] == -1; } DIV endclass class C; rand S c[2]; constraint k { foreach (c[i]) c[i].q[0] != -1; } function new(); c[0] = new(); c[1] = new(); endfunction endclass",
        "o.randomize()",
        "o.c[0].q[0] inside {[0:15]} && o.c[1].q[0] inside {[0:15]} && o.c[0].q[1] == -1",
    ),
    (
        "priority_compat",
        "class C; rand int x; constraint h { x inside {[0:15]}; } constraint s { soft x == 3; soft x == 5; } DIV endclass",
        "o.randomize()",
        "o.x == 5",
    ),
    (
        "priority_conflict",
        "class C; rand int x; constraint h { x inside {[0:4]}; } constraint s { soft x == 3; soft x == 5; } DIV endclass",
        "o.randomize()",
        "o.x == 3",
    ),
    (
        "derived",
        "class B; rand int x; constraint h { x inside {[0:15]}; } constraint sb { soft x == 3; } DIV endclass class C extends B; constraint sd { soft x == 5; } endclass",
        "o.randomize()",
        "o.x == 5",
    ),
    (
        "rel_compat",
        "class C; rand int x; constraint h { x >= -1; } constraint s { soft x == -1; } DIV endclass",
        "o.randomize()",
        "o.x == -1",
    ),
    (
        "rel_conflict",
        "class C; rand int x; constraint h { x >= 0; x < 100; } constraint s { soft x == -1; } DIV endclass",
        "o.randomize()",
        "o.x >= 0 && o.x < 100",
    ),
    (
        "eq_conflict",
        "class C; rand int x; constraint h { x == 5; } constraint s { soft x == 3; } DIV endclass",
        "o.randomize()",
        "o.x == 5",
    ),
    (
        "elem_rel_compat",
        "class C; rand int q[2]; constraint h { foreach (q[i]) q[i] >= -1; } constraint s { foreach (q[i]) soft q[i] == -1; } DIV endclass",
        "o.randomize()",
        "o.q[0] == -1 && o.q[1] == -1",
    ),
];

/// Rename the case's classes to `<name><index>` (the class table is keyed
/// by bare name, so each case needs its own).
fn rename_classes(src: &str, i: usize) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |w: &mut String, out: &mut String| {
        if matches!(w.as_str(), "C" | "S" | "B") {
            out.push_str(&format!("{w}{i}"));
        } else {
            out.push_str(w);
        }
        w.clear();
    };
    for ch in src.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            word.push(ch);
        } else {
            flush(&mut word, &mut out);
            out.push(ch);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn run(trial_loop: bool) -> Vec<String> {
    let div = if trial_loop {
        "rand int dv_a, dv_b; constraint k_div { dv_b == dv_a / 8; }"
    } else {
        ""
    };
    let mut src = String::new();
    for (i, (_, cls, _, _)) in CASES.iter().enumerate() {
        src.push_str(&rename_classes(&cls.replace("DIV", div), i));
        src.push('\n');
    }
    src.push_str("module top;\n  initial begin\n");
    for (i, (name, _, call, chk)) in CASES.iter().enumerate() {
        src.push_str(&format!(
            "    begin
      C{i} o = new(); int ok, bad = 0;
      for (int n = 0; n < 5; n++) begin ok = {call}; if (!ok || !({chk})) bad++; end
      $display(\"{name} bad=%0d\", bad);
    end
"
        ));
    }
    src.push_str("  end\nendmodule\n");
    simulate(&src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn expected() -> Vec<String> {
    CASES
        .iter()
        .map(|(name, ..)| format!("{name} bad=0"))
        .collect()
}

#[test]
fn soft_constraints_joint_solver() {
    assert_eq!(run(false), expected());
}

#[test]
fn soft_constraints_trial_loop() {
    assert_eq!(run(true), expected());
}
