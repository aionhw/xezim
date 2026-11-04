//! §7.2 — unpacked-struct storage outside the top module, continued. Found by
//! sweeping the same construct through every write form (member-wise / pattern
//! / whole copy) in both scopes; all reference-validated.
//!
//! 1. **An array of unpacked structs in an INSTANCE stored packed elements.**
//!    The top-level path registers every element member-wise; the inlining
//!    path did not, so `arr[i].m = v` wrote where no read ever looked and the
//!    member came back `x` — while the identical declaration at top level was
//!    correct. The child array also recorded no element type, so
//!    `arr[i] = '{...}` could not resolve the element struct either.
//! 2. **An unpacked struct member of an INTERFACE never took a pattern.**
//!    `bi.us = '{a:.., b:..}` has no signal of its own — only member leaves —
//!    so `resolve_hier_name` collapsed it to the leaf `us`, which owns no
//!    type, and the aggregate spread was skipped. The lvalue's fully-joined
//!    path is now tried as well.
//! 3. **An element-to-element struct copy in an instance copied nothing.**
//!    `arr[2] = arr[0]` resolved no type for `arr[2]`; an element carries no
//!    type of its own, but the container's recorded ELEMENT type is exactly
//!    it, so `p_elem_type` now falls back to the container.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Every write form into an array of unpacked structs, inside an instance.
#[test]
fn struct_array_elements_in_an_instance() {
    let src = r#"
module leaf;
  typedef struct { logic [7:0] a; logic [7:0] b; } s_t;
  s_t arr [3];
  s_t untouched [2];
  initial begin
    arr[0]   = '{a:8'h11, b:8'h22};   // pattern into an element
    arr[1].a = 8'h33;                 // member-wise into an element
    arr[1].b = 8'h44;
    arr[2]   = arr[0];                // element-to-element copy
  end
endmodule
module tb;
  leaf u();
  int p_a, p_b, m_a, m_b, c_a, c_b, untouched_x;
  initial begin
    #1;
    p_a = u.arr[0].a; p_b = u.arr[0].b;
    m_a = u.arr[1].a; m_b = u.arr[1].b;
    c_a = u.arr[2].a; c_b = u.arr[2].b;
    untouched_x = $isunknown(u.untouched[0].a);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        (u(&sim, "p_a"), u(&sim, "p_b")),
        (0x11, 0x22),
        "pattern into element"
    );
    assert_eq!(
        (u(&sim, "m_a"), u(&sim, "m_b")),
        (0x33, 0x44),
        "member-wise into element"
    );
    assert_eq!(
        (u(&sim, "c_a"), u(&sim, "c_b")),
        (0x11, 0x22),
        "element-to-element copy"
    );
    assert_eq!(u(&sim, "untouched_x"), 1, "an unwritten element stays x");
}

/// Interface members: unpacked struct via a pattern, plus the array and scalar
/// members that already worked — written both through a port and directly.
#[test]
fn interface_unpacked_struct_member_pattern() {
    let src = r#"
interface bus_if;
  typedef struct { logic [7:0] a; logic [7:0] b; } s_t;
  s_t         us, us_direct;
  logic [7:0] arr [2];
  logic [7:0] plain;
endinterface
module drv(bus_if b);
  initial begin
    b.us     = '{a:8'h11, b:8'h22};
    b.arr[0] = 8'hA0;
    b.arr[1] = 8'hA1;
    b.plain  = 8'h5A;
  end
endmodule
module tb;
  bus_if bi();
  drv d(.b(bi));
  int ua, ub, da, db, a0, a1, pl;
  initial begin
    bi.us_direct = '{a:8'h33, b:8'h44};
    #1;
    ua = bi.us.a;        ub = bi.us.b;
    da = bi.us_direct.a; db = bi.us_direct.b;
    a0 = bi.arr[0];      a1 = bi.arr[1];
    pl = bi.plain;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        (u(&sim, "ua"), u(&sim, "ub")),
        (0x11, 0x22),
        "pattern through a modport-less port"
    );
    assert_eq!(
        (u(&sim, "da"), u(&sim, "db")),
        (0x33, 0x44),
        "pattern written directly"
    );
    assert_eq!(
        (u(&sim, "a0"), u(&sim, "a1")),
        (0xA0, 0xA1),
        "array member unaffected"
    );
    assert_eq!(u(&sim, "pl"), 0x5A, "scalar member unaffected");
}

/// §10.4.2: a nonblocking write of one member of an unpacked-struct array
/// element inside an instance. The instance-prefixed flat name
/// (`u.storage[0].value`) used to be given the element's struct type, so the
/// NBA expanded as a whole-struct copy whose leaf was `....value.value` and
/// the expansion recursed without advancing time.
#[test]
fn nba_struct_array_member_in_instance() {
    let src = r#"
module leaf(input logic clk, input logic rst);
  typedef struct { logic [31:0] value; } f_t;
  f_t storage[2];
  always_ff @(posedge clk) begin
    if (rst) storage[0].value <= 32'h0;
    else storage[0].value <= 32'h2a;
  end
endmodule
module tb;
  logic clk = 0;
  always #5 clk = ~clk;
  logic rst = 1;
  leaf u(.clk(clk), .rst(rst));
  initial begin
    #12 rst = 0;
    #10 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "u.storage[0].value"), 0x2a);
    assert!(sim.get_signal("u.storage[1].value").is_some());
}

/// §10.4.2, §7.10: a nonblocking write of a member of a queue element wakes
/// the processes that read it. Readers of a queue element watch the queue, so
/// the write must not be redirected to a flat member signal.
#[test]
fn nba_queue_element_member_wakes_readers() {
    let src = r#"
typedef struct { logic [7:0] f; } s_t;
module tb;
  logic clk = 0;
  s_t q[$];
  logic [7:0] r;
  always_comb r = q[0].f;
  always_ff @(posedge clk) q[0].f <= q[0].f + 8'h1;
  initial begin
    q.push_back('{f: 8'h10});
    repeat (3) begin #5 clk = 1; #5 clk = 0; end
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "r"), 0x13);
}
