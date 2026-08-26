//! §8.9: a fixed-size `static` array property in a class gets ONE shared
//! store, like any other static member.
//!
//! Elaboration used to route only the *dynamic* static collections (queue,
//! dynamic array, associative) into `static_collections`, and the whole
//! `array_properties` registration was gated to non-statics — so a
//! `static int S[3]` was registered nowhere at all and collapsed onto the
//! plain scalar cell built for every property. That single wrong storage
//! decision produced four symptoms at once (issue #126):
//!
//!   * `S[i]` read `x` — the index was a BIT select into a 32-bit scalar,
//!   * writes through one handle were invisible through another (no sharing),
//!   * `$size(S)` returned `32`, the collapsed scalar's WIDTH, not `3`,
//!   * `foreach (S[i])` bound a single garbage element.
//!
//! Fixed-size static arrays now carry their constant bounds through
//! `static_fixed_arrays` and are registered as one global array under the
//! bare name at startup — the same shared-store treatment the static
//! queue/assoc members already got.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── the issue #126 repro ──────────────────────────────────────────────
// A non-static `int M[3]` in the same class is the control: it stored and
// read back correctly throughout, which is what isolated the bug to the
// `static` path.

const STATIC_FIXED_ARRAY_SRC: &str = r#"
module top;
  class C;
    static int S[3];
    int        M[3];

    function void report_size();
      $display("SIZE_S_%0d", $size(S));
      $display("SIZE_M_%0d", $size(M));
    endfunction
  endclass

  initial begin
    C a = new();
    C b = new();

    a.S[0] = 10; a.S[1] = 20; a.S[2] = 30;
    a.M[0] = 1;  a.M[1] = 2;  a.M[2] = 3;

    $display("A_%0d_%0d_%0d", a.S[0], a.S[1], a.S[2]);
    $display("M_%0d_%0d_%0d", a.M[0], a.M[1], a.M[2]);
    // The point of `static`: b sees a's writes.
    $display("B_%0d_%0d_%0d", b.S[0], b.S[1], b.S[2]);
    a.report_size();

    begin
      int sum = 0;
      foreach (a.S[i]) sum += a.S[i];
      $display("SUM_%0d", sum);
    end
  end
endmodule
"#;

#[test]
fn test_static_fixed_array_shared_storage() {
    let sim = simulate(STATIC_FIXED_ARRAY_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let has = |t: &str| msgs.iter().any(|m| m == t);

    // Element reads land in real array storage, not a scalar bit-select.
    assert!(has("A_10_20_30"), "static array element reads: {:?}", msgs);
    // Control: the non-static member was never broken.
    assert!(has("M_1_2_3"), "non-static control array: {:?}", msgs);
    // §8.9 — one copy per class, shared across instances.
    assert!(has("B_10_20_30"), "static array not shared across handles: {:?}", msgs);
    // Array-query returns the SIZE, not the collapsed scalar's bit width.
    assert!(has("SIZE_S_3"), "$size of static array: {:?}", msgs);
    assert!(has("SIZE_M_3"), "$size of non-static array: {:?}", msgs);
    // foreach walks all three elements, not one garbage one.
    assert!(has("SUM_60"), "foreach over static array: {:?}", msgs);
}

// ── declaration forms that reach the same storage ─────────────────────
// `[lo:hi]` ranges, parameter-sized dimensions, typedef-carried dims, a
// narrow element type, inheritance, writes from a static method (no
// `this`), and class-scoped `C::S[i]` access with no handle involved.

const STATIC_FIXED_ARRAY_FORMS_SRC: &str = r#"
module top;
  parameter int N = 4;
  typedef int arr_t[3];

  class Base;
    static int B[2];
  endclass

  class C extends Base;
    static int   R[2:0];
    static int   P[N];
    static arr_t T;
    static byte  W[2];

    static function void bump();
      R[1] = R[1] + 5;
    endfunction
  endclass

  initial begin
    C c1 = new();
    C c2 = new();

    c1.R[0] = 1; c1.R[1] = 2; c1.R[2] = 3;
    c1.P[0] = 7; c1.P[3] = 9;
    c1.T[1] = 42;
    c1.W[0] = 8'hAB;
    c1.B[0] = 99;

    $display("R_%0d_%0d_%0d", c2.R[0], c2.R[1], c2.R[2]);
    $display("P_%0d_%0d", c2.P[0], c2.P[3]);
    $display("T_%0d", c2.T[1]);
    $display("W_%0h", c2.W[0]);
    $display("BASE_%0d", c2.B[0]);
    $display("SIZES_%0d_%0d_%0d_%0d_%0d",
             $size(C::R), $size(C::P), $size(C::T), $size(C::W), $size(C::B));

    C::bump();
    $display("BUMP_%0d", c2.R[1]);

    C::R[2] = 77;
    $display("SCOPED_%0d", c1.R[2]);
  end
endmodule
"#;

#[test]
fn test_static_fixed_array_declaration_forms() {
    let sim = simulate(STATIC_FIXED_ARRAY_FORMS_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let has = |t: &str| msgs.iter().any(|m| m == t);

    // `[2:0]` range form, shared across instances.
    assert!(has("R_1_2_3"), "[lo:hi] static array: {:?}", msgs);
    // Dimension sized by a parameter expression.
    assert!(has("P_7_9"), "parameter-sized static array: {:?}", msgs);
    // Dimensions carried by a typedef (classified at simulator startup, not
    // in elaborate_class, so it is a genuinely separate registration path).
    assert!(has("T_42"), "typedef-dims static array: {:?}", msgs);
    // Sub-word element width must survive.
    assert!(has("W_ab"), "byte-element static array: {:?}", msgs);
    // A static array declared in a base class, reached through a subclass.
    assert!(has("BASE_99"), "inherited static array: {:?}", msgs);
    assert!(has("SIZES_3_4_3_2_2"), "$size across forms: {:?}", msgs);
    // Write from a static method — no `this` handle in scope.
    assert!(has("BUMP_7"), "write from static method: {:?}", msgs);
    // Class-scoped write with no instance involved.
    assert!(has("SCOPED_77"), "C::S[i] scoped write: {:?}", msgs);
}

// ── follow-ups landed with the merge ──────────────────────────────────
// §6.8: a 4-STATE element type defaults to x, not 0 — the initial element
// build used Value::zero unconditionally, so `static logic [7:0] L[2]`
// read 00000000 where the LRM (and the reference simulator) give x.
const STATIC_FIXED_ARRAY_XDEFAULT_SRC: &str = r#"
module top;
  class C;
    static logic [7:0] L[2];
    static int         I[2];
  endclass
  initial begin
    C c = new();
    $display("XD_%b_%0d", c.L[0], c.I[0]);
  end
endmodule
"#;

#[test]
fn test_static_fixed_array_4state_defaults_x() {
    let sim = simulate(STATIC_FIXED_ARRAY_XDEFAULT_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "XD_xxxxxxxx_0"),
        "4-state static array element must default x (2-state stays 0): {:?}",
        msgs
    );
}

// §8.9 cross-class isolation (issue #135): the store is keyed
// `{DeclaringClass}::{member}`, so two classes declaring the same static
// array name get SEPARATE stores — and an inherited static resolves through
// the chain to the declaring class's one shared copy.
const STATIC_FIXED_ARRAY_COLLISION_SRC: &str = r#"
module top;
  class A; static int S[2]; endclass
  class B; static int S[2]; endclass
  initial begin
    A a = new(); B b = new();
    a.S[0] = 11; b.S[0] = 99;
    $display("COLL_%0d_%0d", a.S[0], b.S[0]);
  end
endmodule
"#;

#[test]
fn test_static_fixed_array_cross_class_isolated() {
    let sim = simulate(STATIC_FIXED_ARRAY_COLLISION_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "COLL_11_99"),
        "§8.9: same-named statics of different classes must not share a \
         store: {:?}",
        msgs
    );
}
