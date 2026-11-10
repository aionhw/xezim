//! §7.2.1, §18.5, §11.8.1: a signed member of a packed struct class property
//! (`bit signed [3:0] s`) reads signed, and the solver reads it signed: `p.s
//! inside {[-4:-1]}` and `p.s < 0` are satisfiable. The member read as its
//! unsigned bits through the property (`o.p.s` was 13 for 4'hd), and the
//! solver took every field select unsigned, so `randomize()` returned 0.
//! Expected lines are the reference simulator's.

use xezim::simulate;

#[test]
fn signed_members_read_and_solve_signed() {
    let src = r#"
typedef struct packed { bit signed [3:0] s; bit [3:0] u; } p_t;
typedef struct packed { logic signed [3:0] s; logic [3:0] u; } pl_t;
typedef struct packed { byte b; bit [7:0] u; } pb_t;
class c;
  rand p_t p;
  rand pl_t l;
  rand pb_t b;
  constraint k1 { p.s inside {[-4:-1]}; }
  constraint k2 { l.s < 0; l.s > -3; }
  constraint k3 { b.b inside {[-100:-90]}; }
  constraint k4 { b.u dist { 8'd1 := 1, 8'd2 := 1 }; p.s + 4'sd1 < l.s + b.u; }
  constraint k6 { p.u[3] == 1'b1; }
  function bit check();
    return p.s inside {[-4:-1]} && l.s < 0 && l.s > -3 && b.b inside {[-100:-90]}
           && b.u inside {1, 2} && p.s + 4'sd1 < l.s + b.u && p.u[3];
  endfunction
  function void show();
    $display("read p.s=%0d lt=%0b this=%0d", p.s, p.s < 0, this.p.s);
  endfunction
endclass
module top;
  initial begin
    c o = new;
    int ok = 0, good = 0;
    repeat (20) begin ok += o.randomize(); good += o.check(); end
    $display("signed members ok=%0d good=%0d", ok, good);
    o.p = 8'hd0;
    o.show();
    $display("read o.p.s=%0d lt=%0b sel=%0d", o.p.s, o.p.s < 0, o.p.s[3]);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "signed members ok=20 good=20",
            "read p.s=-3 lt=1 this=-3",
            "read o.p.s=-3 lt=1 sel=1",
        ]
    );
}
