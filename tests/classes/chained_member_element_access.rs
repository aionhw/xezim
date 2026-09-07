//! §7.4.2/§8.23: element access on a multi-dim array member reached through a
//! CHAIN of class handles (`o.sub.g[0][1]`).
//!
//! Elaboration flattens a member chain into one multi-segment Ident, and the
//! two resolvers that own member element access each stopped one hop short:
//!
//! * `class_nd_elem_name` matched 1- and 2-segment receivers only, so a
//!   3+-segment path fell to its `_` arm — chained multi-dim element WRITES
//!   landed nowhere and READS came back x, while a temp handle
//!   (`t = o.sub; t.g[0][1]`) against the same storage worked (which is how
//!   it hid). It now walks the middle segments through heap properties; for
//!   two segments the walk is empty and behavior is unchanged.
//! * `instance_assoc_member` rejected every dotted name, so the fallbacks it
//!   feeds (queue/assoc members among them) resolved nothing for chains. It
//!   now walks 3+-segment handle chains, guarded by a final class-membership
//!   check so a module hierarchical path can never be captured.
//!
//! 1-D members through a chain and multi-dim members through ONE hop always
//! worked — only chain x multi-dim failed. Every expectation below is the
//! reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn chained_handle_multidim_elements_read_and_write() {
    let o = out(r#"
module top;
  class inner_c;
    int g[2][3];
    function void wr(int i, int j, int v); g[i][j] = v; endfunction
    function int  rd(int i, int j); return g[i][j]; endfunction
  endclass
  class c; inner_c sub; function new(); sub = new(); endfunction endclass
  initial begin
    c o = new();
    o.sub.g[0][1] = 7;                       // chained write ...
    $display("A=%0d", o.sub.rd(0, 1));       // ... seen by a method read
    o.sub.wr(1, 2, 9);                       // method write ...
    $display("B=%0d", o.sub.g[1][2]);        // ... seen by a chained read
    o.sub.g[1][0] = 4;
    $display("C=%0d", o.sub.g[1][0]);        // both chained
    $display("D=%0d", o.sub.rd(0, 0));       // untouched element stays 0
  end
endmodule
"#);
    for expect in ["A=7", "B=9", "C=4", "D=0"] {
        assert!(o.contains(expect), "expected {expect} in:\n{o}");
    }
}

#[test]
fn deeper_chains_and_collection_members_resolve() {
    let o = out(r#"
module top;
  class leaf_c; int g[2][2]; int g3[2][2][2]; int q[$]; int m[string]; endclass
  class mid_c; leaf_c leaf; function new(); leaf = new(); endfunction endclass
  class root_c; mid_c mid; function new(); mid = new(); endfunction endclass
  initial begin
    root_c r = new();
    r.mid.leaf.g[1][1] = 42;                 // THREE-hop chain, 2-D
    r.mid.leaf.g3[1][0][1] = 17;             // two-hop chain, 3-D
    r.mid.leaf.q.push_back(5);
    r.mid.leaf.q.push_back(6);               // chained queue member
    r.mid.leaf.m["k"] = 8;                   // chained assoc member
    $display("H3=%0d D3=%0d Q=%0d M=%0d",
             r.mid.leaf.g[1][1], r.mid.leaf.g3[1][0][1],
             r.mid.leaf.q[1] + r.mid.leaf.q.size(), r.mid.leaf.m["k"]);
  end
endmodule
"#);
    assert!(
        o.contains("H3=42 D3=17 Q=8 M=8"),
        "deep chain / collection member access wrong:\n{o}"
    );
}
