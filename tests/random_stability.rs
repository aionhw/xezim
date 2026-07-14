//! IEEE 1800-2023 §18.13 (randomization methods) and §18.14 (random stability).
//!
//! Issue #30 (§18 tests 11 and 12). Both died with a SIMULATION ERROR because
//! the built-in `process` class of §9.7 was rejected as an undeclared identifier
//! and none of the §18.14 random-state methods existed:
//!
//!   * §18.13.3 / §18.14.1 `srandom(seed)` — on the CURRENT PROCESS
//!     (`process::self().srandom(s)`) and on a class OBJECT (`obj.srandom(s)` /
//!     `this.srandom(s)`). Reseeding must be deterministic: the same seed always
//!     yields the same subsequent sequence.
//!   * §18.14.2 `get_randstate()` / `set_randstate(s)` — capture and restore a
//!     stream's state; a restored stream replays the identical sequence.
//!   * §18.14.1 stream independence: an object's stream and a thread's stream are
//!     separate. Reseeding a thread must not disturb an object's sequence, and
//!     `obj.randomize()` must not consume draws from the thread's `$urandom`
//!     sequence (nor from another object's).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// §18.13.3/§18.14.1: `srandom(seed)` is deterministic for BOTH a process
/// (`process::self()`, §9.7 — it reseeds `$urandom`) and a class object (it
/// reseeds the object's `randomize()` solver stream).
#[test]
fn srandom_reseed_is_deterministic() {
    const SRC: &str = r#"
module tb;
  class Obj;
    rand bit [31:0] d;
  endclass

  Obj o;
  int unsigned t1, t2;
  bit [31:0] o1, o2;
  int thread_repeats = 0;
  int object_repeats = 0;
  int thread_varies = 0;

  initial begin
    process p;
    bit [31:0] t1b;
    p = process::self();
    o = new();

    // Same process seed -> same $urandom sequence (§18.14.1).
    p.srandom(32'hDEAD_BEEF);
    t1 = $urandom();
    t1b = $urandom();
    p.srandom(32'hDEAD_BEEF);
    t2 = $urandom();
    if (t1 == t2) thread_repeats = 1;
    if (t1 != t1b) thread_varies = 1;   // the stream actually advances

    // Same object seed -> same randomize() result (§18.13.3).
    o.srandom(32'h1234_5678);
    void'(o.randomize());
    o1 = o.d;
    o.srandom(32'h1234_5678);
    void'(o.randomize());
    o2 = o.d;
    if (o1 == o2) object_repeats = 1;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulation failed");
    assert_eq!(u(&sim, "thread_repeats"), 1, "process srandom(seed) did not reproduce the $urandom sequence");
    assert_eq!(u(&sim, "object_repeats"), 1, "object srandom(seed) did not reproduce the randomize() value");
    assert_eq!(u(&sim, "thread_varies"), 1, "the seeded thread stream is stuck on one value");
}

/// §18.14.2: `get_randstate()` captures the thread's stream; `set_randstate()`
/// restores it, and the stream then replays the EXACT same sequence — including
/// through `shuffle()` (§7.12.1), which draws from the same stream.
#[test]
fn get_set_randstate_replays_the_sequence() {
    const SRC: &str = r#"
module tb;
  int unsigned a0, a1, a2;
  int unsigned b0, b1, b2;
  int seq_matches = 0;
  int seq_varies = 0;
  int shuffle_matches = 0;
  int state_nonempty = 0;

  initial begin
    process p;
    string st;
    int arr[] = '{1, 2, 3, 4, 5, 6, 7, 8};
    int gold[];

    p = process::self();
    st = p.get_randstate();
    if (st.len() > 0) state_nonempty = 1;

    a0 = $urandom();
    a1 = $urandom();
    a2 = $urandom();

    // Rewind: the very same three draws must come back.
    p.set_randstate(st);
    b0 = $urandom();
    b1 = $urandom();
    b2 = $urandom();
    if (a0 == b0 && a1 == b1 && a2 == b2) seq_matches = 1;
    if (a0 != a1 || a1 != a2) seq_varies = 1;

    // Same rewind, but observed through shuffle() — the golden permutation must
    // be reproduced bit-for-bit.
    st = p.get_randstate();
    arr.shuffle();
    gold = arr;
    p.set_randstate(st);
    arr = '{1, 2, 3, 4, 5, 6, 7, 8};
    arr.shuffle();
    shuffle_matches = 1;
    foreach (arr[i])
      if (arr[i] != gold[i]) shuffle_matches = 0;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulation failed");
    assert_eq!(u(&sim, "state_nonempty"), 1, "get_randstate() returned an empty string");
    assert_eq!(u(&sim, "seq_matches"), 1, "set_randstate() did not replay the captured $urandom sequence");
    assert_eq!(u(&sim, "seq_varies"), 1, "the captured stream is stuck on one value");
    assert_eq!(u(&sim, "shuffle_matches"), 1, "set_randstate() did not reproduce the shuffle() permutation");
}

/// §18.14.1: every object owns its own stream, distinct from other objects' and
/// from the calling thread's.
///  * two objects seeded alike produce the same sequence, and draws on one do
///    NOT advance the other;
///  * `obj.randomize()` does not consume the thread's `$urandom` draws, and
///    reseeding the thread does not perturb the object's stream.
#[test]
fn object_and_thread_streams_are_independent() {
    const SRC: &str = r#"
module tb;
  class Obj;
    rand bit [31:0] d;
  endclass

  Obj a, b;
  bit [31:0] av1, av2, bv1;
  int unsigned u1, u2;
  bit [31:0] ov1, ov2;
  int objects_independent = 0;
  int thread_unshifted = 0;
  int object_unshifted = 0;

  initial begin
    process p;
    p = process::self();
    a = new();
    b = new();

    // Two objects, same seed: b's FIRST draw must equal a's FIRST draw even
    // though a has already drawn twice — the streams are separate.
    a.srandom(32'h0000_0063);
    b.srandom(32'h0000_0063);
    void'(a.randomize()); av1 = a.d;
    void'(a.randomize()); av2 = a.d;
    void'(b.randomize()); bv1 = b.d;
    if (bv1 == av1 && av2 != av1) objects_independent = 1;

    // A randomize() in between must not shift the thread's $urandom sequence.
    p.srandom(32'h0000_0005);
    u1 = $urandom();
    p.srandom(32'h0000_0005);
    void'(a.randomize());
    u2 = $urandom();
    if (u1 == u2) thread_unshifted = 1;

    // ...and reseeding the thread must not reset/disturb the object's stream:
    // with the object reseeded identically, its sequence repeats regardless.
    a.srandom(32'h0000_0063);
    void'(a.randomize()); ov1 = a.d;
    p.srandom(32'h0000_0077);        // thread reseed - object must not care
    void'(a.randomize()); ov2 = a.d;
    if (ov1 == av1 && ov2 == av2) object_unshifted = 1;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulation failed");
    assert_eq!(u(&sim, "objects_independent"), 1, "two objects seeded alike do not have independent streams");
    assert_eq!(u(&sim, "thread_unshifted"), 1, "obj.randomize() consumed draws from the thread's $urandom stream");
    assert_eq!(u(&sim, "object_unshifted"), 1, "a process reseed disturbed the object's random stream");
}
