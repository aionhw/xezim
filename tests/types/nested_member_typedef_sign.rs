//! §7.2.1, §11.8.1: a member of a packed struct keeps its own declared
//! type's sign, also below a member whose type is a typedef (`ti.m.a.v` with
//! `mid_t m; leaf_t a;`). The interpreter's read looked for the leaf name only
//! inside members declared as inline structs, so `ti.m.a.v` read unsigned in
//! an initial block (11 for -5) while a compiled `always_comb` read -5. The
//! expected values are the reference simulator's.

#[test]
fn a_nested_typedef_member_reads_with_its_declared_sign() {
    let src = r#"
module top;
  typedef struct packed { logic signed [3:0] v; logic [3:0] u; } leaf_t;
  typedef struct packed { leaf_t a; leaf_t b; } mid_t;
  typedef struct packed { mid_t m; logic [7:0] tail; } top_t;
  top_t ti;
  int o1, o2, o3;
  always_comb o1 = ti.m.a.v;
  initial begin
    ti = '{m: '{a: '{v: -4'sd5, u: 4'd9}, b: '{v: -4'sd2, u: 4'd7}}, tail: 8'h33};
    #1 o2 = ti.m.a.v;
    o3 = ti.m.b.v + 0;
    $display("o1=%0d o2=%0d o3=%0d u=%0d", o1, o2, o3, ti.m.b.u);
  end
endmodule
"#;
    let got: Vec<String> = xezim::simulate(src, 1_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_eq!(got, ["o1=-5 o2=-5 o3=-2 u=7"]);
}
