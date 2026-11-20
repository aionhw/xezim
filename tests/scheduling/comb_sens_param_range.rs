//! §9.2.2.2.1: an `always_comb` is sensitive to the longest static prefix of
//! each select it reads, excluding only what it writes. In a module instance,
//! a block that reads one bit of a vector whose range depends on a parameter
//! (`logic [P+1:0] b`) and writes another bit of it never ran again after time
//! zero: the sensitivity code sized the vector with the top module's
//! parameters, where the instance's `P` is not visible, so it came out one bit
//! wide, the read fell outside it and counted as a whole-vector read, and the
//! block's own element write excluded that. Caliptra's
//! `caliptra_prim_arbiter_fixed` builds its priority tree that way (§27): the
//! node blocks never saw their leaves, and the arbiter's index stayed x.

use xezim::simulate;

fn run(src: &str) -> (Vec<String>, u64) {
    let sim = simulate(src, 100).expect("simulate failed");
    let msgs = sim.output.iter().map(|o| o.message.clone()).collect();
    (msgs, sim.settle_limit_report().0)
}

/// Every way to size the vector by the instance's parameter: a sum, a power,
/// a shift, an ascending range, a vector in a generate block of the instance,
/// and an overridden parameter that puts the read above the default range.
/// The literal range is the control that always worked.
#[test]
fn always_comb_reading_one_bit_of_a_parameter_sized_vector_in_an_instance_wakes() {
    let (msgs, hits) = run(r#"
module m #(parameter int P = 1) (input logic r, output logic [5:0] o);
  logic [P+1:0]            b;  // [2:0] by a sum
  logic [2**(P+1)-2:0]     c;  // [2:0] by a power
  logic [(1<<(P+1))-2:0]   d;  // [2:0] by a shift
  logic [0:P+1]            e;  // ascending
  logic [2:0]              f;  // literal (control)
  assign b[1] = r; assign c[1] = r; assign d[1] = r; assign e[1] = r; assign f[1] = r;
  always_comb b[0] = b[1];
  always_comb c[0] = c[1];
  always_comb d[0] = d[1];
  always_comb e[0] = e[1];
  always_comb f[0] = f[1];
  // A vector declared in a generate block of the instance, sized by the module's parameter.
  if (1) begin : g
    logic [P:0] v;
    assign v[1] = r;
    always_comb v[0] = v[1];
  end
  assign o = {b[0], c[0], d[0], e[0], f[0], g.v[0]};
endmodule
// An overridden parameter: the element read sits above the default range.
module w #(parameter int P = 1) (input logic r, output logic o);
  logic [P:0] b;
  assign b[P] = r;
  always_comb b[0] = b[P];
  assign o = b[0];
endmodule
module top;
  logic r = 0;
  logic [5:0] o; logic ow;
  m u (.r, .o);
  w #(.P(5)) uw (.r, .o(ow));
  initial begin
    #1 $display("o=%b ow=%b", o, ow);
    r = 1; #1 $display("o=%b ow=%b", o, ow);
    r = 0; #1 $display("o=%b ow=%b", o, ow);
  end
endmodule
"#);
    assert_eq!(msgs, ["o=000000 ow=0", "o=111111 ow=1", "o=000000 ow=0"]);
    assert_eq!(hits, 0);
}

/// The arbiter tree, for two and four requesters, starting from x requests,
/// beside the same two-requester tree written out without generate blocks or
/// parameters. Verilator prints the same lines.
#[test]
fn generate_tree_of_always_comb_nodes_follows_its_leaf_assignments() {
    let (msgs, hits) = run(r#"
// Caliptra caliptra_prim_arbiter_fixed's tree: leaves driven by continuous assignments in one
// generate branch, nodes as always_comb blocks in its sibling branch.
module arb #(parameter int N = 2, localparam int IdxW = $clog2(N))
  (input [N-1:0] req_i, input ready_i, output logic [N-1:0] gnt_o,
   output logic [IdxW-1:0] idx_o, output logic valid_o);
  logic [2**(IdxW+1)-2:0]           req_tree;
  logic [2**(IdxW+1)-2:0]           gnt_tree;
  logic [2**(IdxW+1)-2:0][IdxW-1:0] idx_tree;
  for (genvar level = 0; level < IdxW+1; level++) begin : gen_tree
    localparam int Base0 = (2**level)-1;
    localparam int Base1 = (2**(level+1))-1;
    for (genvar offset = 0; offset < 2**level; offset++) begin : gen_level
      localparam int Pa = Base0 + offset;
      localparam int C0 = Base1 + 2*offset;
      localparam int C1 = Base1 + 2*offset + 1;
      if (level == IdxW) begin : gen_leafs
        assign req_tree[Pa] = req_i[offset];
        assign idx_tree[Pa] = offset;
        assign gnt_o[offset] = gnt_tree[Pa];
      end else begin : gen_nodes
        logic sel;
        always_comb begin
          sel = ~req_tree[C0];
          req_tree[Pa] = req_tree[C0] | req_tree[C1];
          idx_tree[Pa] = sel ? idx_tree[C1] : idx_tree[C0];
          gnt_tree[C0] = gnt_tree[Pa] & ~sel;
          gnt_tree[C1] = gnt_tree[Pa] & sel;
        end
      end
    end
  end
  assign idx_o = idx_tree[0];
  assign valid_o = req_tree[0];
  assign gnt_tree[0] = valid_o & ready_i;
endmodule
// The same N = 2 tree written out without generate blocks or parameters (control).
module arbflat (input [1:0] req_i, input ready_i, output logic [1:0] gnt_o,
                output logic idx_o, output logic valid_o);
  logic [2:0] req_tree, gnt_tree; logic [2:0][0:0] idx_tree; logic sel;
  assign req_tree[1] = req_i[0];
  assign req_tree[2] = req_i[1];
  assign idx_tree[1] = 0;
  assign idx_tree[2] = 1;
  assign gnt_o[0] = gnt_tree[1];
  assign gnt_o[1] = gnt_tree[2];
  always_comb begin
    sel = ~req_tree[1];
    req_tree[0] = req_tree[1] | req_tree[2];
    idx_tree[0] = sel ? idx_tree[2] : idx_tree[1];
    gnt_tree[1] = gnt_tree[0] & ~sel;
    gnt_tree[2] = gnt_tree[0] & sel;
  end
  assign idx_o = idx_tree[0];
  assign valid_o = req_tree[0];
  assign gnt_tree[0] = valid_o & ready_i;
endmodule
module top;
  logic [1:0] r2 = 'x; logic [3:0] r4 = 'x; logic ready = 1;
  logic [1:0] g2, gf; logic i2, if_; logic v2, vf;
  logic [3:0] g4; logic [1:0] i4; logic v4;
  arb #(.N(2)) a2 (.req_i(r2), .ready_i(ready), .gnt_o(g2), .idx_o(i2), .valid_o(v2));
  arbflat af (.req_i(r2), .ready_i(ready), .gnt_o(gf), .idx_o(if_), .valid_o(vf));
  arb #(.N(4)) a4 (.req_i(r4), .ready_i(ready), .gnt_o(g4), .idx_o(i4), .valid_o(v4));
  task show(); $display("r2=%b gen:%b,%b,%b flat:%b,%b,%b | r4=%b %b,%b,%b", r2, i2, v2, g2, if_, vf, gf, r4, i4, v4, g4); endtask
  initial begin
    #1 r2 = 2'b00; r4 = 4'b0000; #1 show();
    r2 = 2'b01; r4 = 4'b0100; #1 show();
    r2 = 2'b10; r4 = 4'b1010; #1 show();
    r2 = 2'b11; r4 = 4'b1000; #1 show();
  end
endmodule
"#);
    assert_eq!(
        msgs,
        [
            "r2=00 gen:1,0,00 flat:1,0,00 | r4=0000 11,0,0000",
            "r2=01 gen:0,1,01 flat:0,1,01 | r4=0100 10,1,0100",
            "r2=10 gen:1,1,10 flat:1,1,10 | r4=1010 01,1,0010",
            "r2=11 gen:0,1,01 flat:0,1,01 | r4=1000 11,1,1000",
        ]
    );
    assert_eq!(hits, 0);
}

/// The fix keeps the exclusion: a block that writes the whole
/// parameter-sized vector it reads does not re-run itself.
#[test]
fn always_comb_writing_the_whole_parameter_sized_vector_it_reads_settles() {
    let (msgs, hits) = run(r#"
// A block that writes the whole parameter-sized vector it reads stays out of its own list.
module m #(parameter int P = 1) (input logic [P+1:0] a, output logic [P+1:0] y);
  logic [P+1:0] b;
  always_comb begin b = a; b[0] = b[1] ^ b[2]; end
  assign y = b;
endmodule
module top;
  logic [2:0] a; logic [2:0] y;
  m u (.a, .y);
  initial begin
    a = 3'b010; #1 $display("y=%b", y);
    a = 3'b110; #1 $display("y=%b", y);
  end
endmodule
"#);
    assert_eq!(msgs, ["y=011", "y=110"]);
    assert_eq!(hits, 0);
}
