//! Inside an inlined instance, a subroutine formal, a subroutine local or a
//! block-local declaration shadows the module's own names (§6.21): the
//! inliner prefixed every use whose head matched a module-level variable or
//! a sub-instance, so `begin int u; u = 5; end` clobbered the module-level
//! `u`, a task-local `core` next to an instance `core` read `core.c` as x,
//! and a block-local handle named like the enclosing instance read null.
//! (The `u = new(...)` form of that last shape is still open: the constructor
//! path looks the target's type up through instance-path name resolution.)
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn instance_block_and_subroutine_locals_shadow_module_names() {
    let msgs = messages(
        "package p; class R; int c; function new(int x); c = x; endfunction endclass endpackage
module core_m(input logic clk); int seq = 5; endmodule
module host(input logic clk); import p::*; int u = 9; core_m core(.clk(clk));
  task automatic t_local(); R core; core = new(11); $display(\"sib=%0d\", core.c); endtask
  task automatic t_formal(int u); u = u + 1; $display(\"formal=%0d\", u); endtask
  initial begin
    begin int u; u = 5; $display(\"blk=%0d\", u); end
    $display(\"mod=%0d\", u);
    begin R u = new(3); $display(\"hnull=%0d hc=%0d\", u == null, u.c); end
    begin R u; R t; t = new(4); u = t; $display(\"hcopy=%0d\", u.c); end
    t_local(); t_formal(1);
    $display(\"mod2=%0d\", u);
  end
endmodule
module tb; logic clk = 0; host u(.clk(clk)); initial #5 $finish; endmodule",
    );
    for want in ["blk=5", "mod=9", "hnull=0 hc=3", "hcopy=4", "sib=11", "formal=2", "mod2=9"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
}
