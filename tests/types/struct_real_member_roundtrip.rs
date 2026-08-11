//! A `real` member of an unpacked struct must survive a WHOLE-struct
//! assignment. Packing stores `f64::to_bits()`; the spread path recovered it
//! with an integer cast (`to_u64() as f64`), so 4.0 came back as
//! 4.6161896e18 — the raw IEEE-754 bit pattern read as an integer.
//! Reference-validated.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn real_member_survives_whole_struct_assignment() {
    // Member-wise assignment always worked; the two whole-struct forms
    // (function return, assignment pattern) are the ones that went through
    // the spread path.
    let src = r#"
module tb;
  typedef struct { real r; int i; } s_t;
  s_t direct, pattern, returned;
  function automatic s_t mk();
    s_t t;
    t.r = 4.0;
    t.i = 7;
    return t;
  endfunction
  initial begin
    direct.r = 4.0; direct.i = 7;
    pattern  = '{r: 4.0, i: 7};
    returned = mk();
    #1 $display("T|%f %0d|%f %0d|%f %0d",
                direct.r, direct.i, pattern.r, pattern.i, returned.r, returned.i);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        msgs(&sim)
            .iter()
            .any(|m| m == "T|4.000000 7|4.000000 7|4.000000 7"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn negative_and_fractional_real_members_round_trip() {
    // A fractional value cannot survive an integer cast at all, so this
    // pins the representation rather than just the happy path.
    let src = r#"
module tb;
  typedef struct { real a; real b; int i; } s_t;
  s_t s;
  function automatic s_t mk();
    s_t t;
    t.a = -2.5;
    t.b = 0.125;
    t.i = -3;
    return t;
  endfunction
  initial begin
    s = mk();
    #1 $display("T|%f %f %0d", s.a, s.b, s.i);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|-2.500000 0.125000 -3"),
        "got {:?}",
        msgs(&sim)
    );
}
