//! UVM integration tests against the real Accellera UVM library.
//!
//! Requirement: a checkout of https://github.com/nitronis/UVM — one repo
//! carrying the four UVM releases as subdirectories (1.1d, 1.2,
//! 1800.2-2017, 1800.2-2020). Located in this order:
//!   1. `XEZIM_UVM_DIR` (points at the checkout root)
//!   2. `../UVM` (sibling of this repo)
//!   3. auto-cloned (shallow) into `target/uvm-checkout`
//!
//! The bare run-phase tests are the #109 regression pin: `run_test()` used
//! to spin at 99% CPU without advancing time; it now completes the phase
//! cycle, so an objection held for #100 must end the run at t=100 on all
//! three validated UVM versions (1.1d bootstraps but is not yet
//! label-clean, see debug_notes round 44). The 2020 tests additionally pin
//! the full TLM bench (sequencer/driver/monitor/scoreboard) and config_db.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use xezim::*;

fn uvm_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Ok(d) = std::env::var("XEZIM_UVM_DIR") {
            let p = PathBuf::from(d);
            assert!(
                p.join("1.2/src/uvm_pkg.sv").exists(),
                "XEZIM_UVM_DIR does not look like a nitronis/UVM checkout: {}",
                p.display()
            );
            return p;
        }
        let sibling = Path::new("../UVM");
        if sibling.join("1.2/src/uvm_pkg.sv").exists() {
            return sibling.canonicalize().expect("canonicalize ../UVM");
        }
        let target = Path::new("target/uvm-checkout");
        if !target.join("1.2/src/uvm_pkg.sv").exists() {
            let status = Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "https://github.com/nitronis/UVM",
                    target.to_str().unwrap(),
                ])
                .status()
                .expect("git not available to fetch the UVM checkout");
            assert!(
                status.success(),
                "cloning https://github.com/nitronis/UVM failed \
                 (set XEZIM_UVM_DIR to an existing checkout to skip the clone)"
            );
        }
        target.canonicalize().expect("canonicalize target/uvm-checkout")
    })
    .clone()
}

/// Objection raised in run_phase, held #100, dropped: the simulation must
/// end at exactly t=100 (phase cycle completes, $finish fires).
const BARE_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class my_test extends uvm_test;
   `uvm_component_utils(my_test)
   function new(string name = "my_test", uvm_component parent = null);
      super.new(name, parent);
   endfunction
   task run_phase(uvm_phase phase);
      phase.raise_objection(this);
      #100;
      phase.drop_objection(this);
   endtask
endclass

module top;
   initial run_test("my_test");
endmodule
"#;

fn run_uvm(version: &str, extra_incdirs: &[String], test_src: String, top: &str) -> Result<compiler::Simulator, String> {
    run_uvm_opt(version, extra_incdirs, test_src, top, true)
}

/// `no_dpi = false` compiles UVM with its DPI imports live, so xezim's
/// native implementations of the UVM DPI-C helpers (uvm_hdl_*) serve them —
/// which is what a register model's BACKDOOR access needs.
fn run_uvm_opt(
    version: &str,
    extra_incdirs: &[String],
    test_src: String,
    top: &str,
    no_dpi: bool,
) -> Result<compiler::Simulator, String> {
    let root = uvm_dir().join(version);
    let src_dir = root.join("src");
    let uvm_pkg = std::fs::read_to_string(src_dir.join("uvm_pkg.sv"))
        .unwrap_or_else(|e| panic!("read {}/src/uvm_pkg.sv: {}", version, e));
    let mut include_dirs = vec![src_dir.to_str().unwrap().to_string()];
    include_dirs.extend_from_slice(extra_incdirs);
    let defines = if no_dpi {
        vec![("UVM_NO_DPI".to_string(), None)]
    } else {
        Vec::new()
    };
    simulate_multi(
        &[uvm_pkg, test_src],
        2000,
        Some(top),
        &include_dirs,
        &[],
        None,
        false,
        None,
        None,
        &defines,
        &[],
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
        None,
    )
}

fn bare_run_completes_on(version: &str) {
    let sim = run_uvm(version, &[], BARE_TEST.to_string(), "top")
        .unwrap_or_else(|e| panic!("UVM {} bare test failed to simulate: {}", version, e));
    assert_eq!(
        sim.time, 100,
        "UVM {}: run_phase objection held to #100 must end the run at t=100",
        version
    );
}

#[test]
fn uvm_1_2_bare_run_phase_completes() {
    bare_run_completes_on("1.2");
}

#[test]
fn uvm_1800_2_2017_bare_run_phase_completes() {
    bare_run_completes_on("1800.2-2017");
}

#[test]
fn uvm_1800_2_2020_bare_run_phase_completes() {
    bare_run_completes_on("1800.2-2020");
}

/// Full driver/sequencer/monitor/scoreboard bench on UVM 1800.2-2020: the
/// sequence drives 10 transactions through the sequencer/driver TLM
/// handshake; the monitor's analysis port fans out to the scoreboard. Pins
/// the assoc-key fix that unblocked TLM: connection registries are keyed by
/// hierarchical names (dotted strings), which key enumeration used to drop —
/// every `connect()` then resolved to 0 connections and BUILDERR aborted
/// the run at t=0.
#[test]
fn uvm_2020_complete_bench_runs_traffic() {
    let test_src = std::fs::read_to_string("tests/uvm/uvm_complete_test.sv")
        .expect("Could not read uvm_complete_test.sv");
    let sim = run_uvm("1800.2-2020", &[], test_src, "top")
        .expect("UVM 2020 complete bench failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        // The report SUMMARY legitimately prints zero-count rows
        // ("UVM_ERROR :    0") — matching them made the assert fail the
        // moment the summary became reference-correct. Match actual
        // report lines ("UVM_ERROR @ <time>") and nonzero counts only.
        !out.iter().flat_map(|m| m.lines()).any(|l| {
            (l.contains("UVM_ERROR") || l.contains("UVM_FATAL"))
                && !l
                    .trim_end()
                    .rsplit(':')
                    .next()
                    .is_some_and(|c| c.trim() == "0")
        }),
        "UVM errors:\n{}",
        out.join("\n")
    );
    let checked = out.iter().filter(|l| l.contains("[SB] Checked data:")).count();
    assert_eq!(checked, 10, "scoreboard must check all 10 transactions:\n{}", out.join("\n"));
    assert_eq!(sim.time, 100, "test drops its objection at t=100");
}

/// UVM 1800.2-2020 `uvm_config_db#(int)`: a value set at the top against
/// `uvm_test_top` must be visible to the test's build_phase get.
#[test]
fn uvm_2020_config_db_reaches_build_phase() {
    let sim = run_uvm("1800.2-2020", &[], CFG_TEST.to_string(), "top")
        .expect("UVM 2020 config_db test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|l| l.contains("cfg=77")),
        "config_db set(uvm_test_top, magic, 77) must reach build_phase:\n{}",
        out.join("\n")
    );
}

/// `base_c::type_id::set_type_override(deriv_c::get_type())` must make
/// `type_id::create` return the derived type. Pins the unpacked-struct
/// class-property copy fix: the factory's override matcher copies
/// `override.orig` (a struct with a class-handle + string member) into a
/// local through a ?: — that whole-struct read through a frame-held handle
/// came back x, so every registered override failed to match.
#[test]
fn uvm_2020_factory_type_override() {
    let sim = run_uvm("1800.2-2020", &[], CFG_TEST.to_string(), "top")
        .expect("UVM 2020 factory test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|l| l.contains("kind=deriv")),
        "type override must make create() return the derived type:\n{}",
        out.join("\n")
    );
}

/// Restoration of the shim-era `analysis_imp_decl_test.sv` (removed with the
/// PURE_SV_LRM=0 intercepts in 8bace9e without a native successor): two
/// `uvm_analysis_imp_decl` suffixed imps must route `port.write()` through
/// the macro-generated forwarders to the RIGHT suffixed method
/// (`write_in`/`write_out`). This is the requirement whose lost coverage let
/// the dotted-key enumeration regression (92abea7) break `connect()`
/// silently — now pinned against the real 1800.2-2020 library.
#[test]
fn uvm_2020_analysis_imp_decl_routes_to_suffixed_write() {
    let sim = run_uvm("1800.2-2020", &[], IMP_DECL_TEST.to_string(), "top")
        .expect("UVM 2020 imp_decl test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|l| l.contains("TEST_PASS")),
        "imp_decl write routing must hit the suffixed subscribers (2/1):\n{}",
        out.join("\n")
    );
}

const IMP_DECL_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

`uvm_analysis_imp_decl(_in)
`uvm_analysis_imp_decl(_out)

module top;
  class scoreboard;
    int n_in, n_out;
    function void write_in(int t);  n_in++;  endfunction
    function void write_out(int t); n_out++; endfunction
  endclass

  initial begin
    automatic scoreboard scb = new;
    automatic uvm_analysis_imp_in  #(int, scoreboard) imp_in  = new("imp_in",  scb);
    automatic uvm_analysis_imp_out #(int, scoreboard) imp_out = new("imp_out", scb);
    automatic uvm_analysis_port #(int) in_ap  = new("in_ap");
    automatic uvm_analysis_port #(int) out_ap = new("out_ap");

    in_ap.connect(imp_in);
    out_ap.connect(imp_out);
    in_ap.resolve_bindings();
    out_ap.resolve_bindings();

    in_ap.write(7);
    in_ap.write(8);
    out_ap.write(9);

    if (scb.n_in !== 2 || scb.n_out !== 1)
      $display("TEST_FAIL: n_in=%0d (exp 2) n_out=%0d (exp 1)", scb.n_in, scb.n_out);
    else
      $display("TEST_PASS");
    $finish;
  end
endmodule
"#;

/// Factory-override + config_db probe (see the two tests above).
const CFG_TEST: &str = r#"
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

class cfg_test extends uvm_test;
  `uvm_component_utils(cfg_test)
  base_c comp;
  int cfgval;
  function new(string name = "cfg_test", uvm_component parent = null); super.new(name, parent); endfunction
  virtual function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    base_c::type_id::set_type_override(deriv_c::get_type());
    comp = base_c::type_id::create("comp", this);
    if (!uvm_config_db#(int)::get(this, "", "magic", cfgval)) cfgval = -1;
  endfunction
  virtual task run_phase(uvm_phase phase);
    phase.raise_objection(this);
    `uvm_info("CFG", $sformatf("kind=%s cfg=%0d", comp.kind(), cfgval), UVM_LOW)
    #10;
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial begin
    uvm_config_db#(int)::set(null, "uvm_test_top", "magic", 77);
    run_test("cfg_test");
  end
endmodule
"#;

// §6.21: package-scope class-handle aliasing (`uvm_default_printer =
// uvm_default_table_printer`) — const-eval stored null and the initial
// block died at t=0 before run_test(). Now a static init; the example
// phases through top.run_phase's #1us objection window.
#[test]
fn test_uvm_hello_world() {
    let hw_dir = uvm_dir().join("1.2/examples/simple/hello_world");
    let test_src = std::fs::read_to_string(hw_dir.join("hello_world.sv"))
        .expect("Could not read hello_world.sv");
    let incs = vec![hw_dir.to_str().unwrap().to_string()];
    let sim = run_uvm("1.2", &incs, test_src, "hello_world").expect("hello_world failed");
    assert!(sim.time > 0, "hello_world must advance past t=0");
}

/// End-to-end pin for the built-in UVM DPI-C helpers: compiled WITHOUT
/// UVM_NO_DPI, the command-line processor walks the real argv, so
/// +UVM_CONFIG_DB_TRACE must produce CFGDB trace messages. Under
/// UVM_NO_DPI this plusarg is dead in every simulator (the no-DPI
/// fallback returns no args) — the builtin DPI layer is what makes it
/// live.
#[test]
fn uvm_1800_2_2017_config_db_trace_plusarg_with_dpi() {
    const TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class my_test extends uvm_test;
   `uvm_component_utils(my_test)
   function new(string name = "my_test", uvm_component parent = null);
      super.new(name, parent);
   endfunction
   function void build_phase(uvm_phase phase);
      int got;
      uvm_config_db#(int)::set(this, "", "knob", 42);
      void'(uvm_config_db#(int)::get(this, "", "knob", got));
      `uvm_info("KNOB", $sformatf("got=%0d", got), UVM_LOW)
   endfunction
   task run_phase(uvm_phase phase);
      phase.raise_objection(this);
      #100;
      phase.drop_objection(this);
   endtask
endclass

module top;
   initial run_test("my_test");
endmodule
"#;
    let root = uvm_dir().join("1800.2-2017");
    let src_dir = root.join("src");
    let uvm_pkg = std::fs::read_to_string(src_dir.join("uvm_pkg.sv")).expect("read uvm_pkg.sv");
    let include_dirs = vec![src_dir.to_str().unwrap().to_string()];
    let sim = simulate_multi(
        &[uvm_pkg, TEST.to_string()],
        2000,
        Some("top"),
        &include_dirs,
        &[],
        None,
        false,
        None,
        None,
        &[], // NO defines — UVM_NO_DPI deliberately absent
        &["+UVM_CONFIG_DB_TRACE".to_string()],
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
        None,
    )
    .expect("UVM 2017 with DPI builtins failed to simulate");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("CFGDB/SET") && m.contains("knob")),
        "+UVM_CONFIG_DB_TRACE must produce a CFGDB/SET trace for the knob set; output tail: {:?}",
        &msgs[msgs.len().saturating_sub(15)..]
    );
    assert!(
        msgs.iter().any(|m| m.contains("got=42")),
        "config_db get must still return the value; output tail: {:?}",
        &msgs[msgs.len().saturating_sub(15)..]
    );
    assert_eq!(sim.time, 100, "phase cycle must still end at t=100");
}


/// TLM-1 blocking path audit (reference-verified 7/7): blocking put through
/// an imp with REAL back-pressure (producer completes at t=6, gated by the
/// consumer's #2 per item), analysis broadcast to two subscribers, and a
/// bounded uvm_tlm_fifo with blocking-put back-pressure, peek, in-order
/// delivery, and drain. All of this rode on the round-72 mailbox fixes
/// (nested-receiver inlining + multi-waiter drain).
const TLM1_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class item_c extends uvm_object;
  int val;
  `uvm_object_utils(item_c)
  function new(string name = "item_c"); super.new(name); endfunction
endclass

// ---- consumer implements blocking put ----
class consumer_c extends uvm_component;
  `uvm_component_utils(consumer_c)
  uvm_blocking_put_imp #(item_c, consumer_c) put_export;
  int got[$];
  function new(string name, uvm_component parent);
    super.new(name, parent);
    put_export = new("put_export", this);
  endfunction
  task put(item_c t);
    #2;               // consume takes time — producer must block with us
    got.push_back(t.val);
  endtask
endclass

class producer_c extends uvm_component;
  `uvm_component_utils(producer_c)
  uvm_blocking_put_port #(item_c) put_port;
  int done_at = -1;
  function new(string name, uvm_component parent);
    super.new(name, parent);
    put_port = new("put_port", this);
  endfunction
  task run_phase(uvm_phase phase);
    item_c t;
    phase.raise_objection(this);
    for (int k = 0; k < 3; k++) begin
      t = item_c::type_id::create($sformatf("t%0d", k));
      t.val = 100 + k;
      put_port.put(t);
    end
    done_at = $time;
    phase.drop_objection(this);
  endtask
endclass

// ---- analysis broadcast to two subscribers ----
class sub_c extends uvm_subscriber #(item_c);
  `uvm_component_utils(sub_c)
  int seen[$];
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  function void write(item_c t);
    seen.push_back(t.val);
  endfunction
endclass

// ---- fifo-coupled producer/consumer ----
class fifo_prod_c extends uvm_component;
  `uvm_component_utils(fifo_prod_c)
  uvm_blocking_put_port #(item_c) out;
  function new(string name, uvm_component parent);
    super.new(name, parent);
    out = new("out", this);
  endfunction
  task run_phase(uvm_phase phase);
    item_c t;
    phase.raise_objection(this);
    for (int k = 0; k < 4; k++) begin
      #1;
      t = item_c::type_id::create($sformatf("f%0d", k));
      t.val = 200 + k;
      out.put(t);       // fifo depth 2: 3rd put must block until a get
    end
    phase.drop_objection(this);
  endtask
endclass

class fifo_cons_c extends uvm_component;
  `uvm_component_utils(fifo_cons_c)
  uvm_blocking_get_peek_port #(item_c) inp;
  int got[$];
  int peeked = -1;
  function new(string name, uvm_component parent);
    super.new(name, parent);
    inp = new("inp", this);
  endfunction
  task run_phase(uvm_phase phase);
    item_c t;
    phase.raise_objection(this);
    #10;
    inp.peek(t); peeked = t.val;          // peek leaves item
    for (int k = 0; k < 4; k++) begin
      inp.get(t);
      got.push_back(t.val);
    end
    phase.drop_objection(this);
  endtask
endclass

class tlm_test extends uvm_test;
  `uvm_component_utils(tlm_test)
  producer_c prod;
  consumer_c cons;
  uvm_analysis_port #(item_c) ap;
  sub_c sub1, sub2;
  fifo_prod_c fprod;
  fifo_cons_c fcons;
  uvm_tlm_fifo #(item_c) fifo;
  int failures = 0;
  function new(string name = "tlm_test", uvm_component parent = null);
    super.new(name, parent);
  endfunction
  function void build_phase(uvm_phase phase);
    prod = producer_c::type_id::create("prod", this);
    cons = consumer_c::type_id::create("cons", this);
    ap = new("ap", this);
    sub1 = sub_c::type_id::create("sub1", this);
    sub2 = sub_c::type_id::create("sub2", this);
    fprod = fifo_prod_c::type_id::create("fprod", this);
    fcons = fifo_cons_c::type_id::create("fcons", this);
    fifo = new("fifo", this, 2);
  endfunction
  function void connect_phase(uvm_phase phase);
    prod.put_port.connect(cons.put_export);
    ap.connect(sub1.analysis_export);
    ap.connect(sub2.analysis_export);
    fprod.out.connect(fifo.blocking_put_export);
    fcons.inp.connect(fifo.blocking_get_peek_export);
  endfunction
  task chk(bit ok, string what);
    if (!ok) begin failures++; $display("FAIL: %s", what); end
    else $display("PASS: %s", what);
  endtask
  task run_phase(uvm_phase phase);
    item_c t;
    phase.raise_objection(this);
    // analysis broadcast (functions, immediate)
    t = item_c::type_id::create("a0"); t.val = 7;  ap.write(t);
    t = item_c::type_id::create("a1"); t.val = 9;  ap.write(t);
    #40;
    chk(cons.got.size() == 3 && cons.got[0] == 100 && cons.got[2] == 102,
        $sformatf("blocking put through imp (got %0d items)", cons.got.size()));
    chk(prod.done_at == 6, $sformatf("producer blocked with consumer (done at %0d, want 6)", prod.done_at));
    chk(sub1.seen.size() == 2 && sub1.seen[0] == 7 && sub1.seen[1] == 9, "analysis sub1 saw both writes");
    chk(sub2.seen.size() == 2 && sub2.seen[1] == 9, "analysis sub2 saw both writes");
    chk(fcons.peeked == 200, $sformatf("fifo peek saw first item (got %0d)", fcons.peeked));
    chk(fcons.got.size() == 4 && fcons.got[0] == 200 && fcons.got[3] == 203,
        $sformatf("fifo delivered all 4 in order (got %0d)", fcons.got.size()));
    chk(fifo.used() == 0, $sformatf("fifo drained (used=%0d)", fifo.used()));
    if (failures == 0) $display("TEST_PASS");
    else $display("TEST_FAIL count=%0d", failures);
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial run_test("tlm_test");
endmodule

"#;

#[test]
fn uvm_tlm1_blocking_ports_fifo_analysis() {
    let sim = run_uvm("1.2", &[], TLM1_TEST.to_string(), "top")
        .expect("UVM 1.2 TLM1 test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "TLM1 blocking audit did not pass:
{}",
        out.join("
")
    );
    assert!(
        !out.iter().any(|m| m.starts_with("FAIL:")),
        "TLM1 blocking audit had failures:
{}",
        out.join("
")
    );
}

/// TLM-1 nonblocking family audit (reference-verified 18/18): try_put/
/// try_get/try_peek/can_put/can_get on a bounded fifo (incl. try_put on
/// FULL), used()/is_full/is_empty, the analysis fifo's unbounded write
/// side with blocking gets, and flush().
const TLM2_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class item_c extends uvm_object;
  int val;
  `uvm_object_utils(item_c)
  function new(string name = "item_c"); super.new(name); endfunction
endclass

class nb_test extends uvm_test;
  `uvm_component_utils(nb_test)
  uvm_tlm_fifo #(item_c) fifo;
  uvm_tlm_analysis_fifo #(item_c) afifo;
  uvm_analysis_port #(item_c) ap;
  int failures = 0;
  function new(string name = "nb_test", uvm_component parent = null);
    super.new(name, parent);
  endfunction
  function void build_phase(uvm_phase phase);
    fifo  = new("fifo", this, 2);
    afifo = new("afifo", this);
    ap    = new("ap", this);
  endfunction
  function void connect_phase(uvm_phase phase);
    ap.connect(afifo.analysis_export);
  endfunction
  task chk(bit ok, string what);
    if (!ok) begin failures++; $display("FAIL: %s", what); end
    else $display("PASS: %s", what);
  endtask
  task run_phase(uvm_phase phase);
    item_c t, r;
    bit ok;
    phase.raise_objection(this);
    // ---- nonblocking try/can family on bounded fifo ----
    chk(fifo.can_put() == 1, "empty bounded fifo can_put");
    chk(fifo.can_get() == 0, "empty bounded fifo cannot get");
    chk(fifo.try_get(r) == 0, "try_get on empty returns 0");
    chk(fifo.try_peek(r) == 0, "try_peek on empty returns 0");
    t = item_c::type_id::create("n0"); t.val = 11; ok = fifo.try_put(t);
    chk(ok == 1, "try_put #1");
    t = item_c::type_id::create("n1"); t.val = 22; ok = fifo.try_put(t);
    chk(ok == 1, "try_put #2");
    t = item_c::type_id::create("n2"); t.val = 33; ok = fifo.try_put(t);
    chk(ok == 0, "try_put on FULL returns 0");
    chk(fifo.used() == 2 && fifo.is_full(), "used()==2 and is_full");
    ok = fifo.try_peek(r);
    chk(ok == 1 && r.val == 11, "try_peek front");
    ok = fifo.try_get(r);
    chk(ok == 1 && r.val == 11, "try_get front");
    ok = fifo.try_get(r);
    chk(ok == 1 && r.val == 22, "try_get second");
    chk(fifo.is_empty(), "fifo empty again");
    // ---- analysis fifo: unbounded write-side, blocking get side ----
    t = item_c::type_id::create("a0"); t.val = 55; ap.write(t);
    t = item_c::type_id::create("a1"); t.val = 66; ap.write(t);
    t = item_c::type_id::create("a2"); t.val = 77; ap.write(t);
    chk(afifo.used() == 3, $sformatf("analysis fifo buffered 3 (used=%0d)", afifo.used()));
    afifo.get(r); chk(r.val == 55, "analysis fifo get #1");
    afifo.get(r); chk(r.val == 66, "analysis fifo get #2");
    afifo.get(r); chk(r.val == 77, "analysis fifo get #3");
    // ---- flush ----
    t = item_c::type_id::create("f0"); t.val = 1; void'(fifo.try_put(t));
    fifo.flush();
    chk(fifo.used() == 0, "flush empties fifo");
    if (failures == 0) $display("TEST_PASS");
    else $display("TEST_FAIL count=%0d", failures);
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial run_test("nb_test");
endmodule

"#;

#[test]
fn uvm_tlm1_nonblocking_family_analysis_fifo() {
    let sim = run_uvm("1.2", &[], TLM2_TEST.to_string(), "top")
        .expect("UVM 1.2 TLM2 test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "TLM1 nonblocking audit did not pass:
{}",
        out.join("
")
    );
    assert!(
        !out.iter().any(|m| m.starts_with("FAIL:")),
        "TLM1 nonblocking audit had failures:
{}",
        out.join("
")
    );
}


/// uvm_event trigger data + timing, uvm_barrier 3-way release, global event pool identity (reference-verified 6/6)
const UVM_EVENTS_BARRIERS_POOLS_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"
class ev_test extends uvm_test;
  `uvm_component_utils(ev_test)
  int failures = 0;
  uvm_event #(int) ev;
  uvm_barrier bar;
  int woke_at = -1;
  int bar_done[3];
  function new(string name = "ev_test", uvm_component parent = null); super.new(name, parent); endfunction
  task chk(bit ok, string what);
    if (!ok) begin failures++; $display("FAIL: %s", what); end
    else $display("PASS: %s", what);
  endtask
  task run_phase(uvm_phase phase);
    uvm_event_pool pool = uvm_event_pool::get_global_pool();
    uvm_event pooled_a = pool.get("sync_a");
    uvm_event pooled_b = pool.get("sync_a");
    phase.raise_objection(this);
    ev = new("ev");
    bar = new("bar", 3);
    chk(pooled_a == pooled_b, "event pool returns same event for same name");
    fork
      begin
        int d;
        ev.wait_trigger();
        d = ev.get_trigger_data();
        woke_at = $time;
        chk(d == 42, $sformatf("event trigger data (got %0d)", d));
      end
      begin #7 ev.trigger(42); end
      begin bar.wait_for(); bar_done[0] = $time; end
      begin #3 bar.wait_for(); bar_done[1] = $time; end
      begin #12 bar.wait_for(); bar_done[2] = $time; end
    join
    chk(woke_at == 7, $sformatf("event woke at 7 (got %0d)", woke_at));
    chk(bar_done[0] == 12 && bar_done[1] == 12 && bar_done[2] == 12,
        $sformatf("barrier released all at 12 (got %0d %0d %0d)", bar_done[0], bar_done[1], bar_done[2]));
    chk(ev.is_on(), "event stays on after trigger");
    ev.reset();
    chk(!ev.is_on(), "event reset clears state");
    if (failures == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", failures);
    phase.drop_objection(this);
  endtask
endclass
module top; initial run_test("ev_test"); endmodule

"#;

#[test]
fn uvm_events_barriers_pools() {
    let sim = run_uvm("1.2", &[], UVM_EVENTS_BARRIERS_POOLS_TEST.to_string(), "top")
        .expect("UVM 1.2 uvm_events_barriers_pools failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "uvm_events_barriers_pools did not pass:
{}",
        out.join("
")
    );
    assert!(
        !out.iter().any(|m| m.starts_with("FAIL:")),
        "uvm_events_barriers_pools had failures:
{}",
        out.join("
")
    );
}

/// uvm_resource_db int/string, report catcher demotion (error count stays 0), verbosity filtering (reference-verified)
const UVM_RESOURCE_DB_REPORT_CATCHER_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"
class demoter extends uvm_report_catcher;
  int caught = 0;
  function new(string name = "demoter"); super.new(name); endfunction
  function action_e catch();
    if (get_id() == "NOISY") begin
      caught++;
      set_severity(UVM_INFO);
    end
    return THROW;
  endfunction
endclass
class rd_test extends uvm_test;
  `uvm_component_utils(rd_test)
  int failures = 0;
  function new(string name = "rd_test", uvm_component parent = null); super.new(name, parent); endfunction
  task chk(bit ok, string what);
    if (!ok) begin failures++; $display("FAIL: %s", what); end
    else $display("PASS: %s", what);
  endtask
  task run_phase(uvm_phase phase);
    int v;
    string sv;
    demoter d = new();
    uvm_report_server srv = uvm_report_server::get_server();
    phase.raise_objection(this);
    // resource_db
    uvm_resource_db#(int)::set("scope_a", "knob", 77, this);
    chk(uvm_resource_db#(int)::read_by_name("scope_a", "knob", v), "resource_db read_by_name");
    chk(v == 77, $sformatf("resource_db value (got %0d)", v));
    uvm_resource_db#(string)::set("scope_a", "sknob", "hello", this);
    chk(uvm_resource_db#(string)::read_by_name("scope_a", "sknob", sv) && sv == "hello", "resource_db string value");
    // report catcher demotes an error
    uvm_report_cb::add(null, d);
    `uvm_error("NOISY", "should be demoted to info")
    chk(d.caught == 1, "report catcher saw the message");
    chk(srv.get_severity_count(UVM_ERROR) == 0, $sformatf("error demoted (count=%0d)", srv.get_severity_count(UVM_ERROR)));
    // verbosity filtering
    set_report_verbosity_level(UVM_LOW);
    `uvm_info("VFILT", "this HIGH message must be filtered", UVM_HIGH)
    `uvm_info("VKEEP", "this LOW message must appear", UVM_LOW)
    if (failures == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", failures);
    phase.drop_objection(this);
  endtask
endclass
module top; initial run_test("rd_test"); endmodule

"#;

#[test]
fn uvm_resource_db_report_catcher() {
    let sim = run_uvm("1.2", &[], UVM_RESOURCE_DB_REPORT_CATCHER_TEST.to_string(), "top")
        .expect("UVM 1.2 uvm_resource_db_report_catcher failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "uvm_resource_db_report_catcher did not pass:
{}",
        out.join("
")
    );
    assert!(
        !out.iter().any(|m| m.starts_with("FAIL:")),
        "uvm_resource_db_report_catcher had failures:
{}",
        out.join("
")
    );
}

/// TLM-2 b_target/initiator sockets: b_transport with target-side time consumption and payload mutation (reference-verified 3/3)
const UVM_TLM2_B_TRANSPORT_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"
class t2_target extends uvm_component;
  `uvm_component_utils(t2_target)
  uvm_tlm_b_target_socket #(t2_target) sock;
  int writes = 0;
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    sock = new("sock", this);
  endfunction
  task b_transport(uvm_tlm_generic_payload t, uvm_tlm_time delay);
    byte unsigned data[];
    #3;
    t.get_data(data);
    writes++;
    data[0] = data[0] + 8'h10;      // response mutates payload
    t.set_data(data);
    t.set_response_status(UVM_TLM_OK_RESPONSE);
  endtask
endclass
class t2_init extends uvm_component;
  `uvm_component_utils(t2_init)
  uvm_tlm_b_initiator_socket sock;
  int failures = 0;
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    sock = new("sock", this);
  endfunction
  task chk(bit ok, string what);
    if (!ok) begin failures++; $display("FAIL: %s", what); end
    else $display("PASS: %s", what);
  endtask
  task run_phase(uvm_phase phase);
    uvm_tlm_generic_payload gp = new("gp");
    uvm_tlm_time delay = new("d");
    byte unsigned data[] = '{8'h21};
    phase.raise_objection(this);
    gp.set_address(64'h1000);
    gp.set_data(data);
    gp.set_data_length(1);
    gp.set_write();
    sock.b_transport(gp, delay);
    chk($time == 3, $sformatf("b_transport consumed target time (t=%0t)", $time));
    chk(gp.get_response_status() == UVM_TLM_OK_RESPONSE, "OK response");
    gp.get_data(data);
    chk(data[0] == 8'h31, $sformatf("payload mutated by target (got %h)", data[0]));
    if (failures == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", failures);
    phase.drop_objection(this);
  endtask
endclass
class t2_test extends uvm_test;
  `uvm_component_utils(t2_test)
  t2_init ini;
  t2_target tgt;
  function new(string name = "t2_test", uvm_component parent = null); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    ini = t2_init::type_id::create("ini", this);
    tgt = t2_target::type_id::create("tgt", this);
  endfunction
  function void connect_phase(uvm_phase phase);
    ini.sock.connect(tgt.sock);
  endfunction
endclass
module top; initial run_test("t2_test"); endmodule

"#;

#[test]
fn uvm_tlm2_b_transport() {
    let sim = run_uvm("1.2", &[], UVM_TLM2_B_TRANSPORT_TEST.to_string(), "top")
        .expect("UVM 1.2 uvm_tlm2_b_transport failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "uvm_tlm2_b_transport did not pass:
{}",
        out.join("
")
    );
    assert!(
        !out.iter().any(|m| m.starts_with("FAIL:")),
        "uvm_tlm2_b_transport had failures:
{}",
        out.join("
")
    );
}

/// factory type/instance overrides + create_object_by_name honoring overrides (round-75 registry fixes) — reference-verified TEST_PASS.
const UVM_FACTORY_OVERRIDES_AND_BY_NAME_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"
class base_item extends uvm_object;
  `uvm_object_utils(base_item)
  function new(string name = "base_item"); super.new(name); endfunction
  virtual function string whoami(); return "base"; endfunction
endclass
class derived_item extends base_item;
  `uvm_object_utils(derived_item)
  function new(string name = "derived_item"); super.new(name); endfunction
  virtual function string whoami(); return "derived"; endfunction
endclass
class comp_a extends uvm_component;
  `uvm_component_utils(comp_a)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  virtual function string kind(); return "A"; endfunction
endclass
class comp_b extends comp_a;
  `uvm_component_utils(comp_b)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
  virtual function string kind(); return "B"; endfunction
endclass
class fac_test extends uvm_test;
  `uvm_component_utils(fac_test)
  int failures = 0;
  comp_a child;
  function new(string name = "fac_test", uvm_component parent = null); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    set_type_override_by_type(base_item::get_type(), derived_item::get_type());
    set_inst_override_by_type("child", comp_a::get_type(), comp_b::get_type());
    child = comp_a::type_id::create("child", this);
  endfunction
  task chk(bit ok, string what);
    if (!ok) begin failures++; $display("FAIL: %s", what); end
    else $display("PASS: %s", what);
  endtask
  task run_phase(uvm_phase phase);
    base_item it;
    uvm_object o;
    uvm_factory f = uvm_factory::get();
    phase.raise_objection(this);
    it = base_item::type_id::create("it");
    chk(it.whoami() == "derived", "type override applied on object create");
    chk(child.kind() == "B", "instance override applied on component create");
    o = f.create_object_by_name("base_item", "", "byname");
    chk(o != null, "create_object_by_name returns object");
    if (o != null) begin
      base_item bi;
      chk($cast(bi, o) && bi.whoami() == "derived", "by-name create honors override");
    end
    if (failures == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", failures);
    phase.drop_objection(this);
  endtask
endclass
module top; initial run_test("fac_test"); endmodule

"#;

#[test]
fn uvm_factory_overrides_and_by_name() {
    let sim = run_uvm("1.2", &[], UVM_FACTORY_OVERRIDES_AND_BY_NAME_TEST.to_string(), "top")
        .expect("UVM 1.2 uvm_factory_overrides_and_by_name failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "uvm_factory_overrides_and_by_name did not pass:
{}",
        out.join("
")
    );
}

/// field automation: copy/compare/clone/pack/unpack/sprint (clone needed the frame-scoped type overlay) — reference-verified TEST_PASS.
const UVM_FIELD_AUTOMATION_CLONE_PACK_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"
class pkt extends uvm_object;
  rand bit [15:0] addr;
  rand bit [31:0] data;
  bit [7:0] tag;
  `uvm_object_utils_begin(pkt)
    `uvm_field_int(addr, UVM_ALL_ON)
    `uvm_field_int(data, UVM_ALL_ON)
    `uvm_field_int(tag, UVM_ALL_ON)
  `uvm_object_utils_end
  function new(string name = "pkt"); super.new(name); endfunction
endclass
class fa_test extends uvm_test;
  `uvm_component_utils(fa_test)
  int failures = 0;
  function new(string name = "fa_test", uvm_component parent = null); super.new(name, parent); endfunction
  task chk(bit ok, string what);
    if (!ok) begin failures++; $display("FAIL: %s", what); end
    else $display("PASS: %s", what);
  endtask
  task run_phase(uvm_phase phase);
    pkt a, b, c;
    bit bits[];
    int n;
    string s;
    phase.raise_objection(this);
    a = pkt::type_id::create("a");
    a.addr = 16'hBEEF; a.data = 32'h12345678; a.tag = 8'h5A;
    // copy
    b = pkt::type_id::create("b");
    b.copy(a);
    chk(b.addr == 16'hBEEF && b.data == 32'h12345678 && b.tag == 8'h5A, "copy replicates fields");
    // compare
    chk(a.compare(b), "compare equal objects");
    b.tag = 8'h00;
    chk(!a.compare(b), "compare detects difference");
    // clone
    chk($cast(c, a.clone()) && c.addr == 16'hBEEF && c.data == 32'h12345678, "clone deep-copies");
    // pack/unpack
    n = a.pack(bits);
    chk(n == 56, $sformatf("pack size 56 (got %0d)", n));
    b = pkt::type_id::create("b2");
    void'(b.unpack(bits));
    chk(b.addr == a.addr && b.data == a.data && b.tag == a.tag, "unpack round-trips");
    // sprint
    s = a.sprint();
    chk(s.len() > 0, "sprint produces text");
    if (failures == 0) $display("TEST_PASS"); else $display("TEST_FAIL count=%0d", failures);
    phase.drop_objection(this);
  endtask
endclass
module top; initial run_test("fa_test"); endmodule

"#;

#[test]
fn uvm_field_automation_clone_pack() {
    let sim = run_uvm("1.2", &[], UVM_FIELD_AUTOMATION_CLONE_PACK_TEST.to_string(), "top")
        .expect("UVM 1.2 uvm_field_automation_clone_pack failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "uvm_field_automation_clone_pack did not pass:
{}",
        out.join("
")
    );
}
/// UVM 1.2 sequencer/driver handshake: a sequence's start_item must BLOCK
/// until the driver's get_next_item grants arbitration, and all items must
/// reach the driver. Pinned against the reference flow after three fixes:
/// the `while`-final-iteration `continue` flag leak (killed
/// m_choose_next_request's return), and the class-member-queue locator
/// resolution (grant_queued_locks' find_first_index returned empty, so a
/// REQ entry was treated as a leading lock — arbitration was granted with
/// no driver and the queue drained, starving get_next_item forever).
#[test]
fn uvm_1_2_sequencer_driver_handshake() {
    let test_src = std::fs::read_to_string("tests/uvm/uvm_sequencer_handshake.sv")
        .expect("Could not read uvm_sequencer_handshake.sv");
    let sim = run_uvm("1.2", &[], test_src, "top")
        .expect("UVM 1.2 sequencer handshake failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        !out.iter().any(|l| (l.contains("UVM_ERROR") && !l.contains("UVM_ERROR :"))
            || (l.contains("UVM_FATAL") && !l.contains("UVM_FATAL :"))),
        "UVM errors:\n{}",
        out.join("\n")
    );
    let got = out.iter().filter(|l| l.contains("DRV: got item")).count();
    assert_eq!(got, 3, "driver must receive all 3 items:\n{}", out.join("\n"));
    assert!(out.iter().any(|l| l.contains("TEST_PASS")), "TEST_PASS:\n{}", out.join("\n"));
}

/// UVM 1.2 register-model FRONTDOOR path end-to-end: read of reset value,
/// write + auto-predict mirror, read-back, field read-modify-write, and
/// set/update — with REAL data (the checks below pin exact values because
/// the TB's own `==` checks pass vacuously on x). Data integrity depends on
/// the unpacked-struct decl-init member copy (`op_s x = accesses[i]` in the
/// reg map's bus access loop) and on 2-state typedef locals defaulting to 0
/// (`uvm_reg_data_t value_adjust` in the field RMW).
#[test]
fn uvm_1_2_ral_frontdoor_read_write() {
    let test_src = std::fs::read_to_string("tests/uvm/uvm_ral_frontdoor.sv")
        .expect("Could not read uvm_ral_frontdoor.sv");
    let sim = run_uvm("1.2", &[], test_src, "top")
        .expect("UVM 1.2 RAL frontdoor failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        !out.iter().any(|l| (l.contains("UVM_ERROR") && !l.contains("UVM_ERROR :"))
            || (l.contains("UVM_FATAL") && !l.contains("UVM_FATAL :"))),
        "UVM errors:\n{}",
        out.join("\n")
    );
    assert!(!out.iter().any(|l| l.starts_with("FAIL:")), "TB FAILs:\n{}", out.join("\n"));
    for pin in [
        "read reset value (got 00000000000000c0)",
        "read-back (got 00000000a5a55a5a)",
        "MODE field write (got 0000000000003ac0)",
        "set+update (got 0000000012345678)",
        "TEST_PASS",
    ] {
        assert!(
            out.iter().any(|l| l.contains(pin)),
            "missing `{pin}`:\n{}",
            out.join("\n")
        );
    }
}

/// UVM 1.2 register-model BACKDOOR access end-to-end: `get_full_hdl_path`
/// composes `top.dut.<reg>` from the block's root path and the register's
/// configure-supplied slice, and the peek/poke reach the RTL signals
/// directly (no bus traffic, so it completes in one delta at t=20).
///
/// Exercises the nested `ref` queue formals in `get_full_hdl_path` (both
/// overloads take `ref ... paths[$]`, and the callee declares its own
/// `parent_paths`), plus the class-member pool whose element type is a
/// nested specialization.
#[test]
fn uvm_1_2_ral_backdoor_peek_poke() {
    let test_src = std::fs::read_to_string("tests/uvm/uvm_ral_backdoor.sv")
        .expect("Could not read uvm_ral_backdoor.sv");
    let sim = run_uvm_opt("1.2", &[], test_src, "top", false)
        .expect("UVM 1.2 RAL backdoor failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        !out.iter().any(|l| (l.contains("UVM_ERROR") && !l.contains("UVM_ERROR :"))
            || (l.contains("UVM_FATAL") && !l.contains("UVM_FATAL :"))),
        "UVM errors:\n{}",
        out.join("\n")
    );
    assert!(!out.iter().any(|l| l.starts_with("FAIL:")), "TB FAILs:\n{}", out.join("\n"));
    for pin in [
        "backdoor read reset (got 00000000000000c0)",
        "backdoor read-back (got 00000000facecafe)",
        "TEST_PASS",
    ] {
        assert!(out.iter().any(|l| l.contains(pin)), "missing `{pin}`:\n{}", out.join("\n"));
    }
}

/// Sequence-field static type resolution inside a started `body()`.
///
/// `uvm_create(seq)` (and a bare `seq.get_type()` in `body()`) recovers the
/// compile-time type of a sequence field that is still NULL by dispatching the
/// registered `static get_type()` on the field's DECLARED type. Here `seq` is
/// a `my_sequence` member of `top_seq`; `uvm_sequence_base` also defines a
/// `static get_type()` (via `uvm_object_abstract_utils`), so the concrete
/// member type must win. xezim's `receiver_static_class` used the loose
/// `class_of_var` heuristic first, which drifted a `my_sequence` class member
/// to `uvm_sequence_item`, so `uvm_create(seq)` asked the factory to build an
/// abstract `uvm_sequence_base`, and `start_item(seq)` then saw a null item
/// (NULLITM). Reference and xezim must both pick `my_sequence`.
const SEQ_GET_TYPE_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

class my_item extends uvm_sequence_item;
  `uvm_object_utils(my_item)
  function new(string name = "my_item"); super.new(name); endfunction
endclass

class my_sequence extends uvm_sequence #(my_item);
  `uvm_object_utils(my_sequence)
  function new(string name = "my_sequence"); super.new(name); endfunction
endclass

class top_seq extends uvm_sequence #(my_item);
  `uvm_object_utils(top_seq)
  my_sequence seq;
  function new(string name = "top_seq"); super.new(name); endfunction
  task body();
    uvm_object_wrapper w = seq.get_type();
    if (w != null && w.get_type_name() == "my_sequence")
      $display("TEST_PASS");
    else
      $display("TEST_FAIL got=%s", (w==null)?"<null>":w.get_type_name());
  endtask
endclass

class my_sqr extends uvm_sequencer #(my_item);
  `uvm_component_utils(my_sqr)
  function new(string name, uvm_component parent); super.new(name, parent); endfunction
endclass

class seq_test extends uvm_test;
  `uvm_component_utils(seq_test)
  my_sqr sq;
  function new(string name = "seq_test", uvm_component parent = null); super.new(name, parent); endfunction
  function void build_phase(uvm_phase phase);
    super.build_phase(phase);
    sq = my_sqr::type_id::create("sq", this);
  endfunction
  task run_phase(uvm_phase phase);
    top_seq ts;
    phase.raise_objection(this);
    ts = new("ts");
    ts.start(sq);
    #1;
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial run_test("seq_test");
endmodule
"#;

/// 2020 `uvm_sequence.start()` + `uvm_create`-style `get_type` on a sequence
/// FIELD (still null) must resolve to the concrete `my_sequence`, not the
/// abstract `uvm_sequence_base` / `uvm_sequence_item`.
#[test]
fn uvm_2020_sequence_field_get_type_resolves_declared_type() {
    let sim = run_uvm("1800.2-2020", &[], SEQ_GET_TYPE_TEST.to_string(), "top")
        .expect("UVM 2020 sequence-field get_type test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m.contains("TEST_PASS")),
        "sequence field get_type must resolve to my_sequence:
{}",
        out.join("\n")
    );
    assert!(
        !out.iter().any(|m| m.contains("TEST_FAIL")),
        "sequence field get_type mis-resolved:
{}",
        out.join("\n")
    );
}

/// A printer rendering an ENUM collection element must resolve the value's
/// member name against the collection's element type (`numbers`), not a global
/// scan that picks a larger outer-scope enum. UVM's own enums share the values
/// 0..3 with `numbers`, so a freshly-defaulted empty enum array element used to
/// render as UVM_NORADIX / UVM_PHASE_* instead of ONE/TWO/THREE/FOUR.
const ENUM_ARRAY_PRINT_TEST: &str = r#"
import uvm_pkg::*;
`include "uvm_macros.svh"

typedef enum bit [1:0] { ONE, TWO, THREE, FOUR } numbers;

class myobj extends uvm_object;
  numbers numa[];
  `uvm_object_utils_begin(myobj)
    `uvm_field_array_enum(numbers, numa, UVM_DEFAULT)
  `uvm_object_utils_end
  function new(string n="myobj");
    super.new(n);
    numa = new[4];
    for (int i=0;i<4;i++) numa[i]=numbers'(i);
  endfunction
endclass

class my_test extends uvm_test;
  `uvm_component_utils(my_test)
  function new(string name="my_test", uvm_component parent=null);
    super.new(name, parent);
  endfunction
  task run_phase(uvm_phase phase);
    myobj o = new;
    o.print();
    phase.raise_objection(this);
    phase.drop_objection(this);
  endtask
endclass

module top;
  initial run_test("my_test");
endmodule
"#;

#[test]
fn uvm_2020_printer_renders_enum_array_members() {
    let sim = run_uvm("1800.2-2020", &[], ENUM_ARRAY_PRINT_TEST.to_string(), "top")
        .expect("UVM 2020 enum-array printer test failed to simulate");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let joined = out.join("\n");
    assert!(
        joined.contains("ONE")
            && joined.contains("TWO")
            && joined.contains("THREE")
            && joined.contains("FOUR"),
        "printer must render the enum members:\n{}",
        joined
    );
    assert!(
        !joined.contains("UVM_NORADIX") && !joined.contains("UVM_PHASE_DORMANT"),
        "enum array elements must NOT fall back to a foreign enum's labels:\n{}",
        joined
    );
}

