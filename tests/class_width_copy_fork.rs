//! Three defects found by user testbenches, all silent (wrong value, no error).
//!
//! 1. §6.18 / §8.3 — a class property's DECLARED WIDTH was ignored. Three
//!    converging causes: `elaborate_class` resolved property types with no
//!    typedef table, so every `TypeReference` property recorded 32 bits;
//!    `infer_lhs_width` knew nothing about class properties and fell back to
//!    32, so `std::randomize(prop)` drew a full 32-bit value; and a property
//!    write never clamped, so even `prop = 32'hDEADBEEF` on a 2-bit property
//!    kept all 32 bits.
//!
//! 2. §8.12 — `arr[i] = new src` (the SHALLOW COPY constructor) was not a copy
//!    at all. The index-lvalue path matched any `new` and called
//!    `instantiate_class`, running the constructor fresh: the element became a
//!    default-constructed object with newly allocated nested handles. The
//!    plain-handle form `h = new src` was already correct.
//!
//! 3. §9.3.2 — `fork automatic int j = i; begin ... end join_none` spawned the
//!    DECLARATION as its own process, with a frame the sibling could not see,
//!    so `j` read X. Fixing that alone still left every child reading the last
//!    iteration's value, because a declaration in a frameless process lands in
//!    the shared signal map; the declared names must be captured by value into
//!    each child's frame, as loop variables already were.

use xezim::simulate;

const CLASS_WIDTH: &str = r#"
typedef logic [1:0] u2_t;
class base_c; u2_t bp; endclass
class c extends base_c;
  u2_t        tv;    // typedef'd
  logic [1:0] iv;    // inline
  rand u2_t   rv;
  real        r;     // must NOT be resized
  string      s;     // must NOT be resized
  int         w32;   // must keep its full width
endclass
module tb;
  c o;
  logic [1:0] mod_scope;
  // Class properties live on the heap, not in the signal table, so copy the
  // values out to module signals the harness can read.
  int seen_tv, seen_iv, seen_bp, seen_w32;
  int rand_in_range, real_ok, string_ok;
  initial begin
    o = new();
    o.tv  = 32'hDEADBEEF;   // ordinary wide assign must truncate
    o.iv  = 32'hDEADBEEF;
    o.bp  = 32'hDEADBEEF;   // inherited property
    o.w32 = 32'hDEADBEEF;
    o.r   = 2.5;
    o.s   = "hi";
    void'(std::randomize(mod_scope));   // module scope was never broken
    void'(o.randomize());

    seen_tv  = o.tv;
    seen_iv  = o.iv;
    seen_bp  = o.bp;
    seen_w32 = o.w32;
    rand_in_range = (o.rv <= 3) ? 1 : 0;
    real_ok   = (o.r == 2.5)   ? 1 : 0;   // a real must not be resized
    string_ok = (o.s == "hi")  ? 1 : 0;   // nor a string
  end
endmodule
"#;

/// `std::randomize` on a class property, from inside the class.
const CLASS_RANDOMIZE: &str = r#"
typedef logic [1:0] u2_t;
class mini_class;
  u2_t var0;
  logic [2:0] var1;
  function new(); void'(std::randomize(var0, var1)); endfunction
endclass
module tb;
  mini_class obj;
  int in_range0, in_range1;
  initial begin
    obj = new();
    in_range0 = (obj.var0 <= 3) ? 1 : 0;
    in_range1 = (obj.var1 <= 7) ? 1 : 0;
  end
endmodule
"#;

const SHALLOW_COPY: &str = r#"
class inner_c; int v; endclass
class outer_c;
  int flat;
  inner_c nest;
  function new(); flat = 0; nest = new(); nest.v = 7; endfunction
endclass
module tb;
  outer_c src, h;
  outer_c arr [];
  int h_flat, a_flat, src_after_write;
  int h_shares, a_shares, ctor_still_works;
  initial begin
    src = new(); src.flat = 5; src.nest.v = 9;
    h   = new src;      // plain handle copy
    arr = new[1];
    arr[0] = new src;   // dynamic-array element copy

    h_flat = h.flat;    // flat members are copied
    a_flat = arr[0].flat;

    h.flat = 1; arr[0].flat = 2;
    src_after_write = src.flat;   // ...into a DISTINCT object

    h.nest.v = 11;                // nested handles are SHARED (shallow)
    h_shares = (src.nest.v == 11) ? 1 : 0;
    arr[0].nest.v = 22;
    a_shares = (src.nest.v == 22) ? 1 : 0;

    arr[0] = new();               // plain construction still constructs
    ctor_still_works = (arr[0].flat == 0 && arr[0].nest.v == 7) ? 1 : 0;
  end
endmodule
"#;

/// The LRM's own idiom: capture the loop variable per spawned process.
const FORK_AUTOMATIC: &str = r#"
module tb;
  int seen [5];
  int ended_at [5];
  initial begin
    for (int i = 0; i < 5; i++) begin
      fork
        automatic int local_i = i;
        begin
          time t;
          t = 10 - local_i;
          seen[local_i] = local_i;
          #t;
          ended_at[local_i] = $time;
        end
      join_none
    end
    #50;
  end
endmodule
"#;

/// Two declarations, and a fork whose declaration is NOT inside a loop
/// (the shape that accidentally worked before).
const FORK_EDGE: &str = r#"
module tb;
  int a_seen, b_seen, no_loop_seen;
  int join_all_done;
  initial begin
    for (int i = 3; i < 4; i++) begin
      fork
        automatic int a = i;
        automatic int b = i * 2;
        begin a_seen = a; b_seen = b; end
      join_none
    end
    fork
      automatic int x = 42;
      begin no_loop_seen = x; end
    join_none
    // `join` (not join_none) must still wait for its children.
    fork
      automatic int y = 7;
      begin #5; join_all_done = y; end
    join
    #1;
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

fn elem(sim: &xezim::compiler::Simulator, arr: &str, i: usize) -> u64 {
    u(sim, &format!("{}[{}]", arr, i))
}

#[test]
fn a_class_property_keeps_its_declared_width() {
    let sim = simulate(CLASS_WIDTH, 100).expect("simulate failed");
    // 32'hDEADBEEF truncated to 2 bits is 2'b11.
    assert_eq!(u(&sim, "seen_tv"), 3, "typedef'd property kept 32 bits");
    assert_eq!(u(&sim, "seen_iv"), 3, "inline-typed property kept 32 bits");
    assert_eq!(u(&sim, "seen_bp"), 3, "inherited property kept 32 bits");
}

#[test]
fn a_wide_class_property_is_not_truncated() {
    let sim = simulate(CLASS_WIDTH, 100).expect("simulate failed");
    assert_eq!(u(&sim, "seen_w32"), 0xDEAD_BEEF, "an int property must keep 32 bits");
    assert_eq!(u(&sim, "rand_in_range"), 1, "obj.randomize() must respect the width");
    assert_eq!(u(&sim, "real_ok"), 1, "a real property must not be resized");
    assert_eq!(u(&sim, "string_ok"), 1, "a string property must not be resized");
}

#[test]
fn std_randomize_respects_a_class_property_width() {
    // Randomized: run enough times that a 32-bit draw would show up.
    for _ in 0..16 {
        let sim = simulate(CLASS_RANDOMIZE, 100).expect("simulate failed");
        assert_eq!(u(&sim, "in_range0"), 1, "std::randomize drew wider than 2 bits");
        assert_eq!(u(&sim, "in_range1"), 1, "std::randomize drew wider than 3 bits");
    }
}

#[test]
fn a_shallow_copy_copies_values_into_a_distinct_object() {
    let sim = simulate(SHALLOW_COPY, 100).expect("simulate failed");
    assert_eq!(u(&sim, "h_flat"), 5, "h = new src did not copy");
    assert_eq!(u(&sim, "a_flat"), 5, "arr[i] = new src constructed a fresh object");
    assert_eq!(u(&sim, "src_after_write"), 5, "writing the copy corrupted the source");
}

#[test]
fn a_shallow_copy_shares_its_nested_handles() {
    let sim = simulate(SHALLOW_COPY, 100).expect("simulate failed");
    assert_eq!(u(&sim, "h_shares"), 1, "h = new src must share nested handles");
    assert_eq!(u(&sim, "a_shares"), 1, "arr[i] = new src must share nested handles");
}

#[test]
fn plain_construction_into_an_array_element_still_constructs() {
    let sim = simulate(SHALLOW_COPY, 100).expect("simulate failed");
    assert_eq!(u(&sim, "ctor_still_works"), 1, "arr[i] = new() regressed");
}

#[test]
fn a_fork_scope_automatic_is_captured_per_spawn() {
    let sim = simulate(FORK_AUTOMATIC, 200).expect("simulate failed");
    for i in 0..5u64 {
        assert_eq!(elem(&sim, "seen", i as usize), i, "process {} read the wrong local_i", i);
    }
}

#[test]
fn a_fork_scope_automatic_drives_a_real_delay() {
    let sim = simulate(FORK_AUTOMATIC, 200).expect("simulate failed");
    // t = 10 - local_i, so the LAST-spawned process finishes FIRST.
    for i in 0..5u64 {
        assert_eq!(
            elem(&sim, "ended_at", i as usize),
            10 - i,
            "process {} ended at the wrong time (an X delay collapses to 0)",
            i
        );
    }
}

#[test]
fn fork_declarations_work_in_every_shape() {
    let sim = simulate(FORK_EDGE, 200).expect("simulate failed");
    assert_eq!(u(&sim, "a_seen"), 3, "first of two fork declarations");
    assert_eq!(u(&sim, "b_seen"), 6, "second of two fork declarations");
    assert_eq!(u(&sim, "no_loop_seen"), 42, "a fork declaration outside a loop");
    assert_eq!(u(&sim, "join_all_done"), 7, "a fork declaration under plain `join`");
}
