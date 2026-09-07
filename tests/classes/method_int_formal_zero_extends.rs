//! A narrow unsigned actual bound to an `int` class-method or constructor
//! formal was stamped signed BEFORE it was widened, so the frame later
//! sign-extended it: `new(2'b10)` and `new(v[25:24])` read -2. Module
//! functions widened first and were fine.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn narrow_actual_to_int_method_formal_zero_extends() {
    let msgs = messages(
        "module tb;
  logic [31:0] v = 32'h1200c000;
  class C2; int f; function new(int f_arg); f = f_arg; endfunction endclass
  class C3; int f; byte b; logic signed [15:0] s16; bit signed [7:0] b8;
    function void set(int f_arg, byte b_arg); f = f_arg; b = b_arg; endfunction
    function void set_vec(logic signed [15:0] x); s16 = x; endfunction
    task set_b8(bit signed [7:0] x); b8 = x; endtask endclass
  C2 c2; C3 c3;
  initial begin
    c2 = new(v[25:24]);  $display(\"ctor=%0d\", c2.f);
    c2 = new(2'b10);     $display(\"lit=%0d\", c2.f);
    c2 = new(.f_arg(v[25:24])); $display(\"named=%0d\", c2.f);
    c3 = new; c3.set(v[25:24], 3'b110); $display(\"method=%0d byte=%0d\", c3.f, c3.b);
    c2 = new(-3); $display(\"neg=%0d\", c2.f);
    c3.set_vec(v[25:24]); c3.set_b8(v[25:24]); $display(\"vec=%0d b8=%0d\", c3.s16, c3.b8);
    c3.set_vec(-7); $display(\"vecneg=%0d\", c3.s16);
    $finish;
  end
endmodule",
    );
    for want in ["ctor=2", "lit=2", "named=2", "method=2 byte=6", "neg=-3", "vec=2 b8=2", "vecneg=-7"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
}
