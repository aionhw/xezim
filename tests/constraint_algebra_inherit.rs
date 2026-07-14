//! Constraint-solver algebra, signedness and class-local types — issue #30,
//! IEEE 1800-2017 §18.3, §18.5.1, §18.5.5 and §18.5.12.
//!
//! §18.3 — a rand variable of a SIGNED type (`rand integer`) has a signed
//! domain. `int_val inside {[-100:100]}` denotes the 201 values from -100 to
//! 100; read as u64 its endpoints are 0xFFFF_FF9C and 100, i.e. an EMPTY range
//! that yields garbage draws. A solved value must also keep `is_signed`, or
//! every later compare (`int_val < 0`, the satisfaction judge) silently reads
//! it as a huge positive number.
//!
//! §18.5.12 — a constraint equality is ALGEBRAIC, not merely a "bare identifier
//! on one side of ==". `int_val + $signed({1'b0, l_val}) == 32'd50` and
//! `(r_val - 8'd10) * (l_val + 8'd5) == 16'd0` are affine in each of their rand
//! variables and must be SOLVED; leaving them to random luck made every trial
//! fail and randomize() return 0.
//!
//! §11.6.1 (which §18.5.12 defers to for operand rules) — the operands of a
//! comparison are extended to the CONTEXT width max(w_lhs, w_rhs) BEFORE the
//! arithmetic runs. Evaluated at their self-determined 8-bit width,
//! `(r_val - 8'd10) * (l_val + 8'd5)` wraps to 0 for r_val=12, l_val=123 (256
//! mod 256), so a violating draw reported itself "satisfied" and sailed through
//! the generate-and-test backstop. At the 16-bit context width, 256 != 0.
//!
//! §18.3/§18.4 — a type declared INSIDE a class body (`typedef enum bit [1:0]
//! {…} e; typedef struct packed {…} s;`) is a real type: a `rand e` draws only
//! declared members, and a `rand s`'s members alias bit slices of one integral
//! value.
//!
//! §18.5.1 — an out-of-class constraint body (`constraint C::name { … }`) is
//! part of the class's constraint set.
//!
//! §18.5.5 — an array SLICE inside a `unique {…}` list stands for its elements.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// §18.3: a signed rand variable's `inside` range with NEGATIVE endpoints is a
/// real interval, the draw lands in it, and the solved value stays signed (so
/// `int_val < 0` is true for roughly half the draws rather than never).
#[test]
fn signed_rand_inside_negative_range() {
    const SRC: &str = r#"
class Bus;
  rand integer int_val;
  rand int     small_val;
  constraint ranges {
    int_val   inside {[-100 : 100]};
    small_val inside {[-5 : -1]};
  }
endclass

module tb;
  int failures = 0;
  int neg_seen = 0;
  int pos_seen = 0;
  initial begin
    Bus b = new();
    repeat (60) begin
      if (b.randomize() != 1) failures++;
      // The range must be honoured as a SIGNED interval.
      if (!(b.int_val >= -100 && b.int_val <= 100)) failures++;
      if (!(b.small_val >= -5 && b.small_val <= -1)) failures++;
      // A solved value keeps its signedness: an all-ones 32-bit pattern read
      // unsigned would make `< 0` unreachable.
      if (b.int_val < 0) neg_seen++;
      if (b.int_val > 0) pos_seen++;
    end
    if (neg_seen == 0) failures++;  // domain never reached the negative half
    if (pos_seen == 0) failures++;
  end
endmodule
"#;
    let sim = simulate(SRC, 100_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.3: a signed rand var's inside-range endpoints are signed, and the \
         solved value keeps is_signed"
    );
    assert!(u(&sim, "neg_seen") > 0 && u(&sim, "pos_seen") > 0);
}

/// §18.5.12: an ALGEBRAIC equality is solved, not left to luck. Both an affine
/// sum across a signed and an unsigned variable and a factored product resolve,
/// and randomize() reports success.
#[test]
fn affine_equality_is_solved() {
    const SRC: &str = r#"
class Eq;
  rand reg   [7:0] r_val;
  rand logic [7:0] l_val;
  rand integer     int_val;

  constraint algebraic_factoring {
    // Only r_val == 10 can satisfy this at the 16-bit context width:
    // l_val + 5 is never 0 there for an 8-bit l_val.
    (r_val - 8'd10) * (l_val + 8'd5) == 16'd0;
    r_val inside {[10:20]};
  }
  constraint mixed_types {
    int_val inside {[-100 : 100]};
    int_val + $signed({1'b0, l_val}) == 32'd50;
  }
endclass

module tb;
  int failures = 0;
  initial begin
    Eq e = new();
    repeat (50) begin
      if (e.randomize() != 1) failures++;
      // The factored equation forces the only in-range root.
      if (e.r_val != 8'd10) failures++;
      // The affine sum holds exactly...
      if (e.int_val + $signed({1'b0, e.l_val}) != 32'd50) failures++;
      // ...and the inside range still holds alongside it.
      if (!(e.int_val >= -100 && e.int_val <= 100)) failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 200_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.5.12: affine/factored equalities are solved and randomize() succeeds"
    );
}

/// §11.6.1/§18.5.12: the satisfaction judge uses the same exact, context-width
/// arithmetic as the solver, so an 8-bit wrap cannot pass off a violating
/// assignment as satisfied. `sum` is 8 bits wide but is compared against a
/// 16-bit constant, so the addition happens at 16 bits and cannot wrap.
#[test]
fn truncating_arithmetic_cannot_fake_satisfaction() {
    const SRC: &str = r#"
class Wrap;
  rand bit [7:0] a;
  rand bit [7:0] b;
  constraint c {
    // At 8 bits a=200,b=66 would "satisfy" this (266 mod 256 == 10);
    // at the 16-bit context width of the comparison it does not.
    a + b == 16'd266;
    a inside {[150:255]};
  }
endclass

module tb;
  int failures = 0;
  initial begin
    Wrap w = new();
    repeat (40) begin
      if (w.randomize() != 1) failures++;
      // 16-bit truth: the operands really do add up to 266.
      if ((16'(w.a) + 16'(w.b)) != 16'd266) failures++;
      if (!(w.a >= 150)) failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§11.6.1: constraint operands extend to the context width before the \
         arithmetic — an 8-bit wrap must not read as satisfied"
    );
}

/// §18.3/§18.4: a CLASS-LOCAL `typedef enum` gives its rand property a member
/// domain (never an illegal encoding), and a CLASS-LOCAL packed struct lays out
/// its members as bit slices of the property's integral value — so a constraint
/// on `pkt_hdr.valid` drives real bits and reads back what it wrote.
#[test]
fn class_local_enum_and_packed_struct_rand() {
    const SRC: &str = r#"
class Bus;
  typedef enum bit [1:0] { START, DATA, STOP } type_e;   // 3 of 4 encodings legal
  typedef struct packed {
    bit       valid;
    bit [2:0] tag;
  } header_s;

  rand type_e   pkt_type;
  rand header_s pkt_hdr;

  constraint hdr_rules {
    (pkt_type == START) -> (pkt_hdr.valid == 1'b1 && pkt_hdr.tag > 3'd4);
    (pkt_type == STOP)  -> (pkt_hdr.valid == 1'b0 && pkt_hdr.tag == 3'd0);
  }
endclass

module tb;
  int failures = 0;
  int start_seen = 0;
  initial begin
    Bus b = new();
    repeat (60) begin
      if (b.randomize() != 1) failures++;
      // §18.3: a rand enum only ever takes a DECLARED member (2'b11 is not one).
      if (!(b.pkt_type == Bus::START || b.pkt_type == Bus::DATA
            || b.pkt_type == Bus::STOP)) failures++;
      // §18.4: the packed-struct members are live bit slices, so the
      // implications actually hold on read-back.
      if (b.pkt_type == Bus::START) begin
        start_seen++;
        if (b.pkt_hdr.valid !== 1'b1) failures++;
        if (!(b.pkt_hdr.tag > 3'd4)) failures++;
      end
      if (b.pkt_type == Bus::STOP) begin
        if (b.pkt_hdr.valid !== 1'b0) failures++;
        if (b.pkt_hdr.tag !== 3'd0) failures++;
      end
    end
    if (start_seen == 0) failures++;  // the START branch was never exercised
  end
endmodule
"#;
    let sim = simulate(SRC, 200_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.3/§18.4: a class-local enum restricts the draw to its members and a \
         class-local packed struct's members alias bit slices of the property"
    );
}

/// §18.5.1/§18.5.5: an out-of-class constraint body reaches the solver, and an
/// array SLICE named in a `unique {…}` list makes its elements distinct — from
/// the scalars AND from each other — without knocking a scalar out of the range
/// the out-of-class block gave it.
#[test]
fn out_of_class_block_and_unique_array_slice() {
    const SRC: &str = r#"
class Engine;
  rand bit [7:0] uniq_a;
  rand bit [7:0] uniq_b;
  rand bit [7:0] arr[4];

  constraint external_block_rule;          // §18.5.1 prototype

  constraint unique_group {
    unique { uniq_a, uniq_b, arr[0:2] };   // §18.5.5 — slice = its elements
  }
endclass

constraint Engine::external_block_rule {
  uniq_a inside {[1:10]};
  uniq_b inside {[11:20]};
}

module tb;
  int failures = 0;
  initial begin
    Engine e = new();
    repeat (40) begin
      if (e.randomize() != 1) failures++;
      // §18.5.1: the out-of-class body constrains the class.
      if (!(e.uniq_a >= 1 && e.uniq_a <= 10)) failures++;
      if (!(e.uniq_b >= 11 && e.uniq_b <= 20)) failures++;
      // §18.5.5: every member of the unique list is pairwise distinct.
      if (e.uniq_a == e.arr[0] || e.uniq_a == e.arr[1] || e.uniq_a == e.arr[2])
        failures++;
      if (e.uniq_b == e.arr[0] || e.uniq_b == e.arr[1] || e.uniq_b == e.arr[2])
        failures++;
      if (e.arr[0] == e.arr[1] || e.arr[1] == e.arr[2] || e.arr[0] == e.arr[2])
        failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 200_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.5.1/§18.5.5: an out-of-class constraint body constrains the class, \
         and a unique-list array slice expands to its elements"
    );
}
