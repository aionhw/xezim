//! `w.r.c` (a property of an object reached through a handle held by another
//! object) read x when `w` was a TASK local inside a sub-instance; the same
//! chain from a top-level task, from an initial block of the instance, or
//! split into two steps (`r2 = w.r; r2.c`) was fine. The mailbox-delivered
//! wake object of a UVM-style BFM host is exactly this shape.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn handle_chain_reads_through_a_task_local_in_a_sub_instance() {
    let msgs = messages(
        "package p;
  class R; int c; function new(int x); c = x; endfunction endclass
  class W; R r; function new(R rr); r = rr; endfunction endclass
  class B; mailbox #(W) mb; function new(); mb = new(); endfunction
    task put_it(R r); W w; w = new(r); mb.put(w); endtask endclass
endpackage
module core_m(input logic clk); int seq = 5; endmodule
module host(input logic clk); import p::*; B b = new(); core_m core(.clk(clk));
  task automatic t_sibling_name(); R core; core = new(11); $display(\"sibling_name=%0d\", core.c); endtask
  task automatic t_plain(); R r; W w; r = new(7); w = new(r); $display(\"plain=%0d\", w.r.c); endtask
  task t_static(); R r; W w; r = new(8); w = new(r); $display(\"static=%0d\", w.r.c); endtask
  task automatic t_mb(); R r; W w; r = new(9); b.put_it(r); b.mb.get(w); $display(\"mailbox=%0d\", w.r.c); endtask
  task automatic t_forked(); W w; b.mb.get(w); $display(\"forked=%0d\", w.r.c); endtask
  initial begin t_plain(); t_static(); t_mb(); t_sibling_name(); end
  initial begin R r; #1; fork t_forked(); join_none #1; r = new(10); b.put_it(r); end
endmodule
module tb; logic clk = 0; host u(.clk(clk)); initial #5 $finish; endmodule",
    );
    for want in ["plain=7", "static=8", "mailbox=9", "forked=10", "sibling_name=11"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
}
