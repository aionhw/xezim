//! Parser-level gaps fixed together — constructs that were REJECTED at
//! parse time (causing whole library-compilation units to fail).
//!
//!   disable iff without @(...)   §16.6: the `disable iff` clause in a
//!                                concurrent assertion was only parsed
//!                                inside the clocked `@(...) ` branch.
//!   virtual interface type arg  §25.9/§8.25.1: `C#(virtual my_if)` used
//!                                a virtual-interface type as a parameter
//!                                argument.
//!   inherited-type queue port   §6.3/§13.3: `f(string src, dest[$])`
//!                                inherits the type from the previous port.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── disable iff without a preceding @(...) clocking event ──────────────
// UVM interface files use `assert property (disable iff(g) body)` with a
// default clocking block, so `disable iff` appears directly after `property (`
// — no `@(...)`. The parser must accept this (§16.6).

const DISABLE_IFF_NOCLK_SRC: &str = r#"
module top;
  logic clk = 0;
  logic [3:0] grant = 0;
  logic has_checks = 1;
  default clocking cb @(posedge clk); endclocking
  a1: assert property (
        disable iff(!has_checks)
        ($onehot(grant)))
        else $display("TAG_FAIL");
  always #5 clk = ~clk;
  initial begin
    grant = 4'b0010;
    ##2;
    $display("TAG_PASS");
    $finish;
  end
endmodule
"#;

#[test]
fn disable_iff_without_clocking_event() {
    let sim = simulate(DISABLE_IFF_NOCLK_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "disable iff without @(...) should parse and run; got {:?}",
        msgs
    );
}

// ── virtual interface as type-parameter argument ──────────────────────
// `C#(virtual my_if)` passes a virtual-interface type as a type-parameter
// argument. Must parse without error (§25.9/§8.25.1).

const VIRTUAL_IF_TYPEARG_SRC: &str = r#"
interface my_if;
  logic [7:0] data;
endinterface

module top;
  class container #(type T = int);
    T val;
    function void set(T v);
      val = v;
    endfunction
  endclass

  my_if u_if();
  container #(virtual my_if) obj;

  initial begin
    obj = new();
    obj.set(u_if);
    $display("TAG_PASS");
    $finish;
  end
endmodule
"#;

#[test]
fn virtual_interface_type_argument_parses() {
    let sim = simulate(VIRTUAL_IF_TYPEARG_SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "C#(virtual my_if) should parse; got {:?}",
        msgs
    );
}

// ── inherited-type queue port `name [$]` ──────────────────────────────
// `function f(string src, dest[$])` — dest inherits type string from src,
// [$] is the unpacked queue dimension (§6.3/§13.3).

const INHERITED_QUEUE_PORT_SRC: &str = r#"
module top;
  function automatic int count(string src, dest[$]);
    return dest.size();
  endfunction
  initial begin
    string q[$];
    q.push_back("a");
    q.push_back("b");
    $display("count=%0d", count("x", q));
    if (count("x", q) == 2) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn inherited_type_queue_port() {
    let sim = simulate(INHERITED_QUEUE_PORT_SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "count=2"),
        "inherited-type queue port should parse and run; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "count should be 2; got {:?}",
        msgs
    );
}
