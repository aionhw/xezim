//! §23.10 / §7.4.5: a packed declaration inside an instance is sized by that
//! instance's parameters, overlaid on the design-wide table. The overlay is
//! swapped between instances rather than rebuilt, so these selects move
//! between two instances of one module with different `N`, and a third
//! module whose declaration names the compilation-unit `N` that the
//! instances' own `N` shadows. The last block selects with an index read
//! from another instance's array while the outer select still holds its own
//! instance's parameters. The second test moves between a module and its
//! generate blocks, which shadow the module's `N` with their own values: the
//! module's scope stays applied across those moves, and taking a block's
//! scope off must leave the module's `N` again. Expected values agree with
//! Verilator 5.046.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const SRC: &str = "localparam int N = 3;
typedef struct packed { logic a; logic [3:0] d; logic u; } s_t;

module leaf #(parameter int N = 2) ();
  s_t [N-1:0] par;
endmodule

module other #(parameter int P = 5) ();
  s_t [N-1:0] q;
  logic [P-1:0] pad;
endmodule

module top;
  leaf #(.N(2)) u1 ();
  leaf #(.N(4)) u2 ();
  other o ();
  initial begin
    u1.par = '0; u2.par = '0; o.q = '0;
    for (int k = 0; k < 3; k++) begin
      u1.par[1].a = 1'b1;
      o.q[2].d = 4'(k + 9);
      u2.par[3].d = 4'(k + 5);
      u1.par[0].u = k[0];
      o.q[0].a = 1'b1;
      u2.par[0].u = 1'b1;
    end
    $display(\"u1=%b u2=%b o=%b\", u1.par, u2.par, o.q);
    $display(\"r u1a1=%b u2d3=%h oq2d=%h bits=%0d/%0d/%0d\", u1.par[1].a, u2.par[3].d, o.q[2].d,
             $bits(u1.par), $bits(u2.par), $bits(o.q));
    u2.par[1].d = 4'd1;
    o.q[1].d = 4'd2;
    u1.par[u2.par[1].d].d = 4'hc;
    u2.par[o.q[1].d].d = 4'h7;
    $display(\"x u1=%b u2=%b\", u1.par, u2.par);
    $display(\"y %h %h %h\", u1.par[u2.par[1].d].d, u2.par[o.q[1].d].d, o.q[u1.par[1].a].d);
    $finish;
  end
endmodule";

#[test]
fn instance_parameter_overlays_swap_between_instances() {
    let msgs = messages(SRC);
    let want = [
        "u1=100000000000 u2=001110000000000000000001 o=010110000000100000",
        "r u1a1=1 u2d3=7 oq2d=b bits=12/24/18",
        "x u1=111000000000 u2=001110001110000010000001",
        "y c 7 2",
    ];
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:?}");
    }
}

const NESTED: &str = "typedef struct packed { logic a; logic [3:0] d; logic u; } s_t;

module leafg #(parameter int N = 2) ();
  s_t [N-1:0] par;
  initial par = '0;
  for (genvar g = 0; g < 2; g++) begin : gb
    localparam int N = 3 + g;
    s_t [0:N-1] q;
    initial begin
      q = '0;
      for (int k = 0; k < 3; k++) begin
        #1;
        q[N-1].d = 4'(k + 4 * g);
        par[g].d = 4'(k + 8);
        q[0].a = 1'b1;
        par[g].u = q[N-1].d[0];
        q[1].u = par[g].u;
      end
      $display(\"%m q=%b n=%0d d=%h u=%b bits=%0d\", q, N, q[N-1].d, q[1].u, $bits(q));
    end
  end
  initial begin
    #10 $display(\"%m par=%b d1=%h u0=%b bits=%0d\", par, par[1].d, par[0].u, $bits(par));
  end
endmodule

module top;
  leafg #(.N(2)) u ();
  leafg #(.N(3)) v ();
endmodule";

#[test]
fn generate_block_parameters_shadow_and_restore_the_module_parameter() {
    let sim = simulate(NESTED, 1_000).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let want = [
        "top.u.gb[0] q=100000000000000100 n=3 d=2 u=0 bits=18",
        "top.u.gb[1] q=100000000000000000001100 n=4 d=6 u=0 bits=24",
        "top.u par=010100010100 d1=a u0=0 bits=12",
        "top.v.gb[0] q=100000000000000100 n=3 d=2 u=0 bits=18",
        "top.v.gb[1] q=100000000000000000001100 n=4 d=6 u=0 bits=24",
        "top.v par=000000010100010100 d1=a u0=0 bits=18",
    ];
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:?}");
    }
}
