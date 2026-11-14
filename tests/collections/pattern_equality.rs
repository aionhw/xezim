//! IEEE 1800-2017 §11.4.5 / §10.9: `==`, `!=`, `===` and `!==` between an
//! unpacked aggregate and an ASSIGNMENT PATTERN compare element by element
//! (member by member for a struct). The pattern is in an assignment-like
//! context, so it takes the other operand's type: `index:`, `default:`,
//! nested patterns and the element-type conversion of each item apply.
//! The pattern used to evaluate to a packed concatenation compared with the
//! array's nonexistent container signal, so `a == '{9, 5, 5, 5}` was 0.
//! Expected values are the reference simulator's, except an open relation
//! (x/z elements and no differing one), which is x (§11.4.5).

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

const PATTERN_EQ: &str = r#"
typedef struct { int x; int y; } st_t;
class C;
  int a[4]; int m[2][2]; int d[]; int q[$]; string sa[2]; st_t s;
  function string run();
    a = '{9, 5, 5, 5}; m = '{'{1, 2}, '{3, 4}}; d = '{1, 2, 3}; q = '{7, 8};
    sa = '{"a", "b"}; s = '{1, 2};
    return $sformatf("%0d%0d%0d%0d%0d %0d%0d%0d%0d%0d%0d %0d%0d%0d%0d%0d", a == '{9, 5, 5, 5}, a != '{9, 5, 5, 6},
      a === '{9, 5, 5, 5}, '{9, 5, 5, 5} == a, a != '{9, 5, 5, 5},
      m == '{'{1, 2}, '{3, 4}}, m == '{'{1, 2}, '{3, 5}}, d == '{1, 2, 3}, d == '{1, 2, 4}, q == '{7, 8}, q != '{7, 8},
      sa == '{"a", "b"}, sa == '{"a", "c"}, s == '{1, 2}, s == '{1, 3}, a == '{default: 5});
  endfunction
endclass
module top;
  int a[4]; int m[2][2]; int d[]; int q[$]; string sa[2]; st_t s; int k;
  int ds[3:1]; st_t stv[2]; logic [3:0] lx[2]; bit [7:0] by[3]; int m3[2][2][2];
  initial begin
    C c;
    a = '{9, 5, 5, 5}; m = '{'{1, 2}, '{3, 4}}; d = '{1, 2, 3}; q = '{7, 8};
    sa = '{"a", "b"}; s = '{1, 2};
    $display("T|m1 %0d%0d%0d%0d%0d %0d%0d%0d%0d%0d%0d %0d%0d%0d%0d%0d", a == '{9, 5, 5, 5}, a != '{9, 5, 5, 6},
      a === '{9, 5, 5, 5}, '{9, 5, 5, 5} == a, a != '{9, 5, 5, 5},
      m == '{'{1, 2}, '{3, 4}}, m == '{'{1, 2}, '{3, 5}}, d == '{1, 2, 3}, d == '{1, 2, 4}, q == '{7, 8}, q != '{7, 8},
      sa == '{"a", "b"}, sa == '{"a", "c"}, s == '{1, 2}, s == '{1, 3}, a == '{default: 5});
    if (a == '{9, 5, 5, 5}) $display("T|m2 if yes"); else $display("T|m2 if no");
    k = 0; while (!(a == '{9, 5, 5, 5}) && k < 3) k++;
    $display("T|m3 k=%0d tern=%0d slice=%0d", k, (a == '{9, 5, 5, 5}) ? 11 : 22, a[1:3] == '{5, 5, 5});
    ds = '{1, 2, 3}; stv = '{'{1, 2}, '{3, 4}}; lx = '{4'b1x01, 4'b0011}; by = '{1, 2, 255};
    m3 = '{'{'{1, 2}, '{3, 4}}, '{'{5, 6}, '{7, 8}}};
    $display("T|m4 ds=%0d%0d stv=%0d%0d lx=%0d%0d%0d%0d", ds == '{1, 2, 3}, ds == '{3, 2, 1}, stv == '{'{1, 2}, '{3, 4}}, stv == '{'{1, 2}, '{3, 5}},
      lx == '{4'b1x01, 4'b0011}, lx != '{4'b1x01, 4'b0011}, lx === '{4'b1x01, 4'b0011}, lx !== '{4'b1x01, 4'b0011});
    $display("T|m5 key=%0d%0d by=%0d%0d m3=%0d%0d sn=%0d%0d%0d", a == '{0: 9, default: 5}, a == '{0: 5, default: 9}, by == '{1, 2, 255}, by == '{1, 2, -1},
      m3 == '{'{'{1, 2}, '{3, 4}}, '{'{5, 6}, '{7, 8}}}, m3 == '{default: 1}, s == '{x: 1, y: 2}, s == '{y: 1, x: 2}, s == '{default: 1});
    c = new;
    repeat (3) $display("T|c1 %s", c.run());
  end
endmodule
"#;

#[test]
fn unpacked_aggregates_compare_with_assignment_patterns() {
    let _ = super::compiled_method_test_env::eager();
    let want = [
        "T|m1 11110 101010 10100",
        "T|m2 if yes",
        "T|m3 k=0 tern=11 slice=1",
        // §11.4.5: `lx` holds an x bit no other element contradicts, so
        // `==` and `!=` are x (the reference simulator prints 0 for both).
        "T|m4 ds=10 stv=10 lx=xx10",
        "T|m5 key=10 by=11 m3=10 sn=100",
        "T|c1 11110 101010 10100",
        "T|c1 11110 101010 10100",
        "T|c1 11110 101010 10100",
    ];
    assert_eq!(t_lines(PATTERN_EQ), want);
}

const CONT_ASSIGN: &str = r#"
module top;
  logic [3:0] cc[2]; int cd[2];
  assign cc = '{4'd3, 4'd4};
  assign cd = '{5, 6};
  initial #1 $display("T|ca %0d %0d %0d %0d %0d", cc[0], cc[1], cd[0], cd[1], cd == '{5, 6});
endmodule
"#;

/// §10.3 / §10.9: a continuous assignment of an assignment pattern to an
/// unpacked array (a different path from the comparison above).
#[test]
fn continuous_assign_pattern_to_unpacked_array() {
    let want = ["T|ca 3 4 5 6 1"];
    assert_eq!(t_lines(CONT_ASSIGN), want);
}

const CONT_ASSIGN_SHAPES: &str = r#"module sub(output int so[2]);
  assign so = '{11, 12};
endmodule
module top;
  logic [3:0] cc[2]; int cd[2]; int dd[1:0]; int ee[3]; int ff[3]; int gg[0:3]; int kk[4]; logic [1:0][3:0] pk;
  int so[2]; logic [7:0] v = 8'd9; int hh[2][2];
  assign cc = '{4'd3, 4'd4};
  assign cd = '{5, 6};
  assign dd = '{7, 8};
  assign ee = '{default: 9};
  assign ff = '{3{v}};
  assign gg = '{0: 1, 2: 3, default: 5};
  assign kk = '{v, v + 1, 2, 3};
  assign pk = '{4'd1, 4'd2};
  assign hh = '{'{1,2},'{3,4}};
  sub u(so);
  initial begin
    #1 $display("T|ca %0d %0d %0d %0d %0d", cc[0], cc[1], cd[0], cd[1], cd == '{5, 6});
    $display("T|dd %0d %0d ee %0d %0d %0d ff %0d %0d", dd[1], dd[0], ee[0], ee[1], ee[2], ff[0], ff[2]);
    $display("T|gg %0d %0d %0d %0d kk %0d %0d %0d %0d pk %h", gg[0], gg[1], gg[2], gg[3], kk[0], kk[1], kk[2], kk[3], pk);
    $display("T|so %0d %0d hh %0d %0d %0d %0d", so[0], so[1], hh[0][0], hh[0][1], hh[1][0], hh[1][1]);
    v = 20;
    #1 $display("T|upd ff %0d kk %0d %0d", ff[1], kk[0], kk[1]);
  end
endmodule
"#;

/// Neighbours: a descending array (first item at the left bound),
/// `default:`, a replication, index keys, items reading a variable (the
/// elements follow it), a sub-module output port, a 2-D array of nested
/// patterns, and a packed array (unchanged path).
#[test]
fn continuous_assign_pattern_shapes() {
    // Expected values from the reference simulator.
    let want = [
        "T|ca 3 4 5 6 1",
        "T|dd 7 8 ee 9 9 9 ff 9 9",
        "T|gg 1 5 3 5 kk 9 10 2 3 pk 12",
        "T|so 11 12 hh 1 2 3 4",
        "T|upd ff 20 kk 20 21",
    ];
    assert_eq!(t_lines(CONT_ASSIGN_SHAPES), want);
}

const CONT_ASSIGN_DIRECTIONS: &str = r#"module top;
  int asc[0:1]; int dsc[1:0]; int a2[2:3]; int d2[3:2];
  assign dsc = asc;
  assign a2 = d2;
  initial begin
    asc[0] = 5; asc[1] = 6; d2[3] = 7; d2[2] = 8;
    #1 $display("T|dsc1=%0d dsc0=%0d a22=%0d a23=%0d", dsc[1], dsc[0], a2[2], a2[3]);
  end
endmodule
"#;

/// §7.6: `assign dst = src;` between unpacked arrays of opposite
/// directions pairs the elements left to right, not index for index.
#[test]
fn continuous_array_copy_pairs_left_to_right() {
    let want = ["T|dsc1=5 dsc0=6 a22=7 a23=8"];
    assert_eq!(t_lines(CONT_ASSIGN_DIRECTIONS), want);
}
