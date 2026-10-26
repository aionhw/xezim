//! §7.2 / §10.4 — a whole-struct blocking assignment between two
//! module-level unpacked-struct variables (`pv = q;`) inside an `always`
//! block copies every member, exactly as the same statement does in an
//! `initial` block. The always-block path lowered the copy to a store into
//! a whole-struct signal that does not exist (an unpacked struct is kept as
//! one leaf signal per member), so the target stayed at its initial value.
//!
//! Covers: a source filled by an assignment pattern and member by member;
//! a struct with a packed-vector member and a nested unpacked struct; a
//! conditional copy; and a copy triggered by the source itself
//! (`always @(q) pv = q;`). Expected lines follow the LRM and the
//! reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn whole_struct_copy_in_always_from_pattern_source() {
    let got = lines(
        r#"
module top;
  typedef struct { int f0; int f1; byte unsigned f2; } R;
  R q, pv;
  bit tick, tock;
  always @(tick) begin pv = q; tock = !tock; end
  initial begin
    q = '{3, 4, 113};
    tick = !tick; @(tock);
    $display("c: pv=%0d %0d %0d", pv.f0, pv.f1, pv.f2);
    q = '{5, 6, 7};
    tick = !tick; @(tock);
    $display("c2: pv=%0d %0d %0d", pv.f0, pv.f1, pv.f2);
  end
endmodule
"#,
    );
    assert_eq!(got, vec!["c: pv=3 4 113", "c2: pv=5 6 7"]);
}

#[test]
fn whole_struct_copy_in_always_from_memberwise_source() {
    let got = lines(
        r#"
module top;
  typedef struct { int f0; int f1; byte unsigned f2; } R;
  R q, pv;
  bit tick, tock;
  always @(tick) begin pv = q; tock = !tock; end
  initial begin
    q.f0 = 3; q.f1 = 4; q.f2 = 113;
    tick = !tick; @(tock);
    $display("d: pv=%0d %0d %0d", pv.f0, pv.f1, pv.f2);
  end
endmodule
"#,
    );
    assert_eq!(got, vec!["d: pv=3 4 113"]);
}

#[test]
fn whole_struct_copy_in_always_with_vector_and_nested_members() {
    let got = lines(
        r#"
module top;
  typedef struct { logic [7:0] lo; logic [7:0] hi; } P;
  typedef struct { int f0; bit [15:0] f3; P n; byte unsigned f2; } R;
  R q, pv;
  bit tick, tock;
  always @(tick) begin pv = q; tock = !tock; end
  initial begin
    q.f0 = 3; q.f3 = 16'hBEEF; q.n.lo = 8'h11; q.n.hi = 8'h22; q.f2 = 113;
    tick = !tick; @(tock);
    $display("e: pv=%0d %h %h %h %0d", pv.f0, pv.f3, pv.n.lo, pv.n.hi, pv.f2);
    q = '{7, 16'h1234, '{8'h33, 8'h44}, 9};
    tick = !tick; @(tock);
    $display("e2: pv=%0d %h %h %h %0d", pv.f0, pv.f3, pv.n.lo, pv.n.hi, pv.f2);
  end
endmodule
"#,
    );
    assert_eq!(got, vec!["e: pv=3 beef 11 22 113", "e2: pv=7 1234 33 44 9"]);
}

#[test]
fn whole_struct_copy_in_always_under_condition() {
    let got = lines(
        r#"
module top;
  typedef struct { int f0; int f1; byte unsigned f2; } R;
  R pv, dv;
  bit tick, tock, pend;
  always @(tick) begin
    if (pend) begin dv = pv; pend = 0; end
    tock = !tock;
  end
  initial begin
    pv = '{3, 4, 113};
    tick = !tick; @(tock);
    $display("f0: dv=%0d %0d %0d pend=%0d", dv.f0, dv.f1, dv.f2, pend);
    pend = 1;
    tick = !tick; @(tock);
    $display("f1: dv=%0d %0d %0d pend=%0d", dv.f0, dv.f1, dv.f2, pend);
    pv = '{8, 9, 10};
    tick = !tick; @(tock);
    $display("f2: dv=%0d %0d %0d pend=%0d", dv.f0, dv.f1, dv.f2, pend);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        vec![
            "f0: dv=0 0 0 pend=0",
            "f1: dv=3 4 113 pend=0",
            "f2: dv=3 4 113 pend=0",
        ]
    );
}

/// §9.4.2: an event control naming the whole variable fires on a change of
/// any member; `always_comb` infers the same sensitivity from the read.
#[test]
fn whole_struct_copy_in_always_sensitive_to_source() {
    let got = lines(
        r#"
module top;
  typedef struct { int f0; int f1; byte unsigned f2; } R;
  R q, pv, cv;
  always @(q) pv = q;
  always_comb cv = q;
  initial begin
    #1 q = '{3, 4, 113};
    #1 $display("g: pv=%0d %0d %0d cv=%0d %0d %0d", pv.f0, pv.f1, pv.f2, cv.f0, cv.f1, cv.f2);
    q.f1 = 40;
    #1 $display("g2: pv=%0d %0d %0d cv=%0d %0d %0d", pv.f0, pv.f1, pv.f2, cv.f0, cv.f1, cv.f2);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        vec!["g: pv=3 4 113 cv=3 4 113", "g2: pv=3 40 113 cv=3 40 113"]
    );
}

/// §10.4.2: the nonblocking form commits every member in the NBA region;
/// the `@(tock)` reader resumes in the active region of the same time step,
/// so it still sees the old members, and a later time step sees the copy.
#[test]
fn whole_struct_nonblocking_copy_in_always() {
    let got = lines(
        r#"
module top;
  typedef struct { int f0; int f1; byte unsigned f2; } R;
  R q, pv;
  bit tick, tock;
  always @(tick) begin pv <= q; tock = !tock; end
  initial begin
    q = '{3, 4, 113};
    tick = !tick; @(tock);
    $display("n0: pv=%0d %0d %0d", pv.f0, pv.f1, pv.f2);
    #1 $display("n1: pv=%0d %0d %0d", pv.f0, pv.f1, pv.f2);
  end
endmodule
"#,
    );
    assert_eq!(got, vec!["n0: pv=0 0 0", "n1: pv=3 4 113"]);
}

/// §11.4.5/§7.2: a whole-struct read in an expression of an `always` block
/// (`q == pv`) compares the members, like the same expression in an
/// `initial` block.
#[test]
fn whole_struct_compare_in_always() {
    let got = lines(
        r#"
module top;
  typedef struct { int f0; int f1; byte unsigned f2; } R;
  R q, pv;
  bit tick, tock, same;
  always @(tick) begin same = (q == pv); tock = !tock; end
  initial begin
    q = '{3, 4, 113};
    tick = !tick; @(tock);
    $display("h: same=%0d", same);
    pv = q;
    tick = !tick; @(tock);
    $display("h2: same=%0d", same);
    pv.f2 = 0;
    tick = !tick; @(tock);
    $display("h3: same=%0d", same);
  end
endmodule
"#,
    );
    assert_eq!(got, vec!["h: same=0", "h2: same=1", "h3: same=0"]);
}
