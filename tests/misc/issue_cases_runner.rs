//! The reporters' own test cases from GitHub issues #17/#22/#23/#24/#25,
//! run verbatim as regressions (they were previously verified by hand only).
//! Plus two formerly-ORPHANED .sv testcases under tests/ that no cargo suite
//! ever executed: fork_wait_deadlock.sv (passing) and
//! dpi/force_release_compliance.sv (ratcheted — see its test).

use xezim::simulate;

fn outputs(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn issue_17_dynamic_array_of_mailboxes() {
    let msgs = outputs(include_str!("../issue_cases/dyn.arr.of.mbox.sv"), 100_000);
    let data_lines = msgs.iter().filter(|m| m.starts_with("Data from")).count();
    assert_eq!(data_lines, 80, "all 5x16 mailbox entries must round-trip");
    assert!(
        !msgs.iter().any(|m| m.contains("[x]")),
        "no unbound foreach index may appear"
    );
}

#[test]
fn issue_17_mailbox_in_interface() {
    let msgs = outputs(
        include_str!("../issue_cases/mbox_in_interface.sv"),
        2_000_000,
    );
    let received = msgs.iter().filter(|m| m.contains("Received")).count();
    // 1000 post-reset cycles: sender0 every 3, sender1 every 5.
    assert_eq!(received, 533, "1000/3 + 1000/5 sender puts must arrive");
}

#[test]
fn issue_22_final_blocks() {
    let msgs = outputs(
        include_str!("../issue_cases/final.blocks.test.case.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
    let finals = msgs.iter().filter(|m| m.contains("inal block")).count();
    assert!(finals >= 4, "all four final blocks must run: {:?}", msgs);
}

#[test]
fn issue_23_string_methods() {
    let msgs = outputs(
        include_str!("../issue_cases/string.compliance.tests.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_24_swrite_sformat() {
    let msgs = outputs(
        include_str!("../issue_cases/data.to.string.fmt.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_25_format_specifiers() {
    let msgs = outputs(include_str!("../issue_cases/fmt.specifiers.sv"), 100_000);
    let pass = msgs.iter().filter(|m| m.starts_with("[PASS]")).count();
    let fail = msgs.iter().filter(|m| m.starts_with("[ERROR]")).count();
    assert_eq!(pass, 42, "passing-check count changed: {:?}", msgs);
    assert_eq!(fail, 0, "all format specifier checks should pass");
}

#[test]
fn orphan_fork_wait_deadlock() {
    let msgs = outputs(include_str!("../fork_wait_deadlock.sv"), 100_000);
    assert!(
        msgs.iter()
            .any(|m| m.contains("PASS: fork-local variable sharing works")),
        "{:?}",
        msgs
    );
}

#[test]
fn fork_child_blocking_live_share() {
    // The UVM sequencer watchdog (`m_safe_select_item`) writes a task-local
    // automatic from a `fork … join_none` child that then blocks forever on
    // `await()`, while the parent parks on `wait(select_process != null)`. A
    // merge-on-child-completion model can never deliver the value (the child
    // never completes) — the write must be LIVE-shared with the suspended
    // parent. This regression compiles that exact child-blocks-forever idiom.
    let msgs = outputs(include_str!("../fork_child_blocking_share.sv"), 100_000);
    assert!(
        msgs.iter()
            .any(|m| m.contains("PASS: fork-child live-shared write wakes a parked wait")),
        "{:?}",
        msgs
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:?}", msgs);
}

#[test]
fn fork_child_merge_wake() {
    // A fork...join_none child's write to the parent's task-local automatic
    // reaches the suspended parent through the child->parent frame merge, and
    // must wake the parked level-sensitive `wait` (§9.4.2) — inside a clock
    // edge's cascade, before that edge's NBA region (reference: seen=0,1).
    let msgs = outputs(include_str!("../fork_child_merge_wake.sv"), 100_000);
    assert!(
        msgs.iter().any(|m| m.contains("PASS result=1 seen=0,1")),
        "{:?}",
        msgs
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:?}", msgs);
}

#[test]
fn orphan_force_release_compliance_ratchet() {
    let msgs = outputs(include_str!("../dpi/force_release_compliance.sv"), 100_000);
    let fails = msgs.iter().filter(|m| m.starts_with("FAIL")).count();
    assert_eq!(
        fails,
        0,
        "force/release known-gap count changed — new regression or a fixed \
         gap (lower the count): {:?}",
        msgs.iter()
            .filter(|m| m.starts_with("FAIL"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn issue_21_timescale_handling() {
    // §3.14.3 precision quantization + per-module directive scales.
    let msgs = outputs(
        include_str!("../issue_cases/timescale.handling.sv"),
        1_000_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_18_type_parameters() {
    // §6.20.3 type params: structs, arrays, class handles.
    let msgs = outputs(
        include_str!("../issue_cases/type-parameter-compliance.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_28_constraint_foreach() {
    // §18.5.7 foreach constraint bodies beyond `inside`.
    let msgs = outputs(
        include_str!("../issue_cases/constraint.foreach.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

#[test]
fn issue_29_constraint_typecast() {
    // §18.3/§6.24.1/§11.6.1 casts inside constraint expressions.
    let msgs = outputs(
        include_str!("../issue_cases/constraint.typecast.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

#[test]
fn issue_26_static_init_sysfuncs() {
    // §6.21/§10.5 sim-time syscall inits + §20.6 type operands
    // ($size(logic [7:0])). The test reads plusargs, so it needs the
    // args-aware entry point.
    let src = include_str!("../issue_cases/static.init.sysfuncs.sv").to_string();
    let plusargs = vec!["TEST_MODE".to_string(), "SEED_VAL=42".to_string()];
    let sim = xezim::simulate_multi(
        &[src],
        100_000,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &plusargs,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

/// Issue #233: the reporter's parenless-call matrix (enum methods on locals,
/// foreach keys and class members; free, package and interface functions;
/// string and queue built-ins), run verbatim.
#[test]
fn issue_233_parenless_calls() {
    let msgs = outputs(include_str!("../issue_cases/parenless.calls.sv"), 100_000);
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #246: the reporter's randomize()-over-struct-fields MWE, run
/// verbatim — member dist/inside/relational/equality targets, part-selects,
/// nested members, unpacked members, `unique` on a 2-D array, the handle vs
/// method storage views, and the nested rand-handle solve that used to spin
/// for minutes (it must finish well inside the run).
#[test]
fn issue_246_struct_rand_fields() {
    let start = std::time::Instant::now();
    let msgs = outputs(
        include_str!("../issue_cases/struct.rand.fields.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(60),
        "took {:?}",
        start.elapsed()
    );
}

/// The randomize() width-overflow family and dist obedience MWE, run
/// verbatim: `==` / `!=` forcing a value wider than its rand variable, a
/// >64-bit rand struct with member-sum constraints, and the dist-frequency
/// and width-boundary guards around them. `tests/classes/rand_width_overflow.rs`
/// restates each section as its own test.
#[test]
fn rand_width_overflow_and_dist_mwe() {
    let msgs = outputs(include_str!("../issue_cases/rand.width.dist.sv"), 100_000);
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #262: $fscanf %c must match exactly one character (§21.3.4.3) —
/// no whitespace skipping for %c or literal-only formats, newlines kept
/// across repeated %c, all 256 byte values round-tripping through
/// $fwrite/$fscanf %c, task-form $fscanf converting, and C-stdio pushback
/// semantics ($ftell subtracts, $fread drains, $fseek/$rewind discard).
#[test]
fn issue_262_fscanf_percent_c() {
    let msgs = outputs(include_str!("../issue_cases/fscanf_262.sv"), 100_000);
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #312: `foreach` over an associative array declared through a package
/// typedef of unpacked-struct elements, run verbatim (§12.7.3, §7.8).
#[test]
fn issue_312_foreach_typedef_assoc_of_structs() {
    let msgs = outputs(
        include_str!("../issue_cases/foreach_aa_typedef_312.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #313: a static task calling another static task of its class by bare
/// name must block like any task call, so `mailbox.get()` waits for the
/// puts at 10 and 20 (§8.10, §13.3, §15.4), run verbatim.
#[test]
fn issue_313_static_task_calls_block() {
    let msgs = outputs(
        include_str!("../issue_cases/static_task_blocking_313.sv"),
        100_000,
    );
    assert_eq!(
        msgs.iter()
            .filter(|m| m.starts_with("[t=") || m.starts_with("TEST"))
            .cloned()
            .collect::<Vec<_>>(),
        [
            "[t=10] execute_phase(42)",
            "[t=20] execute_phase(43)",
            "[t=20] phase runner finished",
            "TEST PASSED",
        ],
        "{:#?}",
        msgs
    );
}

/// Issue #316: an unpacked dimension sized by a class-scoped localparam
/// (`a[2 * model_c::BEATS]`) gets all 146 elements (§8.23, §7.4.2), run
/// verbatim.
#[test]
fn issue_316_class_scoped_dimension() {
    let msgs = outputs(
        include_str!("../issue_cases/class_scoped_dimension_316.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.contains("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "PASS"), "{:#?}", msgs);
}

/// Issue #317: `@(cb iff cb.sig ...)` judges its guard on the clockvars
/// sampled at that clocking event, so the monitor wakes at 25 on the first
/// beat (§14.13), run verbatim.
#[test]
fn issue_317_clocking_iff_sees_this_edge() {
    let msgs = outputs(
        include_str!("../issue_cases/clocking_iff_sample_317.sv"),
        100_000,
    );
    assert!(
        msgs.iter()
            .any(|m| m == "[25] woke: cb.cs_n=0 cb.data=0xf000"),
        "{:#?}",
        msgs
    );
    assert!(msgs.iter().any(|m| m == "PASS"), "{:#?}", msgs);
}

/// Issue #312 follow-up (reporter's survey, 41 cases): `foreach` and the
/// collection methods over typedef'd associative arrays, dynamic arrays and
/// queues at package, module, class, interface and block scope, every case
/// in its own top module, run verbatim. The reference passes all 41.
#[test]
fn issue_312_survey_multi_module() {
    let msgs = outputs(
        include_str!("../issue_cases/foreach_typedef_312_multi_module.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// The same 41-case survey with every case in one module, run verbatim. The
/// reference passes all 41.
#[test]
fn issue_312_survey_single_module() {
    let msgs = outputs(
        include_str!("../issue_cases/foreach_typedef_312_single_module.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #388 follow-up (reporter's survey)
#[test]
fn issue_388_unpacked_arr_in_params() {
    let msgs = outputs(
        include_str!("../issue_cases/unpacked_arr_in_params_388.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #390: a queue-of-structs element pushed as a function's struct
/// return (`q.push_back(make(...))`) keeps its string members when the queue
/// is passed to a function, copied to another queue, or iterated with
/// foreach (§7.10.2.2, §13.4.1). One integrated MWE: the issue's shape plus
/// the foreach-formal, queue-copy, multi-element and mixed-sourcing cousins,
/// with working shapes (ref formal, struct var from a call) as controls.
#[test]
fn issue_390_queue_struct_call_elems() {
    let msgs = outputs(
        include_str!("../issue_cases/issue390_queue_struct_call_elems.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// A call is evaluated once even when a deeply nested struct has no leaf
/// layout available (§13.5.1, §7.10.2.2). Expected counts and queue sizes
/// were checked with the reference simulator.
#[test]
fn queue_struct_value_arguments_run_once() {
    let msgs = outputs(
        include_str!("../issue_cases/queue_struct_value_once.sv"),
        100_000,
    );
    let mut rows: Vec<_> = msgs
        .iter()
        .filter(|m| m.starts_with("T|"))
        .cloned()
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        [
            "T|back|1 1",
            "T|conditional|2 2",
            "T|front|1 1",
            "T|insert|1 1",
            "T|method|1 2",
            "T|parenthesized|2 2",
        ],
        "{msgs:#?}",
    );
}

/// A full-word first concatenation part is copied without a shift by 64
/// (§11.4.12). Adjacent widths, nesting and multi-part forms preserve both
/// value and X/Z planes. Expected rows match the reference simulator.
#[test]
fn full_word_concatenation_preserves_value_and_xz_planes() {
    let out = outputs(
        include_str!("../issue_cases/narrow_concat_boundaries.sv"),
        1000,
    );
    let mut rows: Vec<_> = out.into_iter().filter(|m| m.starts_with("T|")).collect();
    rows.sort();
    assert_eq!(
        rows,
        [
            "T|boundaries|x123456789abcdef 10123456789abcdez x123456789abcdezx123456789abcdez",
            "T|known_boundaries|7123456789abcdef 10123456789abcdef fedcba9876543210fedcba9876543210",
            "T|known|fedcba9876543210 fedcba9876543210 fedcba9876543210 fedcba9876543210 fedcba9876543210",
            "T|unknown|x123456789abcdez x123456789abcdez x123456789abcdez x123456789abcdez x123456789abcdez",
        ]
    );
}

/// Issue #391: appending one-character string casts — out = {out, string'(c)}
/// — padded every character with 127 NUL bytes once the function tiered up
/// to bytecode ("HELLO" came back with len 513 = 1 + 4*128). The cast now
/// converts to a string value (§6.16) and the concat takes the §11.4.12
/// string path; bare and prepended casts are covered as cousins.
#[test]
fn issue_391_string_cast_concat_padding() {
    let msgs = outputs(
        include_str!("../issue_cases/issue391_string_cast_concat.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

#[test]
fn issue_427_non_static_vif_elem_null() {
    let msgs = outputs(
        include_str!("../issue_cases/non_static_vif_elem_null_427.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

/// A static virtual-interface member reached through a TYPEDEF ALIAS of its
/// class never resolved. A static property belongs to the class, not to the
/// spelling used to reach it, so after `typedef box_c box_t;` `box_t::svif` is
/// the very same cell as `box_c::svif` — but the vif paths keyed class-scope
/// cells by the source name, so an alias forked a second cell: writing through
/// the alias landed where no real-class read looked, and reading through the
/// alias consulted a cell no write had filled. Both directions read as an
/// unbound vif.
#[test]
fn issue_430_static_vif_typedef_alias() {
    let msgs = outputs(
        include_str!("../issue_cases/static_vif_typedef_alias_430.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

// ---------------------------------------------------------------------------
// Static virtual-interface cells for qualified receivers (§8.9/§25.10/§26.3).
//
// A static property has ONE binding cell, keyed to its class (or
// specialization) at handle 0 and held in `static_vif_bindings`. A class-scope
// read resolved only through the per-object `virtual_iface_bindings` table,
// which cannot hold a static cell, and the chain rewriter's entry gate never
// consulted the static map at all — so a design whose only bound vifs are class
// statics was rejected before its chain could be walked. Both the package
// qualifier and the `#(spec)` qualifier then had no way to name the class.
// Rename these to issueNNN_* once the GitHub numbers are assigned.
// ---------------------------------------------------------------------------

/// A static virtual-interface member read as a VALUE yielded a placeholder, not
/// the bound interface handle. Member access through it worked
/// (`S::sa.count` read the right interface), but `==`/`!=` and use as an actual
/// argument did not see the binding: two statics bound to two different
/// interfaces always compared EQUAL, and a static never compared equal to the
/// interface it was bound to. A class-scope read reaches the comparison
/// classifier as `Ident([Holder]).sa`, and `eval_ident_handle` cannot name a
/// class at all, so the operand classified as unclassifiable and fell through
/// to value semantics.
#[test]
fn issue_428_static_vif_handle_value() {
    let msgs = outputs(
        include_str!("../issue_cases/static_vif_handle_value_428.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// A BARE read of a static virtual-interface member reported null while bound.
/// Inside an instance method the bare name was looked up only in the per-object
/// table, which cannot hold a static cell; inside a STATIC method there is no
/// object at all, so the operand fell out of the classifier entirely; and
/// `this.svif` parses as `MemberAccess { expr: This, .. }`, which neither the
/// one-segment ident arm nor `chain_handle` (Ident chains only) could name.
/// Member access through the same static was always correct, which is why only
/// the scalar/null reads failed.
#[test]
fn issue_432_static_vif_instance_method() {
    let msgs = outputs(
        include_str!("../issue_cases/static_vif_instance_method_432.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// #431: a static virtual-interface member of a PARAMETERIZED class never
/// resolved through the specialization `C#(T)::svif`. Write and read agreed on
/// the key (`a_pm_c#(int)::svif`), yet the qualified member read came back 0
/// while the same class bound through a static method read back correctly
/// through the very same spelling. `vif_prop_target` resolved a class-scope read
/// (handle 0) only through the per-object table, and `vif_rebase_expr`'s entry
/// gate tested `iface_alias_possible`, `virtual_iface_bindings` and
/// `vif_tokens_stored` but never `static_vif_bindings`. Per-specialization cells
/// (§8.26) must stay distinct, so the fix may not collapse the spec key.
#[test]
fn static_vif_param_class_431() {
    let msgs = outputs(
        include_str!("../issue_cases/static_vif_param_class_431.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// #431, the reporter's own case. A parameterized class keeps one static cell
/// per specialization (§8.26) and the write side files the binding under the
/// canonical spec key `C#(T)::p`; the class-scope reads went through `vif_slot`,
/// which canonicalises via `vif_class_of_scope_name` and DISCARDS the signature,
/// so they asked for `C::p` and read a bound static as null. The member-read
/// path does resolve the spec, which is why a member read could succeed while
/// `== null` on the same static still reported unbound — the comparison
/// classifier and the member read disagreed about which cell a static occupies.
/// Bare reads inside a static method are the other half: with no `this`, the
/// name resolves through the executing class context, which must carry the
/// specialization the call was made on. E is the check that stops the fix
/// collapsing the two specialization cells into one.
#[test]
fn static_vif_param_null_bare_431() {
    let msgs = outputs(
        include_str!("../issue_cases/static_vif_param_null_bare_431.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// #433: a frame-local virtual interface failed to shadow the class static of
/// the same name. A static property has one cell, keyed to the declaring class
/// at handle 0, and a bare vif name inside a method resolved against it on
/// BOTH sides without ever asking the executing frame (§6.21/§8.4). The read
/// side fell back to the class cell, so A's never-assigned local read the
/// bound static; the write side resolved a static method's missing `this` to
/// handle 0 and CLEARED that cell, so a declaration's synthesised initializer
/// assignment (`virtual counter_if svif = null;`) destroyed the binding for
/// every later reader — which is why D, a method with no local at all, failed.
/// The controls keep the fix honest: C and D must still reach the class
/// static, so neither guard may widen to "a bare name never reaches a static".
#[test]
fn static_vif_local_shadow_433() {
    let msgs = outputs(
        include_str!("../issue_cases/static_vif_local_shadow_433.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// #1050: the reporter's sweep of nonblocking-assignment shapes, simulated
/// once and checked per topic below. Each test compares the `SHAPE` lines of
/// its shapes with the reference simulator's transcript of the same file;
/// a failed self-check prints a `FAIL: sNN` line, which never matches.
const NBA_SHAPES_1050_REF: &[&str] = &[
    r#"SHAPE s01: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s02: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s03: vld=1 wr=1 addr=000000ba0b tag=1 sz=1 len=03 attr=1 mode=10 hold=1"#,
    r#"SHAPE s04: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s05: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s06: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s07: vec=c00000303792002 bit59=1"#,
    r#"SHAPE s08: vec=c00000303792002 bits33_30=1100"#,
    r#"SHAPE s09: hdr.vld=1 hdr.tag=5 addr=000000face"#,
    r#"SHAPE s10: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s11: vld=1 addr=0000000ff1 len=07"#,
    r#"SHAPE s12[0]: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s12[1]: vld=0 wr=1 addr=000000ba0b tag=1 sz=1 len=03 attr=1 mode=10 hold=1"#,
    r#"SHAPE s13: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s13: a_vld_events=5"#,
    r#"SHAPE s14: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s15: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s16: bus_b.vld=1 data=d00df00d strb=f last=1 b_vld_events=4"#,
    r#"SHAPE s17: uni=deadbeef w=deadbeef want=deadbeef"#,
    r#"SHAPE s18: n3=de000000face hdr2.vld=1 hdr2.tag=6"#,
    r#"SHAPE s19: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s20: v20=c00000303792002 want=c00000303792002"#,
    r#"SHAPE s21: v21=c00000303792002 want=c00000303792002"#,
    r#"SHAPE s22: v22=c00000303792002 want=c00000303792002"#,
    r#"SHAPE s23: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s24: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s25: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s26: vld=0 wr=0 addr=0000000000 tag=0 sz=0 len=00 attr=0 mode=00 hold=0"#,
    r#"SHAPE s27: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=11 attr=0 mode=01 hold=0"#,
    r#"SHAPE s28: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s29: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s30: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s31: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s32: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s33: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s34: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s35: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s36: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s37: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s38: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s39: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s40: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s41: hdl2.vec=deadbeefcafef00d want=deadbeefcafef00d"#,
    r#"SHAPE s42: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s43: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s44: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s45: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s46: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s47: wh.w32=ffffffff want=ffffffff"#,
    r#"SHAPE s48: wh.w33=1ffffffff want=1ffffffff"#,
    r#"SHAPE s49: wh.w64=0123456789abcdef want=0123456789abcdef"#,
    r#"SHAPE s50: wh.r=-1.500000 want=-1.500000"#,
    r#"SHAPE s51: wh.w64=0123456789abcdef wh.r=-1.500000 want=0123456789abcdef -1.500000"#,
    r#"SHAPE s52: wh.r=-1.500000 want=-1.500000"#,
    r#"SHAPE s53: static_holder::sv=0123456789abcdef want=0123456789abcdef"#,
    r#"SHAPE s54: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s61: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s55: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s56: dh.bp=c00000303792002 want=c00000303792002"#,
    r#"SHAPE s57: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s58: ci.bus_a=c00000303792002 ci.drv.bus_a=c00000303792002 want=c00000303792002"#,
    r#"SHAPE s59: vld=1 wr=1 addr=000000c0de tag=2 sz=2 len=00 attr=0 mode=01 hold=0"#,
    r#"SHAPE s60: static_holder::sp=c00000303792002 sp.vld=1 sp.wr=1 want=c00000303792002 1 1"#,
];

/// The `SHAPE`/`FAIL` lines of shapes `ids`: (simulated, reference).
fn nba_shapes_1050(ids: &[&str]) -> (Vec<String>, Vec<String>) {
    static OUT: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let msgs = OUT.get_or_init(|| {
        outputs(
            include_str!("../issue_cases/nba_struct_shapes_1050.sv"),
            100_000,
        )
    });
    let picked = |l: &str| {
        l.strip_prefix("SHAPE ")
            .or_else(|| l.strip_prefix("FAIL: "))
            .map(|r| {
                r.split(|c: char| !c.is_ascii_alphanumeric())
                    .next()
                    .unwrap_or("")
            })
            .is_some_and(|id| ids.contains(&id))
    };
    let got = msgs.iter().filter(|l| picked(l)).cloned().collect();
    let want = NBA_SHAPES_1050_REF
        .iter()
        .filter(|l| picked(l))
        .map(|l| l.to_string())
        .collect();
    (got, want)
}

/// §4.6 b), §10.4.2: nonblocking updates to one variable commit in the order
/// they were scheduled, so a whole-variable update after a partial one (a
/// member, a bit- or part-select, a concatenation piece) wins in every scope
/// and statement context; the reverse order keeps the partial (#1050).
#[test]
fn nba_partial_then_whole_shapes_1050() {
    let ids = [
        "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09", "s10", "s11", "s12", "s13",
        "s14", "s15", "s16", "s17", "s18", "s19", "s20", "s21", "s22", "s23", "s24", "s26", "s27",
        "s28", "s29", "s30", "s31", "s32", "s33", "s34", "s35", "s36", "s37", "s38", "s39", "s44",
        "s45", "s46",
    ];
    let (got, want) = nba_shapes_1050(&ids);
    assert_eq!(got, want);
}

/// §10.4.2, §8.5: a nonblocking update of a class property is sized by the
/// property's declared type — through a handle, `this`, `super` or the class
/// name, wider than 32 bits or `real` — and lands in the property, not in a
/// module signal of the same name (#1050).
#[test]
fn class_property_nba_shapes_1050() {
    let ids = [
        "s25", "s40", "s41", "s42", "s43", "s47", "s48", "s49", "s50", "s51", "s52", "s53", "s54",
        "s55", "s56", "s57", "s61",
    ];
    let (got, want) = nba_shapes_1050(&ids);
    assert_eq!(got, want);
}
