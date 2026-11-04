//! Opt-in relaxations for sources written against tools that accept a
//! construct the standard does not (feature requests #608, #622-#629,
//! reported by git-hevangel). Each testbench is an error by default and
//! prints ALL PASS with its switch; the environment variable does the same.
use std::process::Command;

fn run(source: &str, args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_opt_in_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("repro.sv");
    // `<NUL>` stands for a NUL byte, which a test source cannot carry.
    std::fs::write(&path, source.replace("<NUL>", "\0")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "t", "--no-cache"])
        .args(args)
        .arg(&path)
        .envs(env.iter().copied())
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (text.contains("ALL PASS") && !text.contains("FAIL"), text)
}

fn check(source: &str, args: &[&str]) {
    let (pass, text) = run(source, args, &[]);
    assert!(pass, "{args:?}:\n{text}");
    let (pass, text) = run(source, &[], &[]);
    assert!(!pass && text.contains("error"), "without {args:?}:\n{text}");
}

const CONFIG_KEYWORDS_AS_NAMES_608: &str = r#"// Code that was written before IEEE 1364-2001 used `instance`, `cell`, `use`,
// ... as ordinary names; those words became the configuration keywords in
// 1364-2001 (IEEE 1800-2017 Annex B, 22.14 Table 22-2) and a strict parser
// rejects every use. Some simulators accept them as names unless a
// configuration is in use. The file is ordinary SystemVerilog otherwise (it
// uses `class` and `logic`), so it needs the SystemVerilog keywords WITHOUT
// the ten configuration keywords: no `begin_keywords` version provides that
// (the 1364 versions drop `class` and `logic` too).
//
// Every line should print PASS. A strict parser stops at the first use of
// `instance` with: expected identifier, found KwInstance 'instance'.
class holder;
  int instance = 0;                     // class property
  function void set(int cell);          // argument
    int use = cell;                     // local variable
    instance = use;
  endfunction
endclass

class registry;
  holder instance;                      // a handle named like the keyword
  function new();
    instance = new();
  endfunction
endclass

module t;
  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  logic library = 1'b1;                 // module-scope variable
  wire  [3:0] design = 4'd9;            // net

  initial begin
    registry r = new();
    r.instance.set(3);                  // method call on a property named instance
    check("property named instance holds 3", r.instance.instance, 3);
    check("variable named library",          library,            1);
    check("net named design",                design,             9);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// IEEE 1800-2023 §22.14: `--no-config-keywords` reads `instance`, `cell`, `use` as names (#608).
#[test]
fn config_keywords_as_names_608() {
    check(CONFIG_KEYWORDS_AS_NAMES_608, &["--no-config-keywords"]);
}

const KEYWORDS_1364_1995_608: &str = r#"module t;
  reg generate, signed, automatic;
  integer logic;
  initial begin generate = 1; signed = 0; logic = 7; if (generate === 1 && signed === 0 && logic == 7) $display("ALL PASS"); else $display("FAIL"); end
endmodule
"#;

/// §22.14: `--keywords 1364-1995` frees the words 1364-2001 and SystemVerilog added (#608).
#[test]
fn keywords_1364_1995_608() {
    check(KEYWORDS_1364_1995_608, &["--keywords", "1364-1995"]);
}

const DUPLICATE_DECLS_622: &str = r#"// A name declared twice in the declaration section of a task or function.
//
// IEEE 1800-2023 3.13: within a name space it is illegal to redeclare a name
// already declared by a prior declaration; the block name space is introduced by
// blocks, functions and tasks. Some simulators accept the repeat anyway: they
// warn, ignore the SECOND declaration and keep the first one, with its type and
// its initializer. The expected values below are what the reference simulator
// prints for this file (it accepts every repeat with a warning).
//
// Every line should print PASS. A strict parser stops at the first repeat:
//   error: 'x' is already declared in this subroutine
module t;
  class box_a; int v = 1; endclass
  class box_b; int w = 2; endclass

  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  // the same type twice, initializers on both: the first one stays
  function automatic int both_init();
    int x = 5;
    int x = 7;
    return x;
  endfunction

  // no initializer on the first, one on the second: the second is ignored too
  function automatic int late_init();
    int x;
    int x = 7;
    return x;
  endfunction

  // a different type the second time: the first type stays (300 does not wrap to 12)
  function automatic int other_type();
    int x;
    bit [3:0] x;
    x = 300;
    return x;
  endfunction

  // one name repeated inside a comma list: only the repeated name is ignored
  function automatic int in_a_list();
    int a = 1, b = 2;
    int b = 20, c = 3;
    return a + 10 * b + 100 * c;       // 1 + 20 + 300
  endfunction

  // a handle declared twice with two class types: the first type stays
  function automatic int handle_type();
    box_a h;
    box_b h;
    h = new();
    return h.v;
  endfunction

  // a task
  task automatic in_a_task(output int r);
    int k = 3;
    int k = 4;
    r = k;
  endtask

  // a class method
  class holder;
    function int m();
      int q = 8;
      int q = 9;
      return q;
    endfunction
  endclass

  // a begin ... end block
  function automatic int in_a_block();
    int r;
    begin
      int y = 5;
      int y = 7;
      r = y;
    end
    return r;
  endfunction

  initial begin
    int r;
    holder h = new();
    check("same type, both initialized: the first stays", both_init(),   5);
    check("second initializer is ignored",                late_init(),   0);
    check("second type is ignored",                       other_type(),  300);
    check("repeated name in a list",                      in_a_list(),   321);
    check("second handle type is ignored",                handle_type(), 1);
    in_a_task(r);
    check("task",                                         r,             3);
    check("class method",                                 h.m(),         8);
    check("begin-end block",                              in_a_block(),  5);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// §3.13: a repeated declaration is ignored, the first kept (#622).
#[test]
fn duplicate_decls_622() {
    check(DUPLICATE_DECLS_622, &["--relax-duplicate-decls"]);
}

const ENUM_NO_BASE_623: &str = r#"// IEEE 1800-2023 6.19, Syntax 6-5: enum_base_type is an integer atom type, an integer
// vector type with an optional packed dimension, or a type name. A packed dimension
// straight after `enum` with NO base type is not in the grammar, so a strict parser
// rejects `typedef enum [1:0] {...} t;`.
//
// Some simulators accept the shape. The reference simulator warns that the base type is
// not specified and that it uses `bit`: the enum is a 2-state `bit [1:0]`. Every line below
// should print PASS. With a strict parser the file does not compile:
//   error: expected identifier, found LBracket '['
typedef enum [1:0] {RESP_OK = 0, RESP_EX = 1, RESP_ERR = 2, RESP_DEC = 3} resp_e;

package kinds_pkg;
  typedef enum [3:0] {KIND_LOW = 4'h1, KIND_HIGH = 4'h8} kind_e;
endpackage

module t;
  import kinds_pkg::*;

  int fails = 0;
  resp_e r;                                   // declared, never assigned
  kind_e k;
  enum [1:0] {ST_IDLE, ST_BUSY, ST_DONE} st;  // anonymous enum, same shape

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  initial begin
    // The written range is the width of the type.
    check("$bits of a [1:0] enum is 2",        $bits(resp_e) == 2);
    check("$bits of a [3:0] enum in a package is 4", $bits(kind_e) == 4);
    check("$bits of an anonymous [1:0] enum is 2", $bits(st) == 2);

    // `bit` base type: a variable starts at 0, not x.
    check("variable starts at 0 (2-state)",    r === RESP_OK);
    check("anonymous variable starts at its first label", st === ST_IDLE);

    // `bit` base type: x and z bits cast to 0.
    check("cast of 2'bxx is RESP_OK",          resp_e'(2'bxx) === RESP_OK);

    // Values and methods behave as for `enum bit [1:0]`.
    r = RESP_ERR;
    check("assigned label compares equal",     r == RESP_ERR);
    check("integer value of a label",          int'(RESP_DEC) == 3);
    r = RESP_DEC;
    r = r.next();
    check("next() of the last label wraps",    r == RESP_OK);
    r = r.prev();
    check("prev() of the first label wraps",   r == RESP_DEC);
    check("num() counts the labels",           r.num() == 4);
    k = KIND_HIGH;
    check("package enum label value",          k == 4'h8);
    check("package enum name()",               k.name() == "KIND_HIGH");

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// §6.19: `enum [N:0] {...}` reads as `enum bit [N:0]` (#623).
#[test]
fn enum_no_base_623() {
    check(ENUM_NO_BASE_623, &["--relax-enum-no-base"]);
}

const STATIC_CONST_624: &str = r#"// `const` written AFTER the lifetime in a procedural declaration.
//
// IEEE 1800-2023 6.20.6 and A.2.1.3: data_declaration ::= [ const ] [ var ]
// [ lifetime ] data_type_or_implicit list_of_variable_decl_assignments ;
// so `const static int k = 3;` is a declaration, while `static const int k = 3;`
// is not (inside a class, `static` is a property qualifier and `static const`
// IS legal; this file does not use that). The reference simulator accepts
// `static const` and `automatic const` in a function, task or block and reads
// them as `const static` / `const automatic`; the values below are what it prints.
//
// Every line should print PASS. A strict parser stops at the first one:
//   error: expected identifier, found KwConst 'const'
module t;
  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  task automatic check_s(string label, string got, string expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %s, expected %s", label, got, expected); fails++; end
  endtask

  // a static constant array
  function int lookup(int i);
    static const int tbl[4] = '{10, 20, 30, 40};
    return tbl[i];
  endfunction

  // a static constant string array
  function string name_of(int i);
    static const string names[3] = '{"aa", "bb", "cc"};
    return names[i];
  endfunction

  // a static constant that uses another one
  function int unsigned prefix_len();
    static const string       prefix = "abc_";
    static const int unsigned plen   = prefix.len();
    return plen;
  endfunction

  // an automatic constant is initialized at each call
  function automatic int twice(int v);
    automatic const int k = v * 2;
    return k;
  endfunction

  // a task
  task automatic get(output int r);
    static const int k = 42;
    r = k;
  endtask

  // the standard's own order, as a control
  function int standard_order();
    const static int k = 5;
    return k;
  endfunction

  initial begin
    int r;
    begin
      static const int blk = 9;                 // in a begin ... end block
      check("static const in a block",       blk,           9);
    end
    check("static const array",              lookup(2),     30);
    check_s("static const string array",     name_of(1),    "bb");
    check("static const from static const",  prefix_len(),  4);
    check("automatic const, first call",     twice(3),      6);
    check("automatic const, second call",    twice(5),      10);
    get(r);
    check("static const in a task",          r,             42);
    check("const static (standard order)",   standard_order(), 5);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// §6.20.6: `static const` reads as `const static` (#624).
#[test]
fn static_const_624() {
    check(STATIC_CONST_624, &["--relax-static-const"]);
}

const EVENT_THIS_625: &str = r#"// IEEE 1800-2017 9.4.2: event_control ::= @ hierarchical_event_identifier
//                                       | @ ( event_expression ) | ...
// A hierarchical identifier cannot start with the keyword `this`, so the
// standard spelling of "wait for my own event" is @(this.ev). Some simulators
// also take the bare form @this.ev; and wait on the same event. This file uses
// the bare form for a member event and for an event of a member object, next to
// the standard form as a control. Events are triggered at known times.
//
// Every line should print PASS. A strict parser stops at the first @this.
class leaf;
  event ev;
endclass

class waiter;
  event ready;
  leaf  sub;
  time  woke_ready = 0, woke_sub = 0, woke_std = 0;
  bit   got_ready = 0, got_sub = 0, got_std = 0;
  int   after = 0;

  function new();
    sub = new();
  endfunction

  task wait_ready();          // bare form, one name
    @this.ready;
    woke_ready = $time;
    got_ready = 1;
  endtask

  task wait_sub();            // bare form, a path through a member object
    @this.sub.ev;
    woke_sub = $time;
    got_sub = 1;
  endtask

  task wait_std();            // the standard form, the control
    @(this.ready);
    woke_std = $time;
    got_std = 1;
  endtask

  task wait_then_assign();    // the control is followed by a statement
    @this.ready after = 7;
  endtask
endclass

module t;
  waiter w = new();
  int fails = 0;

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  initial begin
    fork
      w.wait_ready();
      w.wait_sub();
      w.wait_std();
      w.wait_then_assign();
    join_none
    #9;
    check("nothing woke before the first trigger", !w.got_ready && !w.got_sub && !w.got_std && w.after == 0);
    #1 ->w.ready;                         // time 10
    #5 ->w.sub.ev;                        // time 15
    #5;
    check("@this.ready woke at 10",       w.got_ready && w.woke_ready == 10);
    check("@this.sub.ev woke at 15",      w.got_sub   && w.woke_sub   == 15);
    check("@(this.ready) woke at 10",     w.got_std   && w.woke_std   == 10);
    check("the statement after the control ran", w.after == 7);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// §9.4.2: `@this.ev` reads as `@(this.ev)` (#625).
#[test]
fn event_this_625() {
    check(EVENT_THIS_625, &["--relax-event-this"]);
}

const CASEX_INSIDE_626: &str = r#"// IEEE 1800-2023 12.5.4 and Annex A.6.7: the set-membership form of the case statement is
//     case ( case_expression ) inside case_inside_item { case_inside_item } endcase
// where `inside` follows the keyword `case` only. `casex (v) inside` and `casez (v) inside`
// are not in the grammar, so a strict parser rejects them:
//     error: expected expression, found KwInside 'inside'
//
// Some simulators accept them. The reference simulator reads `casex (v) inside` and
// `casez (v) inside` as `case (v) inside`: only an item's own x, z and ? bits are wildcards
// (a wildcard match, ==?), an x or z bit in the case expression is NOT a wildcard, and a range
// item is a relation (an x in the value gives no match). The keyword casex/casez adds nothing.
// The plain casex/casez functions below keep their usual meaning (x/z in the expression IS a
// wildcard); they show that the `inside` forms are not those.
//
// Every line should print PASS.
module t;
  int fails = 0;

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  // Same items in all three `inside` functions.
  function automatic int f_case_inside(logic [7:0] v);
    case (v) inside
      8'h00, 8'h31:   return 1;     // a list
      8'h4x:          return 2;     // x in the item is a wildcard
      8'h5?:          return 3;     // ? in the item is a wildcard
      [8'h10:8'h1f]:  return 4;     // a range
      default:        return 0;
    endcase
  endfunction

  function automatic int f_casex_inside(logic [7:0] v);
    casex (v) inside
      8'h00, 8'h31:   return 1;
      8'h4x:          return 2;
      8'h5?:          return 3;
      [8'h10:8'h1f]:  return 4;
      default:        return 0;
    endcase
  endfunction

  function automatic int f_casez_inside(logic [7:0] v);
    casez (v) inside
      8'h00, 8'h31:   return 1;
      8'h4x:          return 2;
      8'h5?:          return 3;
      [8'h10:8'h1f]:  return 4;
      default:        return 0;
    endcase
  endfunction

  // Plain casex / casez (no inside, no range items): an x or z in v is a wildcard.
  function automatic int f_casex_plain(logic [7:0] v);
    casex (v)
      8'h00, 8'h31:   return 1;
      8'h4x:          return 2;
      8'h5?:          return 3;
      default:        return 0;
    endcase
  endfunction

  function automatic int f_casez_plain(logic [7:0] v);
    casez (v)
      8'h00, 8'h31:   return 1;
      8'h4x:          return 2;
      8'h5?:          return 3;
      default:        return 0;
    endcase
  endfunction

  // value, expected result of all three inside forms
  localparam int N = 10;
  logic [7:0] vals [N] = '{8'h00, 8'h31, 8'h4a, 8'h5c, 8'h17,
                           8'b0100xxxx, 8'bxxxxxxxx, 8'bzzzzzzzz, 8'b0110xxxx, 8'hff};
  int         want [N] = '{1, 1, 2, 3, 4, 2, 0, 0, 0, 0};

  initial begin
    for (int i = 0; i < N; i++) begin
      check($sformatf("case   inside v=%b -> %0d", vals[i], want[i]), f_case_inside(vals[i])   == want[i]);
      check($sformatf("casex  inside v=%b -> %0d", vals[i], want[i]), f_casex_inside(vals[i])  == want[i]);
      check($sformatf("casez  inside v=%b -> %0d", vals[i], want[i]), f_casez_inside(vals[i])  == want[i]);
    end
    // z in the case expression is no wildcard either
    check("casez inside v=0100zzzz -> 2", f_casez_inside(8'b0100zzzz) == 2);
    // not the plain statements: an all-x value is a wildcard there and takes the first item
    check("plain casex v=xxxxxxxx takes the first item", f_casex_plain(8'bxxxxxxxx) == 1);
    check("plain casez v=zzzzzzzz takes the first item", f_casez_plain(8'bzzzzzzzz) == 1);
    check("casex inside v=xxxxxxxx is not that",         f_casex_inside(8'bxxxxxxxx) == 0);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// §12.5.4: `casex (e) inside` reads as `case (e) inside` (#626).
#[test]
fn casex_inside_626() {
    check(CASEX_INSIDE_626, &["--relax-casex-inside"]);
}

const MACRO_SPACE_627: &str = r#"// IEEE 1800-2017 22.5.1 (Syntax 22-3): a macro usage or a compiler directive is a
// backtick written DIRECTLY in front of its name. Some simulators also accept
// spaces or tabs between the backtick and the name and read `` ` NAME `` as `NAME.
// Every line should print PASS. A strict parser stops at the first spaced directive
// (line 10) with: expected Semicolon, found IntegerLiteral '100'
`define LIMIT 5
`define PLUS(a, b) ((a) + (b))
`define SETV(dst, v) dst = (v);
`define LATE ` LIMIT                // a spaced usage inside a macro body
` define EXTRA 100                   // a spaced directive
` ifdef EXTRA                       // spaced conditional directives
`define HAVE_EXTRA 1
` else
`define HAVE_EXTRA 0
` endif
module t;
  int fails = 0;
  int a, b, c, d, e, f, g;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  initial begin
    a = ` LIMIT;                    // object-like usage, one space
    b = `	LIMIT;                  // object-like usage, a tab after the backtick
    c = ` PLUS(1, 2);               // function-like usage
    d = `PLUS(` LIMIT , 10);        // a spaced usage as an actual argument
    `SETV(e, ` LIMIT)               // the last actual argument, spaced
    f = `LATE;                      // a macro whose body has a spaced usage
    g = `HAVE_EXTRA + ` EXTRA;      // the conditional above took the first branch
    check("one space",               a, 5);
    check("tab",                     b, 5);
    check("function-like",           c, 3);
    check("actual argument",         d, 15);
    check("last actual argument",    e, 5);
    check("macro body",              f, 5);
    check("directives and a usage",  g, 101);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// §22.5.1: `` ` NAME `` reads as `` `NAME `` (#627).
#[test]
fn macro_space_627() {
    check(MACRO_SPACE_627, &["--relax-macro-space"]);
}

const NUL_BYTES_628: &str = r#"// IEEE 1800-2017 5.3: white space is blanks, tabs, newlines, formfeeds and the end
// of the file. A NUL byte (0x00) is none of those, so a strict parser rejects it.
// Some simulators take a NUL as a separator between tokens, like a space.
//
// This file is a template: gen_repro.sh writes each NUL marker below (the word NUL
// in angle brackets) as one NUL byte, because an issue text cannot carry one.
// Every line of the generated repro.sv should print PASS. A strict parser stops
// at the first NUL with: expected Semicolon, found Unknown ' '
module t;
  int fails = 0;
  int a<NUL>;                        // between a name and its semicolon
  int<NUL> b;                        // between two tokens
  int c<NUL>= 3;                     // before an operator
  logic [3:0] v <NUL> ;              // between two spaces
  int d = 4;  // a NUL<NUL>inside a comment changes nothing

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  initial begin
    a = 1; b = 2; v = 4'd9;
    check("NUL before the semicolon", a, 1);
    check("NUL between two tokens",   b, 2);
    check("NUL before an operator",   c, 3);
    check("NUL between spaces",        v, 9);
    check("NUL in a comment",         d, 4);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
<NUL>
"#;

/// §5.3: a NUL byte reads as white space (#628).
#[test]
fn nul_bytes_628() {
    check(NUL_BYTES_628, &["--relax-nul-bytes"]);
}

const CROSS_HIER_ITEM_629: &str = r#"// IEEE 1800-2017 19.6: cross_item ::= cover_point_identifier | variable_identifier
// A cross item is a coverpoint label or a plain variable (which then gets an
// implicit coverpoint). Some simulators also take a hierarchical reference, here
// a member of a virtual interface, and make the same implicit coverpoint over it:
//
//     X_mode : cross vif.mode, cp_ena, cp_par;
//
// cg uses that form. cg_plain is the control and uses the standard form: an
// explicit coverpoint on the same reference, crossed by its label. The two must
// report the same coverage, for the cross and for the group, because the implicit
// coverpoint counts as a coverpoint of the group.
//
// Five samples hit four distinct (mode, ena, par) tuples out of 4 x 2 x 2 = 16
// cross bins, so the cross is 25.00; the three coverpoints are fully covered, so
// the group is (100 + 100 + 100 + 25) / 4 = 81.25 (the implicit coverpoint is the
// fourth member of the group).
//
// The expected values were measured on the reference simulator and follow 19.6:
// "When a variable V is part of a cross coverage, SystemVerilog implicitly creates
// a coverage point for the variable, as if it had been created by the statement
// coverpoint V;". A second simulator gives the same cross coverage but leaves the
// implicit coverpoint out of the group figure, so it prints FAIL on the group lines.
//
// Every line should print PASS. A strict parser stops at the cross item.
interface bus_if;
  logic [1:0] mode;
endinterface

class mon;
  virtual bus_if vif;
  bit ena;
  bit par;

  covergroup cg;
    option.per_instance = 1;
    cp_ena : coverpoint ena;
    cp_par : coverpoint par;
    X_mode : cross vif.mode, cp_ena, cp_par;
  endgroup

  covergroup cg_plain;
    option.per_instance = 1;
    cp_ena  : coverpoint ena;
    cp_par  : coverpoint par;
    cp_mode : coverpoint vif.mode;
    X_mode  : cross cp_mode, cp_ena, cp_par;
  endgroup

  function new(virtual bus_if v);
    vif = v;
    cg = new();
    cg_plain = new();
  endfunction

  function void hit(logic [1:0] m, bit e, bit p);
    vif.mode = m; ena = e; par = p;
    cg.sample();
    cg_plain.sample();
  endfunction
endclass

module t;
  bus_if bus();
  mon m;
  int fails = 0;

  function automatic bit near(real a, real b);
    return (a > b ? a - b : b - a) < 0.01;
  endfunction

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  initial begin
    m = new(bus);
    m.hit(0, 0, 0);
    m.hit(1, 1, 0);
    m.hit(2, 0, 1);
    m.hit(3, 1, 1);
    m.hit(3, 1, 1);
    $display("cg: group %0.2f cross %0.2f   control: group %0.2f cross %0.2f",
             m.cg.get_inst_coverage(), m.cg.X_mode.get_inst_coverage(),
             m.cg_plain.get_inst_coverage(), m.cg_plain.X_mode.get_inst_coverage());
    check("cross over vif.mode is 25.00",       near(m.cg.X_mode.get_inst_coverage(), 25.0));
    check("group is 81.25",                     near(m.cg.get_inst_coverage(), 81.25));
    check("cross equals the explicit form",     near(m.cg.X_mode.get_inst_coverage(), m.cg_plain.X_mode.get_inst_coverage()));
    check("group equals the explicit form",     near(m.cg.get_inst_coverage(), m.cg_plain.get_inst_coverage()));
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

/// §19.6: a hierarchical cross item gets an implicit coverpoint (#629).
#[test]
fn cross_hier_item_629() {
    check(CROSS_HIER_ITEM_629, &["--relax-cross-hier-item"]);
}

/// The environment variable is the same switch, and a run without it is
/// not served the relaxed run's cached design.
#[test]
fn environment_switch_and_cache_key() {
    let (pass, text) = run(
        DUPLICATE_DECLS_622,
        &[],
        &[("XEZIM_RELAX_DUPLICATE_DECLS", "1")],
    );
    assert!(pass, "{text}");
    let (pass, text) = run(ENUM_NO_BASE_623, &[], &[("XEZIM_KEYWORDS", "1800-2017")]);
    assert!(!pass, "{text}");
}

/// An unknown keyword set is an error naming the valid ones.
#[test]
fn unknown_keyword_set() {
    let (_, text) = run(KEYWORDS_1364_1995_608, &["--keywords", "1999"], &[]);
    assert!(
        text.contains("unknown keyword set '1999'") && text.contains("1364-1995"),
        "{text}"
    );
}
