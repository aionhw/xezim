//! IEEE 1800-2023 §23.8 (upward name referencing): a hierarchical name whose
//! first segment is not declared in the current scope is resolved scope by
//! scope upward — as the INSTANCE name of a child of an enclosing scope, or
//! as the DEFINITION name of one (`node1.req1` from a module bound into
//! `u_node1`, an instance of `node1`: "the nearest enclosing instance of
//! module node1"). xezim bound such names to a same-named signal of the top
//! module instead, in continuous assigns, procedural blocks, function bodies,
//! event controls, generate blocks and on the left-hand side of stores
//! (aionhw/xezim#890, reported by Ganesh-TNL). Expected values are the
//! reference simulator's.
use xezim::simulate_multi;

fn check(source: &str, expected: &[&str]) {
    let sim = simulate_multi(
        &[source.to_string()],
        200,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
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
    )
    .expect("simulate");
    let actual: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(actual, expected);
}

/// Two instances of the bind target, each with a watcher: each watcher reads
/// ITS target's unconnected port and net by the target's module name; the
/// top-level baits of the same names stay out of it (S1, C21 of #890).
#[test]
fn bound_watcher_names_its_own_target_by_module_name() {
    check(
        r#"
module node (input logic clk, input logic req);
  logic [7:0] tag;
endmodule
module watch (input logic clk);
  logic [7:0] seen;
  assign seen = node.tag;
  int hits = 0;
  always @(posedge clk) if (node.req) hits++;
endmodule
bind node watch u_watch (.clk(clk));
module top;
  logic clk = 0;
  logic req = 1;            // bait at top
  logic [7:0] tag = 8'hEE;  // bait at top
  logic ra = 0, rb = 0;
  node a (.clk(clk), .req(ra));
  node b (.clk(clk), .req(rb));
  initial begin
    a.tag = 8'hA1; b.tag = 8'hB2;
    #1 $display("T|a.seen=%h b.seen=%h", a.u_watch.seen, b.u_watch.seen);
    ra = 1; clk = 1; #1 clk = 0; #1
    $display("T|hits a=%0d b=%0d", a.u_watch.hits, b.u_watch.hits);
    ra = 0; rb = 1; clk = 1; #1 clk = 0; #1
    $display("T|hits a=%0d b=%0d", a.u_watch.hits, b.u_watch.hits);
  end
endmodule
"#,
        &["T|a.seen=a1 b.seen=b2", "T|hits a=1 b=0", "T|hits a=1 b=1"],
    );
}

/// The target sits in a generate loop, or two levels down a wrapper whose
/// port actuals are unconnected: still the nearest enclosing instance (S3).
#[test]
fn bound_watcher_under_generate_and_wrapper_scopes() {
    check(
        r#"
module node (input logic clk, input logic req);
  int v;
endmodule
module watch (input logic clk);
  int seen;
  logic r;
  assign seen = node.v;
  assign r = node.req;
endmodule
bind node watch u_watch (.clk(clk));
module wrap (input logic clk);
  node u_node (.clk(clk), .req());
endmodule
module top;
  logic clk = 0;
  int v = 99;     // bait
  logic req = 1;  // bait
  for (genvar i = 0; i < 3; i++) begin : g
    node u_node (.clk(clk), .req());
    initial u_node.v = 10 + i;
  end
  wrap u_wrap (.clk(clk));
  initial begin
    u_wrap.u_node.v = 77;
    #1 $display("T|seen0=%0d seen1=%0d seen2=%0d wrap=%0d r=%b",
                g[0].u_node.u_watch.seen, g[1].u_node.u_watch.seen,
                g[2].u_node.u_watch.seen, u_wrap.u_node.u_watch.seen,
                u_wrap.u_node.u_watch.r);
  end
endmodule
"#,
        &["T|seen0=10 seen1=11 seen2=12 wrap=77 r=z"],
    );
}

/// Plain (non-bind) child: module-name head (§23.8 case b) and instance-name
/// head (case a) both read the parent's signal, not the top bait (C20).
#[test]
fn plain_child_module_and_instance_name_heads() {
    check(
        r#"
module leaf20 (output logic yb, output logic ya);
  assign yb = mid20.sig;
  assign ya = u_mid20.sig;
endmodule
module mid20;
  logic sig = 1'b0;
  logic yb, ya;
  leaf20 u_leaf20 (.yb(yb), .ya(ya));
endmodule
module top;
  mid20 u_mid20 ();
  logic sig = 1'b0;  // bait
  initial begin
    #1 u_mid20.sig = 1'b1;
    #1 $display("T|yb=%b ya=%b", u_mid20.yb, u_mid20.ya);
  end
endmodule
"#,
        &["T|yb=1 ya=1"],
    );
}

/// Upward names in every procedural shape of a bound module: a procedural
/// read, a function body, an event control, a generate block, and a
/// continuous-assign LHS driving the target's net (C21, C22, C24, C25, C28).
#[test]
fn upward_names_in_procedural_shapes_of_a_bound_module() {
    check(
        r#"
module n (input logic clk, input logic req);
  logic tgt;
endmodule
module w (input logic clk);
  int hits = 0;
  always @(posedge clk) if (n.req) hits++;           // procedural read
  function automatic logic peek();
    return n.req;                                    // function body
  endfunction
  logic y = 1'b0;
  always @(posedge clk) y <= peek();
  int edges = 0;
  always @(n.req) edges++;                           // event control
  logic g;
  if (1) begin : gen
    assign g = n.req;                                // generate block
  end
  assign n.tgt = 1'b1;                               // LHS into the target
endmodule
bind n w u_w (.clk(clk));
module top;
  logic clk = 0;
  logic req = 1'b1;   // bait, constant 1
  logic tgt = 1'b0;   // bait, must stay 0
  logic drv = 1'b0;
  n u_n (.clk(clk), .req());
  assign u_n.req = drv;
  initial begin
    #1 $display("T|t1 tgt=%b top_tgt=%b g=%b edges=%0d", u_n.tgt, tgt, u_n.u_w.g, u_n.u_w.edges);
    clk = 1; #1 clk = 0; #1
    $display("T|t3 hits=%0d y=%b", u_n.u_w.hits, u_n.u_w.y);
    drv = 1; #1
    $display("T|t4 g=%b edges=%0d", u_n.u_w.g, u_n.u_w.edges);
    clk = 1; #1 clk = 0; #1
    $display("T|t6 hits=%0d y=%b", u_n.u_w.hits, u_n.u_w.y);
    drv = 0; #1
    $display("T|t7 edges=%0d", u_n.u_w.edges);
  end
endmodule
"#,
        &[
            "T|t1 tgt=1 top_tgt=0 g=0 edges=1",
            "T|t3 hits=0 y=0",
            "T|t4 g=1 edges=2",
            "T|t6 hits=1 y=1",
            "T|t7 edges=3",
        ],
    );
}

/// Heads that name a sibling of the bind target by instance name (C23), an
/// interface instance declared in the target (C27), the module of an
/// ancestor two levels up, a generate label of an ancestor, and a module
/// name that is both an ancestor's module and an instance name further up
/// (the nearest enclosing instance of that module wins).
#[test]
fn instance_and_module_name_heads_in_enclosing_scopes() {
    check(
        r#"
interface tic_if;
  logic member = 1'b1;
endinterface
module sib;
  logic s = 1'b1;
endmodule
module k;
  int o = 1;
endmodule
module n;
  tic_if tic ();
endmodule
module w;
  logic y, t;
  int o, g;
  assign y = u_sib.s;      // sibling of the target, by instance name
  assign t = tic.member;   // interface instance in the target
  assign o = outer.o;      // module of an ancestor two levels up (also an instance name at top)
  assign g = gb.v;         // generate label of an ancestor
endmodule
bind n w u_w ();
module outer;
  int o = 100;
  if (1) begin : gb
    int v = 42;
  end
  sib u_sib ();
  n   u_n ();
endmodule
module top;
  logic s = 1'b0, member = 1'b0;   // baits
  int o = 2000, v = 5;             // baits
  k outer ();                      // instance named like module outer
  outer u_outer ();
  initial #1 $display("T|y=%b t=%b o=%0d g=%0d", u_outer.u_n.u_w.y,
                      u_outer.u_n.u_w.t, u_outer.u_n.u_w.o, u_outer.u_n.u_w.g);
endmodule
"#,
        &["T|y=1 t=1 o=100 g=42"],
    );
}

/// Stores through an upward name land on the target's objects: a class
/// handle, a class field and a class associative element through that
/// handle, a plain variable, a generate-block variable and an associative
/// element (C41, C42, C43 of #890, with integral values).
#[test]
fn upward_stores_land_on_the_target() {
    check(
        r#"
class Obj;
  int id;
  int arr[int];
  function new(int i); id = i; endfunction
endclass
module n;
  Obj obj;
  int gv;
  int aa[int];
  for (genvar gi = 0; gi < 2; gi++) begin : gb
    int gvar;
    initial gvar = 3;
  end
endmodule
module w;
  initial begin
    n.obj = new(1);
    n.obj.id = 15152;
    n.obj.arr[1] = 15152;
    n.gv = 15152;
    n.gb[1].gvar = 15152;
    n.aa[1] = 15152;
  end
endmodule
bind n w u_w ();
module top;
  int gv = 0, gvar = 0;  // baits
  n u_n ();
  initial #1 $display("T|id=%0d arr=%0d gv=%0d gb0=%0d gb1=%0d aa=%0d top_gv=%0d",
                      u_n.obj.id, u_n.obj.arr[1], u_n.gv, u_n.gb[0].gvar,
                      u_n.gb[1].gvar, u_n.aa[1], gv);
endmodule
"#,
        &["T|id=15152 arr=15152 gv=15152 gb0=3 gb1=15152 aa=15152 top_gv=0"],
    );
}

/// A class collection reached through a handle named by an instance path —
/// `u_n.obj.arr[k]` from the top, or the upward `n.obj.arr[k]` from a bound
/// module — is the target object's collection: the head of such a path is
/// an instance (whose placeholder signal is not a handle), so the walk used
/// to start from null and every element read x and every store vanished
/// (C41 CLASSOC of #890, also downward).
#[test]
fn class_collections_through_a_hierarchical_handle() {
    check(
        r#"
class Obj;
  int id;
  int arr[int];
  function new(int i); id = i; endfunction
endclass
module n;
  Obj obj;
  initial begin
    obj = new(1);
    obj.arr[2] = 7;
    #1 $display("T|local arr2=%0d size=%0d", obj.arr[2], obj.arr.size());
  end
endmodule
module top;
  Obj tobj;
  n u_n ();
  initial begin
    tobj = new(1); tobj.arr[2] = 8;
    u_n.obj.arr[4] = 11;
    #1 $display("T|top arr2=%0d arr4=%0d size=%0d tobj2=%0d", u_n.obj.arr[2],
                u_n.obj.arr[4], u_n.obj.arr.size(), tobj.arr[2]);
    #1 $display("T|exists2=%0d exists4=%0d", u_n.obj.arr.exists(2), u_n.obj.arr.exists(4));
  end
endmodule
"#,
        &[
            "T|local arr2=7 size=2",
            "T|top arr2=7 arr4=11 size=2 tobj2=8",
            "T|exists2=1 exists4=1",
        ],
    );
}
