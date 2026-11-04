//! §18.3: the solver shall not assign a rand enum variable a value outside
//! its named constants. An enum member of a rand unpacked struct declared
//! in a package drew any bit pattern of the enum's width.
#[test]
fn package_struct_enum_member_takes_enum_values() {
    let result = xezim::simulate(
        r#"
package p;
  typedef enum bit [2:0] { M_A = 1, M_B = 5 } mode_t;
  typedef struct {
    rand bit [3:0] x;
    rand mode_t    mode;
  } cfg_t;
  class free_c;
    rand cfg_t s;
  endclass
  class tied_c;
    rand cfg_t s;
    constraint k { (s.mode == M_B) -> (s.x > 3); }
  endclass
endpackage
module t;
  import p::*;
  initial begin
    free_c f = new();
    tied_c c = new();
    int ok_f = 0, bad_f = 0, ok_c = 0, bad_c = 0;
    repeat (20) begin
      if (f.randomize()) begin
        ok_f++;
        bad_f += !(f.s.mode inside {M_A, M_B});
      end
      if (c.randomize()) begin
        ok_c++;
        bad_c += !(c.s.mode inside {M_A, M_B}) || (c.s.mode == M_B && c.s.x <= 3);
      end
    end
    $display("free ok %0d bad %0d", ok_f, bad_f);
    $display("tied ok %0d bad %0d", ok_c, bad_c);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["free ok 20 bad 0", "tied ok 20 bad 0"]);
}
