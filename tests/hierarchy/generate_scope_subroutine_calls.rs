//! §23.6 / §27.6 / §23.8 — subroutine calls into instances that live inside
//! generate scopes or instance arrays. Reference-validated.
//!
//! Such an instance's tasks and functions are registered under the evaluated
//! path (`g_rank[1].g_dev[0].u_leaf.report`), but a callee spelled through a
//! generate select parses as a MemberAccess/Index chain (or an Ident with
//! selects), which every call arm took for an object receiver: a function
//! call read 0 and a task call was silently dropped, while a variable read
//! through the same path worked. A call that names a sibling of an enclosing
//! scope (`sib.report()` inside a child, §23.8 upward resolution) tried only
//! the executing instance itself and was dropped too.

use xezim::simulate;

/// §11.4/§8.6: speculative hierarchy lookup must not evaluate an object
/// receiver's side-effectful selector when a module subroutine shares its name.
#[test]
fn indexed_object_calls_with_hierarchical_name_collisions() {
    const SRC: &str = r#"
class entry_t;
  int data;
  function new(int v); data = v; endfunction
  function int value(); return data; endfunction
  task bump(); data++; endtask
endclass
module leaf;
  function int value(); return 99; endfunction
  task bump(); endtask
endmodule
module tb;
  for (genvar n = 0; n < 2; n++) begin : branches
    leaf u_leaf();
  end
  entry_t entries[2];
  int picks = 0, slot = 0, result;
  function automatic int next_slot(); picks++; return 0; endfunction
  task automatic nested_probe();
    int result;
    result = entries[next_slot()].value();
    entries[slot++].bump();
    $display("T|nested result=%0d picks=%0d slot=%0d data=%0d", result, picks, slot, entries[0].data);
  endtask
  initial begin
    entries[0] = new(17);
    entries[1] = new(40);
    result = entries[next_slot()].value();
    entries[slot++].bump();
    $display("T|direct result=%0d picks=%0d slot=%0d data=%0d", result, picks, slot, entries[0].data);
    slot = 0;
    nested_probe();
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|direct result=17 picks=1 slot=1 data=18",
            "T|nested result=18 picks=2 slot=1 data=19",
        ],
    );
}

fn t_lines(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// A selector rejected by speculative probes still receives unknown-index
/// handling after its one real evaluation, at the array element's full width.
#[test]
fn side_effecting_unknown_array_selector_is_evaluated_once() {
    const SRC: &str = r#"
module tb;
  logic [15:0] words[2];
  int picks = 0;
  function automatic logic [31:0] next_slot();
    picks++;
    return 'x;
  endfunction
  initial begin
    words[0] = 16'h1234;
    words[1] = 16'h5678;
    $display("T|unknown=%h", words[next_slot()]);
    $display("T|picks=%0d", picks);
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(t_lines(&sim), ["T|unknown=xxxx", "T|picks=1"]);
}

#[test]
fn scalar_assignment_selectors_are_evaluated_once() {
    const SRC: &str = r#"
module tb;
  int words[2], picks = 0, slot = 0, result;
  function automatic int next_slot(); picks++; return 0; endfunction
  initial begin
    words[0] = 51;
    words[1] = 62;
    result = words[next_slot()];
    words[slot++] = result + 1;
    $display("T|picks=%0d slot=%0d result=%0d words=%0d,%0d", picks, slot, result, words[0], words[1]);
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(t_lines(&sim), ["T|picks=1 slot=1 result=51 words=52,62"]);
}

/// §7.2/§11.4.1: capturing a selector must preserve the member-wise copy,
/// including string members, rather than falling through to a scalar store.
#[test]
fn aggregate_assignment_captures_function_and_increment_selectors() {
    const SRC: &str = r#"
module tb;
  typedef struct { int tag; string text; } record_t;
  record_t entries[$], fixed_entries[2], source;
  int calls = 0, slot = 0;
  function automatic int pick(); calls++; return 0; endfunction
  initial begin
    source = '{10, "before"};
    entries.push_back(source);
    source = '{73, "after"};
    entries[pick()] = source;
    fixed_entries[slot++] = source;
    $display("T|queue=%0d,%s calls=%0d", entries[0].tag, entries[0].text, calls);
    $display("T|fixed=%0d,%s slot=%0d", fixed_entries[0].tag, fixed_entries[0].text, slot);
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(
        t_lines(&sim),
        ["T|queue=73,after calls=1", "T|fixed=73,after slot=1"]
    );
}

/// §11.5.1/§7.8: single evaluation must preserve unknown and string keys.
#[test]
fn captured_assignment_selectors_preserve_their_value_kind() {
    const SRC: &str = r#"
module tb;
  int words[2], mapping[string];
  int unknown_calls = 0, text_calls = 0;
  function automatic logic [31:0] unknown_slot(); unknown_calls++; return 'x; endfunction
  function automatic string text_slot(); text_calls++; return "sample"; endfunction
  initial begin
    words[0] = 51;
    words[1] = 62;
    words[unknown_slot()] = 99;
    mapping[text_slot()] = 73;
    $display("T|words=%0d,%0d calls=%0d", words[0], words[1], unknown_calls);
    $display("T|map=%0d calls=%0d size=%0d", mapping["sample"], text_calls, mapping.num());
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(
        t_lines(&sim),
        ["T|words=51,62 calls=1", "T|map=73 calls=1 size=1"]
    );
}

/// A local receiver named like the top module must not bind to a global
/// function when its method shares that function's name.
#[test]
fn object_receiver_shadows_implicit_top_scope() {
    const SRC: &str = r#"
class endpoint_t;
  function int probe(); return 37; endfunction
endclass
module top;
  int calls = 0;
  function automatic int probe();
    endpoint_t top;
    calls++;
    if (calls > 4) return -1;
    top = new;
    return top.probe();
  endfunction
  function automatic int absolute_probe();
    endpoint_t top = null;
    return $root.top.probe();
  endfunction
  initial begin
    $display("T|local=%0d", probe());
    $display("T|absolute=%0d", absolute_probe());
    $display("T|unshadowed=%0d", top.probe());
    $display("T|calls=%0d", calls);
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|local=37",
            "T|absolute=37",
            "T|unshadowed=37",
            "T|calls=3"
        ],
    );
}

/// Absolute and relative calls through generate loops, a generate-if, an instance array, generate scopes inside a
/// child, an upward reference, a blocking task, and a class method.
#[test]
fn calls_through_generate_scopes_and_instance_arrays() {
    const SRC: &str = r#"
module leaf #(parameter int ID = 0) ();
  int n = ID + 100;
  int cnt = 0;
  task report(); $display("T| %0t leaf %0d", $time, ID); endtask
  task bump(input int k); cnt += k; endtask
  task automatic wait_report(); #1 $display("T| %0t wleaf %0d", $time, ID); endtask
  function int get(); return n; endfunction
  function int add(int a); return n + a; endfunction
endmodule
module uleaf #(parameter int ID = 0) ();
  // upward reference: a sibling instance in the enclosing generate scope
  task up(); sib.report(); endtask
  function int upf(); return sib.get(); endfunction
endmodule
module mid #(parameter int ID = 0) ();
  leaf #(.ID(ID)) u_leaf ();
  leaf #(.ID(ID+1)) sib ();
  for (genvar i = 0; i < 2; i++) begin : g_in
    leaf #(.ID(ID*10+i)) u ();
  end
  task call_down(); g_in[1].u.report(); endtask
  function int fdown(); return g_in[0].u.get(); endfunction
endmodule
class C;
  task go();
    tb.g_one[0].u_leaf.report();
    $display("T| cls fn=%0d", tb.g_one[1].u_leaf.get());
  endtask
endclass
module tb;
  localparam int N = 2;
  genvar r, d;
  for (r = 0; r < N; r++) begin : g_rank
    for (d = 0; d < 2; d++) begin : g_dev
      leaf #(.ID(r*10 + d)) u_leaf ();
    end
  end
  for (d = 0; d < 2; d++) begin : g_one
    leaf #(.ID(50 + d)) u_leaf ();
    leaf #(.ID(60 + d)) sib ();
    uleaf #(.ID(d)) u_up ();
  end
  leaf #(.ID(70)) arr [0:2] ();
  mid #(.ID(3)) u_mid ();
  for (genvar k = 0; k < 2; k++) begin : g_mid
    mid #(.ID(4+k)) m ();
  end
  if (N == 2) begin : g_if
    leaf #(.ID(80)) u_leaf ();
  end
  int idx = 1;
  initial begin
    C c = new;
    #1;
    $display("T| var=%0d fn=%0d", g_rank[1].g_dev[0].u_leaf.n, g_rank[1].g_dev[0].u_leaf.get());
    $display("T| add=%0d", g_rank[0].g_dev[1].u_leaf.add(5));
    g_rank[1].g_dev[0].u_leaf.report();
    g_one[1].u_leaf.report();
    tb.g_one[0].u_leaf.report();
    g_if.u_leaf.report();
    arr[2].report();
    $display("T| arrfn=%0d", arr[1].get());
    u_mid.g_in[1].u.report();
    u_mid.call_down();
    $display("T| fdown=%0d", u_mid.fdown());
    g_mid[1].m.g_in[0].u.report();
    $display("T| deepfn=%0d", g_mid[1].m.g_in[1].u.get());
    g_one[1].u_leaf.bump(7);
    g_one[1].u_leaf.bump(3);
    $display("T| cnt=%0d", g_one[1].u_leaf.cnt);
    g_one[0].u_up.up();
    $display("T| upf=%0d", g_one[1].u_up.upf());
    g_one[1].u_leaf.wait_report();
    $display("T| %0t after wait", $time);
    c.go();
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T| var=110 fn=110",
            "T| add=106",
            "T| 1 leaf 10",
            "T| 1 leaf 51",
            "T| 1 leaf 50",
            "T| 1 leaf 80",
            "T| 1 leaf 70",
            "T| arrfn=170",
            "T| 1 leaf 31",
            "T| 1 leaf 31",
            "T| fdown=130",
            "T| 1 leaf 50",
            "T| deepfn=151",
            "T| cnt=10",
            "T| 1 leaf 60",
            "T| upf=161",
            "T| 2 wleaf 51",
            "T| 2 after wait",
            "T| 2 leaf 50",
            "T| cls fn=151",
        ],
    );
}

/// `u_leaf.report()` from an always block inside a two-level generate names
/// the instance of the SAME iteration.
#[test]
fn relative_call_from_nested_generate_always_block() {
    const SRC: &str = r#"
module leaf #(parameter int ID = 0) ();
  task report(); $display("T| %0t leaf %0d", $time, ID); endtask
endmodule
module tb;
  int rq = -1;
  genvar r, d;
  for (r = 0; r < 2; r++) begin : g_rank
    for (d = 0; d < 2; d++) begin : g_dev
      leaf #(.ID(r*10 + d)) u_leaf ();
      always @(rq) if (rq == r*10 + d) u_leaf.report();
    end
  end
  initial begin
    #1;
    for (int i = 0; i < 2; i++) for (int j = 0; j < 2; j++) begin rq = i*10 + j; #1; end
    $finish;
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(
        t_lines(&sim),
        ["T| 1 leaf 0", "T| 2 leaf 1", "T| 3 leaf 10", "T| 4 leaf 11",],
    );
}

/// Relative variable reads and calls two generate levels deep. An enclosing
/// iteration rewrote `u_leaf` to `g_rank[0].u_leaf`, which the inner
/// iteration no longer recognized, so the reference named no instance: the
/// read was x and the calls were lost.
#[test]
fn relative_reads_and_calls_two_generate_levels_deep() {
    const SRC: &str = r#"
module leaf #(parameter int ID = 0) ();
  int n = ID;
  task report(); $display("T| %0t leaf %0d", $time, ID); endtask
  function int get(); return ID; endfunction
endmodule
module tb;
  for (genvar r = 0; r < 2; r++) begin : g_rank
    for (genvar d = 0; d < 2; d++) begin : g_dev
      leaf #(.ID(r*10+d)) u_leaf ();
      initial #(1+r*2+d) $display("T| %0t n=%0d fn=%0d", $time, u_leaf.n, u_leaf.get());
      initial #(5+r*2+d) u_leaf.report();
    end
  end
endmodule"#;
    let sim = simulate(SRC, 100).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T| 1 n=0 fn=0",
            "T| 2 n=1 fn=1",
            "T| 3 n=10 fn=10",
            "T| 4 n=11 fn=11",
            "T| 5 leaf 0",
            "T| 6 leaf 1",
            "T| 7 leaf 10",
            "T| 8 leaf 11",
        ],
    );
}
