//! Audit follow-ups to the array-of-queues work: the same "aggregate whose
//! element is a dynamic collection" shape in its other spellings, plus three
//! neighbouring bugs the audit turned up.
//!
//!   int a[2][u8_t]   array of ASSOCIATIVE arrays (§7.8)
//!   int m[2][3]      a plain 2-D array written with SIZES, not ranges
//!   p_t q[$]         a queue whose element is an unpacked struct
//!   foreach (m[, j]) an omitted loop variable

use xezim::simulate;

/// `logic [31:0] mem[2][u8_t]` — the trailing associative dimension was dropped,
/// so `mem[i]` was a plain scalar: every write vanished and every read was X.
const ASSOC: &str = r#"
typedef logic [7:0]  u8_t;
typedef logic [31:0] u32_t;

module tb;
  logic [31:0] mem [2][u8_t];
  logic [31:0] str_keyed [2][string];
  u8_t  addr;
  int   fails;
  int   n0, n1, ns;
  u32_t read_back;

  initial begin
    for (int h = 0; h < 2; h++) begin
      addr = 4;
      do begin
        mem[h][addr] = u32_t'(addr) + u32_t'(h * 256);
        addr++;
      end while (addr != 8);
    end
    for (int h = 0; h < 2; h++) begin
      addr = 4;
      do begin
        if (mem[h][addr] !== u32_t'(addr) + u32_t'(h * 256)) fails++;
        addr++;
      end while (addr != 8);
    end
    read_back = mem[1][6];

    str_keyed[0]["k"] = 9;
    ns = str_keyed[0].num();

    n0 = mem[0].num();
    n1 = mem[1].num();
  end
endmodule
"#;

/// A 2-D array spelled with sizes rather than ranges, and `%p` over it.
const DIMS: &str = r#"
module tb;
  int e2 [2][3];      // size form — used to read X
  int r2 [0:1][0:2];  // range form
  int q  [2][$];      // array of queues, for the %p recursion
  int a, b;
  int skipped_iters;
  initial begin
    foreach (e2[i, j]) e2[i][j] = i * 10 + j;
    r2[1][2] = 12;
    a = e2[1][2];
    b = r2[1][2];

    q[0].push_back(7);
    q[1].push_back(8); q[1].push_back(9);

    // §12.7.3: an OMITTED loop variable means that dimension is not traversed.
    skipped_iters = 0;
    foreach (e2[, j]) skipped_iters++;

    $display("E2=%p", e2);
    $display("Q=%p", q);
  end
endmodule
"#;

/// `push_back` wrote one packed value, so an unpacked-struct element lost every
/// member. Both a struct literal and a struct variable must survive.
const STRUCTQ: &str = r#"
module tb;
  typedef struct { int a; string s; } p_t;
  p_t pq [$];      // plain queue of structs
  p_t sq [2][$];   // array of queues of structs
  p_t src;
  initial begin
    pq.push_back('{5, "lit"});
    src = '{7, "var"};
    pq.push_back(src);
    sq[1].push_back('{9, "arr"});
    $display("PQ0=%p", pq[0]);
    $display("PQ1=%p", pq[1]);
    $display("SQ=%p", sq[1][0]);
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn each_element_of_an_array_of_assoc_arrays_is_its_own_assoc_array() {
    let sim = simulate(ASSOC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "fails"), 0, "assoc element read-back mismatched");
    assert_eq!(u(&sim, "read_back"), 6 + 256, "mem[1][6] wrong");
    // Separate key spaces per element.
    assert_eq!(u(&sim, "n0"), 4);
    assert_eq!(u(&sim, "n1"), 4);
    // String-keyed elements work too.
    assert_eq!(u(&sim, "ns"), 1);
}

#[test]
fn two_dimensional_array_written_with_sizes_is_addressable() {
    let sim = simulate(DIMS, 100).expect("simulate failed");
    assert_eq!(u(&sim, "a"), 12, "int e2[2][3] element read X");
    assert_eq!(u(&sim, "b"), 12, "int r2[0:1][0:2] regressed");
}

#[test]
fn foreach_does_not_traverse_a_dimension_whose_variable_is_omitted() {
    let sim = simulate(DIMS, 100).expect("simulate failed");
    // `foreach (e2[, j])` iterates j over 0..2 only — 3, not 2*3.
    assert_eq!(u(&sim, "skipped_iters"), 3);
}

#[test]
fn p_format_descends_multi_dim_arrays_and_collection_elements() {
    let sim = simulate(DIMS, 100).expect("simulate failed");
    assert_eq!(line(&sim, "E2="), "E2='{'{0, 1, 2}, '{10, 11, 12}}");
    // Elements of an array of queues print as queues, not as X.
    assert_eq!(line(&sim, "Q="), "Q='{'{7}, '{8, 9}}");
}

#[test]
fn push_back_keeps_every_member_of_an_unpacked_struct_element() {
    let sim = simulate(STRUCTQ, 100).expect("simulate failed");
    assert_eq!(line(&sim, "PQ0="), r#"PQ0='{a:5, s:"lit"}"#);
    assert_eq!(line(&sim, "PQ1="), r#"PQ1='{a:7, s:"var"}"#);
    assert_eq!(line(&sim, "SQ="), r#"SQ='{a:9, s:"arr"}"#);
}
