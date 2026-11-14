//! §10.9 / §11.4.5 comparison of a PACKED aggregate against a bare assignment
//! pattern (issue #1043, Bug J). The pattern is in an assignment-like context
//! and takes the packed aggregate's type, so the comparison runs element- by
//! element (member-by-member for a packed struct) — `=`/`!=`/`===`/`!==` on a
//! packed array vs a bare `'{...}` pattern used to always report unequal
//! because the pattern compared as one packed concatenation against the array's
//! (nonexistent) container signal. Reference-verified for packed arrays
//! (`[0:3]` and descending `[1:0]` dims), multi-dimensional packed arrays, and
//! packed structs.

use xezim::simulate;

fn run(sv: &str) -> Vec<String> {
    simulate(sv, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn packed_array_equals_bare_pattern() {
    let msgs = run(r#"
module top;
  typedef logic [2:0] u3_t;
  typedef u3_t des_t [1:0];
  u3_t [0:3] a;
  des_t des;
  initial begin
    a = '{2,2,2,2};
    des[1] = 2; des[0] = 2;
    $display("A1_%b", a == '{2,2,2,2});
    $display("A2_%b", a != '{2,2,2,2});
    $display("A3_%b", a === '{2,2,2,2});
    $display("A4_%b", a !== '{2,2,2,2});
    $display("A5_%b", a == '{2,2,2,3});
    $display("A6_%b", a != '{2,2,2,3});
    $display("D1_%b", des == '{2,2});
    $display("D2_%b", des == '{2,3});
  end
endmodule
"#);
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("A1_1"), "asc eq: {:?}", msgs);
    assert!(has("A2_0"), "asc ne: {:?}", msgs);
    assert!(has("A3_1"), "asc case-eq: {:?}", msgs);
    assert!(has("A4_0"), "asc case-ne: {:?}", msgs);
    assert!(has("A5_0"), "diff elem: {:?}", msgs);
    assert!(has("A6_1"), "diff elem ne: {:?}", msgs);
    assert!(has("D1_1"), "desc eq: {:?}", msgs);
    assert!(has("D2_0"), "desc diff: {:?}", msgs);
}

#[test]
fn packed_multi_dim_array_vs_nested_pattern() {
    let msgs = run(r#"
module top;
  logic [1:0][0:3][2:0] d2d;
  initial begin
    d2d = '{'{1,2,3,4},'{5,6,7,8}};
    $display("M1_%b", d2d == '{'{1,2,3,4},'{5,6,7,8}});
    $display("M2_%b", d2d == '{'{1,2,3,4},'{5,6,7,9}});
    $display("M3_%b", d2d === '{'{1,2,3,4},'{5,6,7,8}});
  end
endmodule
"#);
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("M1_1"), "2-d equal: {:?}", msgs);
    assert!(has("M2_0"), "2-d differing inner element: {:?}", msgs);
    assert!(has("M3_1"), "2-d case-eq: {:?}", msgs);
}

#[test]
fn packed_struct_vs_bare_pattern() {
    let msgs = run(r#"
module top;
  typedef struct packed {
    logic [11:0] x;
    logic [7:0]  y;
  } ps_t;
  ps_t ps;
  initial begin
    ps = '{12'h34, 8'h56};
    $display("S1_%b", ps == '{12'h34, 8'h56});
    $display("S2_%b", ps == '{12'h35, 8'h56});
    $display("S3_%b", ps === '{12'h34, 8'h56});
    $display("S4_%b", ps == '{y:8'h56, x:12'h34});
  end
endmodule
"#);
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("S1_1"), "packed struct positional eq: {:?}", msgs);
    assert!(has("S2_0"), "packed struct differing member: {:?}", msgs);
    assert!(has("S3_1"), "packed struct case-eq: {:?}", msgs);
    assert!(has("S4_1"), "packed struct named items: {:?}", msgs);
}

#[test]
fn wildcard_ops_on_packed_aggregate_vs_pattern() {
    // §11.4.6 `==?`/`!=?` on a packed aggregate vs a bare pattern: these were
    // not dispatched through the pattern comparator (bug cousin of J) and so
    // reported unequal — a no-wildcard pattern must compare like `==`. An x/z
    // in the pattern item is a don't-care; an x/z in the element at a
    // non-masked position makes that element comparison unknown unless another
    // element definitely mismatches (forcing 0). Reference-verified.
    let msgs = run(r#"
module top;
  typedef logic [2:0] u3_t;
  typedef u3_t st_t [0:3];
  typedef struct packed { logic [7:0] a; logic [3:0] b; } ps_t;
  st_t a;
  ps_t ps;
  initial begin
    a = '{1,2,3,4};
    ps = '{8'h34, 4'hC};
    $display("W1_%b", a ==? '{1,2,3,4});
    $display("W2_%b", a !=? '{1,2,3,4});
    $display("W3_%b", a ==? '{1,2,3,5});
    $display("W4_%b", a ==? '{1,2,'x,4});  // 'x item is a don't-care
    $display("W5_%b", ps ==? '{8'h34, 4'hC});
    $display("W6_%b", ps ==? '{8'h34, 4'hD});
  end
endmodule
"#);
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("W1_1"), "wildcard eq no-wildcard pattern: {:?}", msgs);
    assert!(has("W2_0"), "wildcard ne no-wildcard pattern: {:?}", msgs);
    assert!(has("W3_0"), "wildcard eq differing item: {:?}", msgs);
    assert!(has("W4_1"), "wildcard item don't-care: {:?}", msgs);
    assert!(has("W5_1"), "wildcard packed struct eq: {:?}", msgs);
    assert!(has("W6_0"), "wildcard packed struct diff: {:?}", msgs);
}

#[test]
fn packed_slice_vs_bare_pattern() {
    // A packed sub-array slice (`d2[0]`, an Index expression into a packed
    // array) compared against a bare pattern must compare its rows element-
    // wise like the whole array. This was a separate latent gap (fallback to
    // the packed-scalar compare always returned 0). Reference-verified.
    let msgs = run(r#"
module top;
  logic [0:3][3:0][2:0] d2;
  initial begin
    d2 = '{'{1,2,3,4},'{5,6,7,8},'{9,10,11,12},'{13,14,15,16}};
    $display("Z1_%b", d2[0] == '{1,2,3,4});
    $display("Z2_%b", d2[0] == '{1,2,3,5});
    $display("Z3_%b", d2[0] != '{1,2,3,4});
    $display("Z4_%b", d2[0] === '{1,2,3,4});
    $display("Z5_%b", d2[2] == '{9,10,11,12});
  end
endmodule
"#);
    let has = |t: &str| msgs.iter().any(|m| m == t);
    assert!(has("Z1_1"), "slice equal row: {:?}", msgs);
    assert!(has("Z2_0"), "slice differing element: {:?}", msgs);
    assert!(has("Z3_0"), "slice ne equal: {:?}", msgs);
    assert!(has("Z4_1"), "slice case-eq: {:?}", msgs);
    assert!(has("Z5_1"), "slice later row: {:?}", msgs);
}
