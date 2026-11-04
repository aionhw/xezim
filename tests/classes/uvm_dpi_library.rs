//! UVM compiled WITHOUT `UVM_NO_DPI`: the DPI-C helpers of UVM's `src/dpi`
//! (regex, command line, HDL backdoor) are served by xezim's built-in
//! library (`src/compiler/simulator/uvm_dpi.rs`).
//!
//! The expected values were produced by the reference simulator running the
//! same benches with its bundled UVM 1.2 DPI library (`-sv_lib`, and full
//! design visibility for the backdoor). Where that library departs from
//! UVM's own C sources the assertions follow the C sources and say so:
//! regex errors are reported as `UVM/DPI/REGEX_INV` (the bundled library
//! prints its own error line instead), the length limit is UVM 1.2's 2048
//! characters (the bundled library stops at 2040), and a path the backdoor
//! cannot find is a `UVM/DPI/HDL_GET`/`HDL_SET` error (the bundled library
//! prints a simulator error instead).

use xezim::*;

fn run_uvm_dpi(version: &str, src: &str, plusargs: &[&str]) -> compiler::Simulator {
    let src_dir = crate::uvm_integration_tests::uvm_dir()
        .join(version)
        .join("src");
    let uvm_pkg = std::fs::read_to_string(src_dir.join("uvm_pkg.sv"))
        .unwrap_or_else(|e| panic!("read {}/src/uvm_pkg.sv: {}", version, e));
    let plusargs: Vec<String> = plusargs.iter().map(|s| s.to_string()).collect();
    simulate_multi(
        &[uvm_pkg, src.to_string()],
        10_000,
        Some("top"),
        &[src_dir.to_str().unwrap().to_string()],
        &[],
        None,
        false,
        None,
        None,
        &[("UVM_REPORT_DISABLE_FILE_LINE".to_string(), None)],
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
    .unwrap_or_else(|e| panic!("UVM {} bench failed to simulate: {}", version, e))
}

fn lines(sim: &compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .flat_map(|o| o.message.lines().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

fn assert_lines(out: &[String], expected: &[&str]) {
    let got: Vec<&str> = out
        .iter()
        .filter(|l| l.starts_with("T|"))
        .map(String::as_str)
        .collect();
    assert_eq!(got, expected, "full output:\n{}", out.join("\n"));
}

fn count_id(out: &[String], severity: &str, id: &str) -> usize {
    let tag = format!("[{}]", id);
    out.iter()
        .filter(|l| l.starts_with(&format!("{} @", severity)) && l.contains(&tag))
        .count()
}

/// config_db globs and a `/regex/` scope, a regex and a glob resource
/// scope, a wildcard instance override by name, the `+uvm_set_config_int`
/// and `+uvm_set_inst_override` plusargs, and the command-line processor
/// (regex and prefix matches, values) — all through uvm_glob_to_re /
/// uvm_re_match / uvm_dpi_regcomp and the argv walk.
const REGEX_BENCH: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class base_c extends uvm_component;
  `uvm_component_utils(base_c)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  virtual function string kind(); return "base"; endfunction
endclass
class deriv_c extends base_c;
  `uvm_component_utils(deriv_c)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  virtual function string kind(); return "deriv"; endfunction
endclass
class deriv2_c extends base_c;
  `uvm_component_utils(deriv2_c)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  virtual function string kind(); return "deriv2"; endfunction
endclass

class leaf extends uvm_component;
  `uvm_component_utils(leaf)
  int knob;
  uvm_bitstream_t knob2;
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    if (!uvm_config_db#(int)::get(this, "", "knob", knob)) knob = -1;
    if (!uvm_config_int::get(this, "", "knob2", knob2)) knob2 = -1;
  endfunction
endclass

class dpi_test extends uvm_test;
  `uvm_component_utils(dpi_test)
  leaf l1, l2, x1, r7, r77;
  base_c b1, b2, t1;
  function new(string name = "dpi_test", uvm_component parent = null); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    uvm_config_db#(int)::set(this, "l*", "knob", 5);
    uvm_config_db#(int)::set(this, "x?", "knob", 9);
    uvm_config_db#(int)::set(null, "/^uvm_test_top\\.r[0-9]$/", "knob", 7);
    uvm_factory::get().set_inst_override_by_name("base_c", "deriv_c", "uvm_test_top.b2*");
    l1 = leaf::type_id::create("l1", this);
    l2 = leaf::type_id::create("l2", this);
    x1 = leaf::type_id::create("x1", this);
    r7 = leaf::type_id::create("r7", this);
    r77 = leaf::type_id::create("r77", this);
    b1 = base_c::type_id::create("b1", this);
    b2 = base_c::type_id::create("b2", this);
    t1 = base_c::type_id::create("t1", this);
    uvm_resource_db#(int)::set("/^top\\.a[0-9]+$/", "rkey", 33);
    uvm_resource_db#(int)::set("top.b*", "gkey", 44);
  endfunction
  task run_phase(uvm_phase phase);
    uvm_cmdline_processor clp;
    string args[$], m[$], vals[$], v;
    int n, r;
    phase.raise_objection(this);
    $display("T|knob l1=%0d l2=%0d x1=%0d r7=%0d r77=%0d", l1.knob, l2.knob, x1.knob, r7.knob, r77.knob);
    $display("T|knob2 l1=%0d l2=%0d x1=%0d", int'(l1.knob2), int'(l2.knob2), int'(x1.knob2));
    $display("T|kind b1=%s b2=%s t1=%s", b1.kind(), b2.kind(), t1.kind());
    n = uvm_resource_db#(int)::read_by_name("top.a12", "rkey", r) ? r : 0;
    $display("T|res a12=%0d ab=%0d", n, uvm_resource_db#(int)::read_by_name("top.ab", "rkey", r));
    n = uvm_resource_db#(int)::read_by_name("top.bx", "gkey", r) ? r : 0;
    $display("T|res bx=%0d cx=%0d", n, uvm_resource_db#(int)::read_by_name("top.cx", "gkey", r));
    clp = uvm_cmdline_processor::get_inst();
    void'(clp.get_arg_matches("/^\\+fo+/", m));
    $display("T|matches re %0d %s", m.size(), m.size() == 2 ? {m[0], " ", m[1]} : "");
    void'(clp.get_arg_matches("+ba", m));
    $display("T|matches prefix %0d %s", m.size(), m.size() == 1 ? m[0] : "");
    n = clp.get_arg_value("+foo=", v);
    $display("T|value n=%0d v=%s", n, v);
    n = clp.get_arg_values("+uvm_set_config_int=", vals);
    $display("T|values n=%0d v0=%s", n, n ? vals[0] : "");
    clp.get_plusargs(args);
    $display("T|plusargs %0d", args.size());
    $display("T|re %0d %0d %0d %0d", uvm_re_match(uvm_glob_to_re("uvm_test_top.*"), "uvm_test_top.l1"),
             uvm_re_match("/(ab|cd)+x/", "zcdabx"), uvm_re_match("/^[[:digit:]]+$/", "12x"),
             uvm_re_match("", "anything"));
    $display("T|glob '%s' '%s' '%s'", uvm_glob_to_re("a?[0].*"), uvm_glob_to_re("/x+/"), uvm_glob_to_re(""));
    clp.get_args(args);
    $display("T|argv0 %s tool %s", args[0], clp.get_tool_name());
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial run_test();
endmodule
"#;

pub(super) fn uvm_1_2_dpi_regex_config_factory_cmdline() {
    let sim = run_uvm_dpi(
        "1.2",
        REGEX_BENCH,
        &[
            "+UVM_TESTNAME=dpi_test",
            "+foo=1",
            "+foobar",
            "+bar",
            "+uvm_set_config_int=uvm_test_top.l*,knob2,11",
            "+uvm_set_inst_override=base_c,deriv2_c,uvm_test_top.t*",
        ],
    );
    let out = lines(&sim);
    // Reference-verified, except argv0 and the tool name, which name the
    // simulator itself.
    assert_lines(
        &out,
        &[
            "T|knob l1=5 l2=5 x1=9 r7=7 r77=-1",
            "T|knob2 l1=11 l2=11 x1=-1",
            "T|kind b1=base b2=deriv t1=deriv2",
            "T|res a12=33 ab=0",
            "T|res bx=44 cx=0",
            "T|matches re 2 +foo=1 +foobar",
            "T|matches prefix 1 +bar",
            "T|value n=1 v=1",
            "T|values n=1 v0=uvm_test_top.l*,knob2,11",
            "T|plusargs 6",
            "T|re 0 0 1 0",
            "T|glob '/^a.\\[0\\]\\..*$/' '/x+/' ''",
            "T|argv0 xezim tool xezim",
        ],
    );
    assert!(
        !out.iter().any(|l| l.contains("[DPI]")
            || l.starts_with("UVM_ERROR @")
            || l.starts_with("UVM_WARNING @")),
        "no DPI diagnostics or UVM errors expected:\n{}",
        out.join("\n")
    );
}

/// The C code's diagnostics: uvm_dump_re_cache's notice, an invalid regex
/// (REGEX_INV), an over-long one (REGEX_MAX), and a pattern regcomp
/// rejects on the command-line path (REGCOMP, then the processor's own
/// error). Reported through UVM, so they count as UVM messages.
const REGEX_ERRORS_BENCH: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"
module top;
  initial begin
    string long_re, m[$];
    int r;
    chandle h;
    uvm_cmdline_processor clp;
    uvm_dump_re_cache();
    r = uvm_re_match("/a(b/", "ab");
    $display("T|bad_re nonzero=%0d", r != 0);
    long_re = "a";
    repeat (12) long_re = {long_re, long_re};
    r = uvm_re_match(long_re, "a");
    $display("T|long_re=%0d len=%0d", r, long_re.len());
    h = uvm_dpi_regcomp("a(");
    $display("T|regcomp_bad_null=%0d", h == null);
    clp = uvm_cmdline_processor::get_inst();
    void'(clp.get_arg_matches("/+(/", m));
    $display("T|matches_bad=%0d", m.size());
  end
endmodule
"#;

pub(super) fn uvm_1_2_dpi_regex_diagnostics() {
    let sim = run_uvm_dpi("1.2", REGEX_ERRORS_BENCH, &[]);
    let out = lines(&sim);
    assert_lines(
        &out,
        &[
            "T|bad_re nonzero=1",
            "T|long_re=1 len=4096",
            "T|regcomp_bad_null=1",
            "T|matches_bad=0",
        ],
    );
    assert!(
        out.iter().any(|l| l
            == "UVM_INFO @ 0: reporter [UVM/DPI/REGEX_MAX] uvm_dump_re_cache: cache not implemented"),
        "uvm_dump_re_cache notice:\n{}",
        out.join("\n")
    );
    assert!(
        out.iter().any(|l| l.starts_with(
            "UVM_ERROR @ 0: reporter [UVM/DPI/REGEX_INV] uvm_re_match : invalid glob or regular expression: |/a(b/||"
        )),
        "invalid regex report:\n{}",
        out.join("\n")
    );
    assert_eq!(count_id(&out, "UVM_ERROR", "UVM/DPI/REGEX_MAX"), 1);
    assert!(
        out.iter().any(|l| l
            == "UVM_ERROR @ 0: reporter [UVM/DPI/REGCOMP] uvm_dpi_regcomp : Unable to compile regex: |a(|, Element 0 is: a"),
        "regcomp report:\n{}",
        out.join("\n")
    );
    assert_eq!(count_id(&out, "UVM_ERROR", "UVM/DPI/REGCOMP"), 2);
    assert_eq!(count_id(&out, "UVM_ERROR", "UVM_CMDLINE_PROC"), 1);
}

/// 1800.2 changed uvm_glob_to_re's answer for an empty glob from "" (a
/// regex matching everything) to "/^$/"; the builtin follows the release
/// it is compiled with.
pub(super) fn uvm_1800_2_2017_dpi_empty_glob_and_wildcard_config() {
    const SRC: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"
class leaf extends uvm_component;
  `uvm_component_utils(leaf)
  int knob = -1;
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    void'(uvm_config_db#(int)::get(this, "", "knob", knob));
  endfunction
endclass
class t2017 extends uvm_test;
  `uvm_component_utils(t2017)
  leaf a1, b1;
  function new(string name = "t2017", uvm_component parent = null); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    uvm_config_db#(int)::set(this, "a*", "knob", 3);
    a1 = leaf::type_id::create("a1", this);
    b1 = leaf::type_id::create("b1", this);
  endfunction
  task run_phase(uvm_phase phase);
    phase.raise_objection(this);
    $display("T|glob '%s' %0d %0d", uvm_glob_to_re(""), uvm_re_match(uvm_glob_to_re(""), ""),
             uvm_re_match(uvm_glob_to_re(""), "x"));
    $display("T|knob a1=%0d b1=%0d", a1.knob, b1.knob);
    phase.drop_objection(this);
  endtask
endclass
module top;
  initial run_test("t2017");
endmodule
"#;
    let sim = run_uvm_dpi("1800.2-2017", SRC, &[]);
    let out = lines(&sim);
    assert_lines(&out, &["T|glob '/^$/' 0 1", "T|knob a1=3 b1=-1"]);
}

/// Backdoor access by full hierarchical path: whole signals, bit- and
/// part-selects in the declared direction (descending with an offset,
/// ascending), a packed-struct member, a memory word and a slice of it, a
/// wide and a signed signal; deposits through the same forms; a force and
/// release_and_read on a continuously assigned net (the release shows the
/// driven value at once); a force and release on a flop's variable. Missing
/// paths, a reversed part-select, an out-of-range select and a module scope
/// fail with the C code's HDL_GET/HDL_SET errors.
const HDL_BENCH: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

typedef struct packed { logic [3:0] hi; logic [7:0] mid; logic [3:0] lo; } pk_t;

module sub;
  logic [11:4] off;
  logic [0:7] asc;
  pk_t st;
  logic [7:0] mem [0:15];
  logic [99:0] wide;
  logic signed [7:0] sg;
  logic [7:0] cnt;
  logic tick;
  wire [7:0] wd;
  assign wd = off ^ 8'h0f;
  always @(posedge tick) cnt <= cnt + 1;
  initial begin
    off = 8'hA5; asc = 8'h81; st = 16'hBEEF;
    foreach (mem[i]) mem[i] = i * 3;
    wide = {4'h9, 96'h0123456789abcdef01234567};
    sg = -8'sd3; cnt = 0; tick = 0;
  end
endmodule

module top;
  sub s();
  initial begin
    uvm_hdl_data_t d;
    int ok;
    #1;
    $display("T|check %0d%0d%0d%0d%0d%0d%0d%0d%0d",
      uvm_hdl_check_path("top.s"), uvm_hdl_check_path("top.s.off"),
      uvm_hdl_check_path("$root.top.s.off"), uvm_hdl_check_path("s.off"),
      uvm_hdl_check_path("top.s.nope"), uvm_hdl_check_path("top.s.off[7:4]"),
      uvm_hdl_check_path("top.s.off[3]"), uvm_hdl_check_path("top.s.mem[5]"),
      uvm_hdl_check_path("top.s.st.mid"));
    ok = uvm_hdl_read("top.s.off", d);         $display("T|off %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.off[7:4]", d);    $display("T|off[7:4] %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.off[4]", d);      $display("T|off[4] %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.asc[0:3]", d);    $display("T|asc[0:3] %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.asc[7]", d);      $display("T|asc[7] %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.st.mid", d);      $display("T|st.mid %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.mem[5]", d);      $display("T|mem[5] %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.mem[5][3:0]", d); $display("T|mem[5][3:0] %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.wide", d);        $display("T|wide %0d %h", ok, d[127:0]);
    ok = uvm_hdl_read("top.s.sg", d);          $display("T|sg %0d %h", ok, d[15:0]);
    ok = uvm_hdl_read("top.s.wd", d);          $display("T|wd %0d %h", ok, d[15:0]);
    ok = uvm_hdl_deposit("top.s.off[11:8]", 4'h3);  #1; $display("T|dep off %0d %h %h", ok, top.s.off, top.s.wd);
    ok = uvm_hdl_deposit("top.s.asc[6:7]", 2'b10);  #1; $display("T|dep asc %0d %b", ok, top.s.asc);
    ok = uvm_hdl_deposit("top.s.mem[2]", 8'hee);    #1; $display("T|dep mem %0d %h", ok, top.s.mem[2]);
    ok = uvm_hdl_deposit("top.s.st.lo", 4'h1);      #1; $display("T|dep st %0d %h", ok, top.s.st);
    ok = uvm_hdl_deposit("top.s.sg", -2);           #1; $display("T|dep sg %0d %0d", ok, top.s.sg);
    ok = uvm_hdl_deposit("top.s.wide", {100{1'b1}}); #1; $display("T|dep wide %0d %h", ok, top.s.wide);
    ok = uvm_hdl_deposit("top.s.tick", 1);          #1; $display("T|dep tick %0d %0d", ok, top.s.cnt);
    ok = uvm_hdl_force("top.s.wd", 8'h77);          #1; $display("T|force wd %0d %h", ok, top.s.wd);
    top.s.off = 8'h00;                              #1; $display("T|forced wd %h", top.s.wd);
    ok = uvm_hdl_release_and_read("top.s.wd", d);       $display("T|release wd %0d %h %h", ok, d[7:0], top.s.wd);
    ok = uvm_hdl_force("top.s.cnt", 8'h40);         #1; $display("T|force cnt %0d %h", ok, top.s.cnt);
    void'(uvm_hdl_deposit("top.s.tick", 0)); #1; void'(uvm_hdl_deposit("top.s.tick", 1)); #1;
    $display("T|forced cnt %h", top.s.cnt);
    ok = uvm_hdl_release("top.s.cnt");             #1; $display("T|release cnt %0d %h", ok, top.s.cnt);
    void'(uvm_hdl_deposit("top.s.tick", 0)); #1; void'(uvm_hdl_deposit("top.s.tick", 1)); #1;
    $display("T|released cnt %h", top.s.cnt);
    ok = uvm_hdl_read("top.s.off[4:7]", d);    $display("T|reversed %0d", ok);
    ok = uvm_hdl_read("top.s.off[20:16]", d);  $display("T|out_of_range %0d", ok);
    ok = uvm_hdl_deposit("top.s.nope", 1);     $display("T|dep_missing %0d", ok);
    ok = uvm_hdl_read("top.s", d);             $display("T|read_scope %0d", ok);
    ok = uvm_hdl_force("top.s.off[7:4]", 4'h1); $display("T|force_slice %0d", ok);
    $finish;
  end
endmodule
"#;

pub(super) fn uvm_1_2_dpi_hdl_backdoor_paths() {
    let sim = run_uvm_dpi("1.2", HDL_BENCH, &[]);
    let out = lines(&sim);
    // Reference-verified line for line, except `force_slice`: the C code
    // forces a part-select bit by bit, which xezim cannot express, so the
    // call fails (with a one-time [DPI] error) instead of forcing more or
    // fewer bits than asked.
    assert_lines(
        &out,
        &[
            "T|check 111001011",
            "T|off 1 00a5",
            "T|off[7:4] 1 0005",
            "T|off[4] 1 0001",
            "T|asc[0:3] 1 0008",
            "T|asc[7] 1 0001",
            "T|st.mid 1 00ee",
            "T|mem[5] 1 000f",
            "T|mem[5][3:0] 1 000f",
            "T|wide 1 000000090123456789abcdef01234567",
            "T|sg 1 00fd",
            "T|wd 1 00aa",
            "T|dep off 1 35 3a",
            "T|dep asc 1 10000010",
            "T|dep mem 1 ee",
            "T|dep st 1 bee1",
            "T|dep sg 1 -2",
            "T|dep wide 1 fffffffffffffffffffffffff",
            "T|dep tick 1 1",
            "T|force wd 1 77",
            "T|forced wd 77",
            "T|release wd 1 0f 0f",
            "T|force cnt 1 40",
            "T|forced cnt 40",
            "T|release cnt 1 40",
            "T|released cnt 41",
            "T|reversed 0",
            "T|out_of_range 0",
            "T|dep_missing 0",
            "T|read_scope 0",
            "T|force_slice 0",
        ],
    );
    assert_eq!(count_id(&out, "UVM_ERROR", "UVM/DPI/HDL_GET"), 3);
    assert_eq!(count_id(&out, "UVM_ERROR", "UVM/DPI/HDL_SET"), 1);
    assert!(
        out.iter().any(|l| l.starts_with("UVM_ERROR @")
            && l.ends_with(
                "reporter [UVM/DPI/HDL_SET] set: unable to locate hdl path (top.s.nope)"
            )),
        "HDL_SET report:\n{}",
        out.join("\n")
    );
}
