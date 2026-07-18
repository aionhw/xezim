//! IEEE 1800-2017 §21.7 — VCD (value change dump) compliance.
//!
//! Before these fixes:
//!   §21.7.1.4  `$dumpvars(0, top)` matched NOTHING — the scope filter compared
//!              the resolved absolute path (`top`) against signal names that are
//!              relative to the top module (`clk`, `u_sub.c`) — and the dump list
//!              was read from a lazily-synced mirror of the signal table that is
//!              EMPTY unless something dirtied the table first, so a purely
//!              behavioral design dumped nothing at all. The depth argument was
//!              ignored outright.
//!   §21.7.2.1  every `$var` was hardcoded `wire` (reg/integer/time/real/event/
//!              parameter alike); `real` was dumped as a 64-bit binary vector of
//!              raw IEEE-754 bits instead of an `r<decimal>` record; leading-zero
//!              suppression corrupted x/z vectors (`8'b000000x1` → `bx1`, which a
//!              reader x-extends back to `8'bxxxxxxx1`); no bit range was emitted;
//!              module instances / enum literals were dumped as stuck-at-x wires;
//!              an `event` was dumped as a level signal, so a repeat `->ev` whose
//!              toggle cancelled inside one time slot emitted nothing.
//!   §21.7.1.5-9 `$dumpall`, `$dumpflush` and `$dumplimit` did not exist, and
//!              `$dumpoff`/`$dumpon` flipped a bool without emitting anything —
//!              so a viewer painted a stale, false waveform across the whole
//!              off-window instead of X.
//!   §21.7.2    no `#<time>` marker preceded the `$dumpvars` block (a dump started
//!              mid-run landed at t=0) and none closed the file at `$finish`.
//!
//! Each test runs a design through the library, then asserts on the VCD TEXT.

use std::path::PathBuf;
use xezim::simulate;

/// Run `src` (with `{VCD}` replaced by a unique temp path) and return the VCD text.
fn dump(tag: &str, src: &str) -> String {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("xezim_vcd_lrm_{}_{}.vcd", tag, std::process::id()));
    let _ = std::fs::remove_file(&path);
    let src = src.replace("{VCD}", path.to_str().unwrap());
    let _sim = simulate(&src, 1_000_000).expect("simulate failed");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("no VCD written to {}: {}", path.display(), e));
    let _ = std::fs::remove_file(&path);
    text
}

/// The `$var` declaration line for `name`.
fn var_line(vcd: &str, name: &str) -> String {
    vcd.lines()
        .find(|l| {
            l.starts_with("$var")
                && l.split_whitespace().nth(4) == Some(name)
        })
        .unwrap_or_else(|| panic!("no $var for `{}` in:\n{}", name, vcd))
        .to_string()
}

/// The one-character-or-more identifier code assigned to `name`.
fn id_of(vcd: &str, name: &str) -> String {
    var_line(vcd, name)
        .split_whitespace()
        .nth(3)
        .unwrap()
        .to_string()
}

/// Every `$var` line whose reference is `name`, across ALL scopes — the same
/// leaf name appears once per instance that declares it (`clk` in `top`, in
/// `u_mid` and in `u_leaf`).
fn var_lines_all(vcd: &str, name: &str) -> Vec<String> {
    let v: Vec<String> = vcd
        .lines()
        .filter(|l| l.starts_with("$var") && l.split_whitespace().nth(4) == Some(name))
        .map(|l| l.to_string())
        .collect();
    assert!(!v.is_empty(), "no $var for `{}` in:\n{}", name, vcd);
    v
}

/// The identifier codes of every `$var` named `name`, in file order.
fn ids_of_all(vcd: &str, name: &str) -> Vec<String> {
    var_lines_all(vcd, name)
        .iter()
        .map(|l| l.split_whitespace().nth(3).unwrap().to_string())
        .collect()
}

/// Every value-change record emitted for `name`, in file order, without its id.
fn records(vcd: &str, name: &str) -> Vec<String> {
    let id = id_of(vcd, name);
    let body = vcd.split("$enddefinitions $end").nth(1).unwrap_or("");
    body.lines()
        .filter_map(|l| {
            if let Some(rest) = l.strip_suffix(&format!(" {}", id)) {
                // Vector / real record: `b1010 <id>` or `r3.14 <id>`.
                Some(rest.to_string())
            } else if l.len() > id.len() && l.ends_with(&id) && !l.starts_with('$') {
                // Scalar record: `1<id>`.
                let (v, tail) = l.split_at(l.len() - id.len());
                if tail == id && v.len() == 1 {
                    Some(v.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

const HIER: &str = r#"
`timescale 1ns/1ns
module leaf(input logic a, output logic b);
  assign b = ~a;
endmodule
module sub(input logic x, output logic y);
  logic c;
  leaf u_leaf(.a(x), .b(c));
  assign y = c;
endmodule
module top;
  logic clk;
  logic w;
  sub u_sub(.x(clk), .y(w));
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, top);
    clk = 0;
    #5 clk = 1;
    #5 $finish;
  end
endmodule
"#;

/// §21.7.1.4: `$dumpvars(0, top)` names the TOP MODULE — every object in the
/// design is below it, so all of them are dumped, in a nested scope tree.
#[test]
fn dumpvars_of_the_top_module_scope_dumps_the_whole_design() {
    let vcd = dump("hier", HIER);
    assert!(vcd.contains("$scope module top $end"), "{}", vcd);
    assert!(vcd.contains("$scope module u_sub $end"), "{}", vcd);
    assert!(vcd.contains("$scope module u_leaf $end"), "{}", vcd);
    // Top-level, one level down and two levels down are all present.
    for sig in ["clk", "w", "c", "b"] {
        var_line(&vcd, sig);
    }
    // The `$dumpvars` checkpoint states clk's initial x; it then toggles 0 → 1.
    assert_eq!(records(&vcd, "clk"), vec!["x", "0", "1"]);
    assert!(vcd.contains("#5"), "missing time marker:\n{}", vcd);
}

/// §21.7.1.4: the dump list comes from the real signal table, so a design that
/// never dirtied it before `$dumpvars` (a purely behavioral module — no nets, no
/// continuous assigns) still dumps its variables. It used to produce an empty file.
#[test]
fn a_purely_behavioral_design_is_not_dumped_empty() {
    let vcd = dump(
        "behav",
        r#"
module behav;
  logic [3:0] a;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars;
    a = 4'd1;
    #1 a = 4'd2;
    #1 $finish;
  end
endmodule
"#,
    );
    assert_eq!(var_line(&vcd, "a"), format!("$var reg 4 {} a [3:0] $end", id_of(&vcd, "a")));
    // `bx` — an all-x vector left-extends back to full width (§21.7.2.1), and is
    // what a reference simulator writes. See `leading_run_suppression_matches_reference_...`.
    assert_eq!(records(&vcd, "a"), vec!["bx", "b1", "b10"]);
}

/// §21.7.1.4: the depth argument. `1` = only the named scope's own level;
/// `0` = that scope and every level below it.
#[test]
fn dumpvars_depth_limits_how_far_below_the_scope_the_dump_reaches() {
    const SRC: &str = r#"
module leaf; logic deep; initial deep = 1; endmodule
module mid; logic m; leaf u_leaf(); initial m = 0; endmodule
module top;
  logic t;
  mid u_mid();
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(DEPTH, top);
    t = 1;
    #2 $finish;
  end
endmodule
"#;
    let one = dump("depth1", &SRC.replace("DEPTH", "1"));
    var_line(&one, "t");
    assert!(!one.contains("$scope module u_mid"), "depth 1 must not descend:\n{}", one);
    assert!(!one.contains(" m $end"), "depth 1 must not descend:\n{}", one);

    let all = dump("depth0", &SRC.replace("DEPTH", "0"));
    var_line(&all, "t");
    var_line(&all, "m");
    var_line(&all, "deep");

    // A scope argument BELOW the top selects just that subtree.
    let sub = dump(
        "depthsub",
        &SRC.replace("DEPTH, top", "0, top.u_mid"),
    );
    var_line(&sub, "m");
    var_line(&sub, "deep");
    assert!(!sub.contains(" t $end"), "u_mid subtree must not carry `t`:\n{}", sub);
}

/// §21.7.2.1: a `real` is declared `$var real 64` and its changes are
/// `r<decimal_number>` records — NOT a 64-bit binary vector of raw IEEE-754 bits.
#[test]
fn real_variables_use_the_r_record_form() {
    let vcd = dump(
        "real",
        r#"
module tb;
  real r;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    r = 0.0;
    #1 r = 3.14;
    #1 r = -2.5;
    #1 $finish;
  end
endmodule
"#,
    );
    assert_eq!(var_line(&vcd, "r"), format!("$var real 64 {} r $end", id_of(&vcd, "r")));
    assert_eq!(records(&vcd, "r"), vec!["r0", "r3.14", "r-2.5"]);
    assert!(
        !vcd.contains("b0100000000001001"),
        "real must not be dumped as its IEEE-754 bit pattern:\n{}",
        vcd
    );
}

/// §21.7.2.1: a reader LEFT-EXTENDS a value shorter than the `$var` width with
/// its LEFTMOST character, so leading zeros may only be dropped while the first
/// retained character is `1`. `8'b000000x1` must not collapse to `bx1` — that
/// reads back as `8'bxxxxxxx1`.
#[test]
fn leading_zero_suppression_never_corrupts_an_x_or_z_vector() {
    let vcd = dump(
        "xz",
        r#"
module tb;
  logic [7:0] a, b, c, d;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    a = 8'b000000x1;   // first significant bit is x → keep one leading 0
    b = 8'b0000zz11;   // first significant bit is z → keep one leading 0
    c = 8'b00001101;   // first significant bit is 1 → zeros may be dropped
    d = 8'b00000000;   // all zero → collapses to a single 0
    #1 $finish;
  end
endmodule
"#,
    );
    assert_eq!(records(&vcd, "a").last().unwrap(), "b0x1");
    assert_eq!(records(&vcd, "b").last().unwrap(), "b0zz11");
    assert_eq!(records(&vcd, "c").last().unwrap(), "b1101");
    assert_eq!(records(&vcd, "d").last().unwrap(), "b0");
}

/// §21.7.2.1: the `var_type` of each declaration, and the optional bit range on
/// the reference. Everything used to be `$var wire <w> <id> <name> $end`.
#[test]
fn var_declarations_carry_the_right_type_and_bit_range() {
    let vcd = dump(
        "types",
        r#"
module tb;
  wire        n = 1'b0;
  reg  [3:0]  r;
  integer     i;
  time        t;
  real        f;
  event       e;
  logic [15:8] hi;   // non-zero-based range
  logic [0:7]  asc;  // ascending range
  parameter P = 7;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    r = 0; i = 0; t = 0; f = 0.0; hi = 0; asc = 0;
    #1 $finish;
  end
endmodule
"#,
    );
    let l = |n: &str| var_line(&vcd, n);
    assert!(l("n").starts_with("$var wire 1 "), "{}", l("n"));
    assert!(l("r").starts_with("$var reg 4 "), "{}", l("r"));
    assert!(l("i").starts_with("$var integer 32 "), "{}", l("i"));
    assert!(l("t").starts_with("$var time 64 "), "{}", l("t"));
    assert!(l("f").starts_with("$var real 64 "), "{}", l("f"));
    assert!(l("e").starts_with("$var event 1 "), "{}", l("e"));
    assert!(l("P").starts_with("$var parameter 32 "), "{}", l("P"));
    // §21.7.2.1 bit range: `logic [15:8] hi` is NOT `[7:0]`, and an ascending
    // vector keeps its own bit order.
    assert!(l("hi").ends_with(" hi [15:8] $end"), "{}", l("hi"));
    assert!(l("asc").ends_with(" asc [0:7] $end"), "{}", l("asc"));
}

/// §21.7.2.1: an `event` has no level — it emits a bare `1<id>` record at EVERY
/// trigger. Dumping it as a level signal with prev!=cur dedup dropped a repeat
/// `->ev` (its 0→1→0 toggle cancels inside one time slot) and painted the viewer
/// a meaningless square wave.
#[test]
fn an_event_emits_a_pulse_at_every_trigger() {
    let vcd = dump(
        "event",
        r#"
module tb;
  event ev;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    #1 ->ev;
    #1 ->ev;
    #1 ->ev;
    #1 $finish;
  end
endmodule
"#,
    );
    assert!(var_line(&vcd, "ev").starts_with("$var event 1 "), "{}", vcd);
    // Three triggers → three `1<id>` pulses, and no initial-value record.
    assert_eq!(records(&vcd, "ev"), vec!["1", "1", "1"], "{}", vcd);
}

/// §21.7.2.1: only nets and variables are objects. Module INSTANCES and enum
/// LITERALS live in the signal table too, and used to be dumped as stuck-at-x
/// 1-bit wires sitting beside the scope of the same name.
#[test]
fn instances_and_enum_literals_are_not_dumped_as_signals() {
    let vcd = dump(
        "nonsig",
        r#"
typedef enum logic [1:0] { RED, GRN, BLU } color_e;
module leaf; logic z; initial z = 0; endmodule
module tb;
  color_e col;
  leaf u_leaf();
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    col = BLU;
    #1 $finish;
  end
endmodule
"#,
    );
    var_line(&vcd, "col");
    // `u_leaf` is a SCOPE, not an object — it must not also be a $var.
    assert!(vcd.contains("$scope module u_leaf $end"), "{}", vcd);
    let declared: Vec<&str> = vcd
        .lines()
        .filter(|l| l.starts_with("$var"))
        .filter_map(|l| l.split_whitespace().nth(4))
        .collect();
    for bogus in ["u_leaf", "RED", "GRN", "BLU"] {
        assert!(
            !declared.contains(&bogus),
            "`{}` is not an object and must not get a $var:\n{}",
            bogus,
            vcd
        );
    }
    assert_eq!(declared, vec!["col", "z"], "{}", vcd);
}

/// §21.7.2.1: an unpacked array is dumped ELEMENT-WISE (`mem[0]`…). Net arrays
/// used to be flattened into one wide vector while variable arrays expanded.
#[test]
fn unpacked_net_and_variable_arrays_both_expand_element_wise() {
    let vcd = dump(
        "arrays",
        r#"
module tb;
  wire  [3:0] outs [0:1];
  logic [7:0] mem  [0:1];
  assign outs[0] = 4'd5;
  assign outs[1] = 4'd6;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    mem[0] = 8'h11;
    mem[1] = 8'h22;
    #1 $finish;
  end
endmodule
"#,
    );
    assert!(var_line(&vcd, "outs[0]").starts_with("$var wire 4 "), "{}", vcd);
    assert!(var_line(&vcd, "outs[1]").starts_with("$var wire 4 "), "{}", vcd);
    assert!(var_line(&vcd, "mem[0]").starts_with("$var reg 8 "), "{}", vcd);
    // The aggregate must NOT also appear as one wide vector.
    assert!(!vcd.contains(" outs $end"), "net array must not flatten:\n{}", vcd);
    assert_eq!(records(&vcd, "outs[0]").last().unwrap(), "b101");
    assert_eq!(records(&vcd, "outs[1]").last().unwrap(), "b110");
    assert_eq!(records(&vcd, "mem[1]").last().unwrap(), "b100010");
}

/// §21.7.1.6 / §21.7.1.7: `$dumpoff` must mark the suspended window — every
/// dumped variable goes to x — and `$dumpon` must restate every current value.
/// Flipping a bool and emitting nothing leaves a stale, FALSE level on screen
/// across the whole off-window.
#[test]
fn dumpoff_marks_the_window_x_and_dumpon_restates_every_value() {
    let vcd = dump(
        "onoff",
        r#"
module tb;
  logic [3:0] a;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    a = 4'd1;
    #1 $dumpoff;
    #1 a = 4'd7;      // inside the off-window: must not be dumped
    #1 $dumpon;
    #1 $finish;
  end
endmodule
"#,
    );
    let body = vcd.split("$enddefinitions $end").nth(1).unwrap();
    let off = body.find("$dumpoff").expect("no $dumpoff block");
    let on = body.find("$dumpon").expect("no $dumpon block");
    assert!(off < on);
    // The off block drives `a` to x, and is time-stamped.
    assert!(body[off..on].contains("bx "), "$dumpoff must dump x:\n{}", vcd);
    assert!(body[..off].trim_end().ends_with("#1"), "$dumpoff needs a #t:\n{}", vcd);
    // Nothing was emitted for `a` between the two blocks...
    assert!(!body[off..on].contains("b111 "), "off-window change leaked:\n{}", vcd);
    // ...and $dumpon restates the value it reached while off.
    assert!(body[on..].contains("b111 "), "$dumpon must restate values:\n{}", vcd);
}

/// §21.7.1.5: `$dumpall` writes a checkpoint of every dumped variable's current
/// value. It did not exist (no match arm at all).
#[test]
fn dumpall_writes_a_checkpoint_of_every_variable() {
    let vcd = dump(
        "dumpall",
        r#"
module tb;
  logic [3:0] a;
  logic       b;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    a = 4'd9; b = 1'b1;
    #1 $dumpall;
    $dumpflush;
    #1 $finish;
  end
endmodule
"#,
    );
    let body = vcd.split("$enddefinitions $end").nth(1).unwrap();
    let at = body.find("$dumpall").expect("no $dumpall block");
    let block = &body[at..body[at..].find("$end").unwrap() + at];
    assert!(block.contains(&format!("b1001 {}", id_of(&vcd, "a"))), "{}", vcd);
    assert!(block.contains(&format!("1{}", id_of(&vcd, "b"))), "{}", vcd);
}

/// §21.7.1.2 / §21.7.2: the `$dumpvars` checkpoint is stamped with the CURRENT
/// time (a dump started mid-run used to land at t=0 in every viewer), the run's
/// final time closes the file, and a SECOND `$dumpvars` must not re-create the
/// file — the running sink is still draining into it, so a second `File::create`
/// interleaved two byte streams into one corrupt file.
#[test]
fn a_midrun_dump_is_time_stamped_and_a_second_dumpvars_does_not_corrupt_it() {
    let vcd = dump(
        "twice",
        r#"
`timescale 1ns/1ns
module tb;
  logic t;
  initial begin
    $dumpfile("{VCD}");
    #23;
    $dumpvars(0, tb);
    t = 1;
    #2 $dumpvars(0, tb);
    #2 $finish;
  end
endmodule
"#,
    );
    // Exactly one header — the second $dumpvars did not restart the file.
    assert_eq!(vcd.matches("$enddefinitions $end").count(), 1, "{}", vcd);
    assert_eq!(vcd.matches("$var ").count(), 1, "{}", vcd);
    let body = vcd.split("$enddefinitions $end").nth(1).unwrap();
    // The checkpoint sits at the time the dump actually started.
    assert!(body.trim_start().starts_with("#23\n$dumpvars"), "{}", body);
    // The repeat call degrades to a $dumpall checkpoint at #25.
    assert!(body.contains("#25\n$dumpall"), "{}", body);
    // §21.7.2: the file is closed at the final simulation time, so the last
    // value spans to the end of the run instead of stopping at #23.
    assert!(body.trim_end().ends_with("#27"), "no closing time marker:\n{}", body);
}

/// A net threaded down three levels of hierarchy through whole-net port
/// connections, plus a same-named (`clk`) and a differently-named (`src` →
/// `mdin` → `din`) binding.
const PORT_HIER: &str = r#"
`timescale 1ns/1ps
module leaf(input logic clk, input logic [7:0] din, output logic [7:0] dout);
  always @(posedge clk) dout <= din + 8'd1;
endmodule
module mid(input logic clk, input logic [7:0] mdin, output logic [7:0] mdout);
  leaf u_leaf(.clk(clk), .din(mdin), .dout(mdout));
endmodule
module top;
  logic clk = 0;
  logic [7:0] src, snk;
  mid u_mid(.clk(clk), .mdin(src), .mdout(snk));
  always #5 clk = ~clk;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, top);
    src = 8'h10;
    #20 src = 8'h20;
    #20 $finish;
  end
endmodule
"#;

/// §21.7.2.1: "several variables can be mapped to the same identifier code if
/// the variables would always have identical values" — which is exactly a net
/// connected through an instance port. Verilator and a reference simulator both give
/// `src_bus`/`u_sub.din` ONE code and emit ONE value-change record per change;
/// xezim gave them separate codes and wrote every change TWICE (three times at
/// three levels), roughly doubling the file on any hierarchical design and
/// presenting one physical net as two independent signals.
///
/// The alias must resolve through a CHAIN (a port bound to a port bound to a
/// port) to the outermost net, and each name still gets its own `$var` in its
/// own `$scope` — that is what both reference tools emit.
#[test]
fn port_connected_nets_share_one_identifier_code() {
    let vcd = dump("portalias", PORT_HIER);

    // Every scope still declares its own object: the hierarchy is intact.
    assert!(vcd.contains("$scope module u_mid $end"), "{}", vcd);
    assert!(vcd.contains("$scope module u_leaf $end"), "{}", vcd);
    assert_eq!(var_lines_all(&vcd, "clk").len(), 3, "{}", vcd);

    // …but the three `clk` declarations name ONE net, three levels deep.
    let clk = ids_of_all(&vcd, "clk");
    assert!(
        clk.iter().all(|c| *c == clk[0]),
        "port-connected `clk` must share one id code, got {:?}\n{}",
        clk,
        vcd
    );
    // Differently-named formals alias just the same: src → mdin → din.
    assert_eq!(id_of(&vcd, "src"), id_of(&vcd, "mdin"), "{}", vcd);
    assert_eq!(id_of(&vcd, "src"), id_of(&vcd, "din"), "{}", vcd);
    // …and so does an output port bound up the chain: dout → mdout → snk.
    assert_eq!(id_of(&vcd, "snk"), id_of(&vcd, "mdout"), "{}", vcd);
    assert_eq!(id_of(&vcd, "snk"), id_of(&vcd, "dout"), "{}", vcd);

    // ONE record per change, not one per hierarchical name: the checkpoint x,
    // then each of the two `src` writes exactly once.
    assert_eq!(records(&vcd, "src"), vec!["bx", "b10000", "b100000"], "{}", vcd);
    // The clock's 8 edges over 40ns, once each.
    assert_eq!(records(&vcd, "clk").len(), 1 + 8, "{}", vcd);
}

/// §21.7.2.1: only a WHOLE-net actual is the same object as the formal. A
/// bit-select, a concatenation or an expression actual is a distinct object —
/// Verilator and a reference simulator both give those their own identifier code — so the
/// aliasing must not over-reach and collapse them.
#[test]
fn a_port_bound_to_a_bit_select_concat_or_expression_is_not_aliased() {
    let vcd = dump(
        "portnoalias",
        r#"
`timescale 1ns/1ps
module sub(input logic [3:0] p_bit, input logic [3:0] p_cat, input logic [3:0] p_expr,
           input logic [3:0] p_whole, output logic [3:0] o);
  assign o = p_whole ^ p_bit;
endmodule
module top;
  logic [7:0] bus;
  logic [3:0] nib, w;
  logic [1:0] lo, hi2;
  sub u_sub(.p_bit(bus[3:0]), .p_cat({lo,hi2}), .p_expr(w + 4'd1), .p_whole(w), .o(nib));
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, top);
    bus = 8'hA5; lo = 2'b01; hi2 = 2'b10; w = 4'd9;
    #10 w = 4'd3;
    #10 $finish;
  end
endmodule
"#,
    );
    // A part of a net, a concat of two nets and a function of a net are all
    // separate objects — each keeps its own code.
    assert_ne!(id_of(&vcd, "p_bit"), id_of(&vcd, "bus"), "{}", vcd);
    let distinct = [
        id_of(&vcd, "p_bit"),
        id_of(&vcd, "p_cat"),
        id_of(&vcd, "p_expr"),
        id_of(&vcd, "bus"),
        id_of(&vcd, "lo"),
        id_of(&vcd, "hi2"),
    ];
    for (i, a) in distinct.iter().enumerate() {
        for b in &distinct[i + 1..] {
            assert_ne!(a, b, "non-whole-net port actuals must not alias:\n{}", vcd);
        }
    }
    // The whole-net actuals in the SAME instance still do alias — both
    // directions (an input's actual and an output's actual).
    assert_eq!(id_of(&vcd, "p_whole"), id_of(&vcd, "w"), "{}", vcd);
    assert_eq!(id_of(&vcd, "o"), id_of(&vcd, "nib"), "{}", vcd);
}

/// §21.7.1.4: aliasing is a property of the dump, not of the design, so it may
/// not leak past the scope/depth filter. `$dumpvars(1, top)` must still stop at
/// the top level, and a dump rooted INSIDE the hierarchy — where the parent net
/// of an aliased port is not dumped at all — must fall back to giving the formal
/// its own code and its own records.
#[test]
fn aliasing_does_not_break_dumpvars_depth_or_scope_filtering() {
    let one = dump("aliasdepth1", &PORT_HIER.replace("$dumpvars(0, top)", "$dumpvars(1, top)"));
    // Depth 1: only the top level's own objects.
    var_line(&one, "clk");
    assert_eq!(var_lines_all(&one, "clk").len(), 1, "{}", one);
    assert!(!one.contains("$scope module u_mid"), "depth 1 must not descend:\n{}", one);
    assert_eq!(records(&one, "clk").len(), 1 + 8, "{}", one);

    // A scope argument below the top: `src` is outside the dump, so `mdin` is
    // the outermost dumped name of the net and carries the records itself.
    let sub = dump(
        "aliasscope",
        &PORT_HIER.replace("$dumpvars(0, top)", "$dumpvars(0, top.u_mid)"),
    );
    assert!(!sub.contains(" src $end"), "u_mid subtree must not carry `src`:\n{}", sub);
    assert_eq!(id_of(&sub, "mdin"), id_of(&sub, "din"), "{}", sub);
    assert_eq!(records(&sub, "mdin"), vec!["bx", "b10000", "b100000"], "{}", sub);
}

/// §21.7.2.1: an unpacked-array ELEMENT carries the bit range of the element,
/// exactly as Verilator emits it (`$var wire 8 ) mem[0] [7:0] $end`). Without it
/// a viewer has no bit numbering for any array element.
#[test]
fn unpacked_array_elements_carry_their_bit_range() {
    let vcd = dump(
        "arrayrange",
        r#"
module tb;
  reg [7:0]   mem [0:2];
  logic [15:8] hi  [0:1];   // non-zero-based element range
  logic        bits[0:1];   // 1-bit elements have no range
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    mem[0] = 8'hDE; mem[1] = 8'hAD; mem[2] = 8'hBE;
    hi[0] = 8'h3C; hi[1] = 8'h5A;
    bits[0] = 1'b1; bits[1] = 1'b0;
    #1 $finish;
  end
endmodule
"#,
    );
    for i in 0..3 {
        let l = var_line(&vcd, &format!("mem[{}]", i));
        assert!(l.ends_with(&format!(" mem[{}] [7:0] $end", i)), "{}", l);
    }
    assert!(var_line(&vcd, "hi[0]").ends_with(" hi[0] [15:8] $end"), "{}", vcd);
    // A scalar element has no range to declare.
    assert!(var_line(&vcd, "bits[0]").ends_with(" bits[0] $end"), "{}", vcd);
}

/// §21.7.2.1 value-change records: a reader LEFT-EXTENDS a value shorter than
/// the `$var` width with its leftmost character — `x` extends with x, `z` with
/// z, anything else with 0. a reference simulator therefore collapses a leading run of x (or of
/// z) to a single character, just as it collapses a leading run of zeros; xezim
/// spelled x/z runs out in full. The one case that may NOT collapse is a leading
/// run of ZEROS in front of an x/z: `8'b000000x1` → `b0x1`, never `bx1` (which
/// reads back as `8'bxxxxxxx1`).
#[test]
fn leading_run_suppression_matches_reference_for_every_vector_shape() {
    let vcd = dump(
        "leadrun",
        r#"
module tb;
  logic [7:0] v0, v1, v2, v3, v4, v5, v6, v7, v8;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    v0 = 8'b000000x1;   // 0-run in front of an x → keep one 0
    v1 = 8'bzzzz0011;   // leading z-run → one z
    v2 = 8'bxxxx0011;   // leading x-run → one x
    v3 = 8'b00001111;   // leading 0-run in front of a 1 → dropped
    v4 = 8'hFF;         // leading 1 → nothing may be dropped
    v5 = 8'bxxxxxxxx;   // all x
    v6 = 8'bzzzzzzzz;   // all z
    v7 = 8'b00000000;   // all 0
    v8 = 8'b1010zz11;   // no leading run at all
    #1 $finish;
  end
endmodule
"#,
    );
    // Each pair is exactly what `a reference simulator`/`vvp` writes for the same assignment.
    let expected = [
        ("v0", "b0x1"),
        ("v1", "bz0011"),
        ("v2", "bx0011"),
        ("v3", "b1111"),
        ("v4", "b11111111"),
        ("v5", "bx"),
        ("v6", "bz"),
        ("v7", "b0"),
        ("v8", "b1010zz11"),
    ];
    for (name, want) in expected {
        assert_eq!(
            records(&vcd, name).last().map(|s| s.as_str()),
            Some(want),
            "`{}` must dump as `{}`:\n{}",
            name,
            want,
            vcd
        );
    }
}

/// §21.7.2.1 `var_type` / §6.5: a variable cannot mix continuous and procedural
/// drivers, so an object driven ONLY by a continuous assign — or by an instance
/// output port, which inlining lowers to one — has no procedural driver and is a
/// net. a reference simulator types exactly those `wire`; xezim typed every `logic` `reg`, so
/// GTKWave coloured driven nets as registers.
#[test]
fn a_variable_with_only_a_continuous_driver_is_typed_wire_like_reference() {
    let vcd = dump(
        "wirekind",
        r#"
module sub(input logic [3:0] din, output logic [3:0] dout);
  always @(din) dout = din + 4'd1;   // procedural driver in the child
endmodule
module top;
  logic [3:0] src;      // procedural driver → reg
  logic [3:0] snk;      // driven only by u_sub's output port → wire
  logic [3:0] cpy;      // driven only by a continuous assign → wire
  assign cpy = src;
  sub u_sub(.din(src), .dout(snk));
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, top);
    src = 4'd7;
    #1 $finish;
  end
endmodule
"#,
    );
    assert!(var_line(&vcd, "src").starts_with("$var reg 4 "), "{}", var_line(&vcd, "src"));
    assert!(var_line(&vcd, "snk").starts_with("$var wire 4 "), "{}", var_line(&vcd, "snk"));
    assert!(var_line(&vcd, "cpy").starts_with("$var wire 4 "), "{}", var_line(&vcd, "cpy"));
    // The child's own port formals: an input port is fed by the port connect
    // (continuous → wire); `dout` is written by an always block (→ reg).
    let child: Vec<&str> = vcd
        .split("$scope module u_sub $end")
        .nth(1)
        .unwrap()
        .lines()
        .take_while(|l| !l.starts_with("$upscope"))
        .collect();
    assert!(
        child.iter().any(|l| l.starts_with("$var wire 4 ") && l.ends_with(" din [3:0] $end")),
        "{:?}",
        child
    );
    assert!(
        child.iter().any(|l| l.starts_with("$var reg 4 ") && l.ends_with(" dout [3:0] $end")),
        "{:?}",
        child
    );
}

/// §21.7 header: a real `$date`, and the timescale derived from the design's
/// actual precision rather than a hardcoded `1ns`.
#[test]
fn the_header_carries_a_date_and_the_designs_timescale() {
    let vcd = dump(
        "hdr",
        r#"
`timescale 1ns/1ps
module tb;
  logic a;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    a = 0;
    #1 $finish;
  end
endmodule
"#,
    );
    assert!(vcd.starts_with("$date\n"), "{}", vcd);
    let date = vcd.lines().nth(1).unwrap().trim();
    assert!(
        date.len() >= 10 && date.starts_with("20") && date.contains('-'),
        "$date must carry a date, got `{}`",
        date
    );
    assert!(vcd.contains("$timescale\n  1ps\n$end"), "{}", vcd);
}
