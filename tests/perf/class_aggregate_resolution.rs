//! IEEE 1800-2023 §§7.2, 7.3, 7.4 and 8.25: shared aggregate declarations
//! must retain type bindings, value-dependent widths, union overlays and X/Z.
//! The deterministic work ceiling guards #815's repeated deep AST clones.

const DESIGN: &str = r#"
typedef struct packed { logic [7:0] value; logic [3:0] tag; } record_t;
typedef union packed {
  logic [15:0] bits;
  struct packed { logic [7:0] hi; logic [7:0] lo; } parts;
} overlay_t;
class holder #(type T=int, int W=8);
  T data;
  struct packed { logic [W-1:0] value; logic [3:0] tag; } sized;
  struct packed { T inner; logic tail; } nested;
endclass
module tb;
  int failures = 0;
  holder #(int, 4) scalar;
  holder #(record_t, 5) narrow_obj;
  holder #(record_t, 9) wide_obj;
  holder #(overlay_t, 8) overlay;
  initial begin
    scalar = new;
    scalar.data = 19;
    // Resolve the scalar specialization first: it must not poison T=struct.
    if (scalar.data !== 19) failures++;
    for (int i=0; i<64; i++) begin
      narrow_obj = new;
      wide_obj = new;
      overlay = new;
      narrow_obj.data.value = 8'h96;
      wide_obj.data.value = 8'h69;
      narrow_obj.sized.value = 5'h1b;
      wide_obj.sized.value = 9'h1a5;
      narrow_obj.sized.tag = 4'hc;
      wide_obj.sized.tag = 4'h3;
      narrow_obj.nested.inner.value = 8'hxz;
      overlay.data.bits = 16'hb7a2;
      if (narrow_obj.data.value !== 8'h96 || wide_obj.data.value !== 8'h69) failures++;
      if (narrow_obj.sized.value !== 5'h1b || narrow_obj.sized.tag !== 4'hc) failures++;
      if (wide_obj.sized.value !== 9'h1a5 || wide_obj.sized.tag !== 4'h3) failures++;
      if (narrow_obj.nested.inner.value !== 8'hxz) failures++;
      if (overlay.data.parts.hi !== 8'hb7 || overlay.data.parts.lo !== 8'ha2) failures++;
    end
    $display("CACHE_CHECK failures=%0d", failures);
  end
endmodule
"#;

#[test]
fn class_aggregate_resolution_scales_with_specializations_not_accesses() {
    let sim = xezim::simulate(DESIGN, 10).expect("simulate class aggregate variants");
    let output: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(output.contains(&"CACHE_CHECK failures=0"), "{output:?}");
    let (declarations, layouts) = sim.aggregate_type_work_counters();
    assert!(
        declarations > 0 && declarations <= 12,
        "declarations={declarations}"
    );
    assert!(layouts > 0 && layouts <= 12, "layouts={layouts}");
}

/// §6.18: block-local typedef registrations can replace the unqualified
/// type table entry between calls. A cache must observe that generation.
#[test]
fn aggregate_cache_observes_local_typedef_registrations() {
    let sim = xezim::simulate(
        r#"class holder #(type T=int);
 T data;
endclass
module tb;
 initial begin
  begin
   typedef struct packed { logic [7:0] value; } payload_t;
   holder #(payload_t) h;
   h = new;
   h.data.value = 8'ha5;
   $display("LOCAL8 %h", h.data.value);
  end
  begin
   typedef struct packed { logic [15:0] value; } payload_t;
   holder #(payload_t) h;
   h = new;
   h.data.value = 16'hbeef;
   $display("LOCAL16 %h", h.data.value);
  end
 end
endmodule
"#,
        10,
    )
    .expect("simulate local typedef variants");
    let output: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(output.contains(&"LOCAL8 a5"), "{output:?}");
    assert!(output.contains(&"LOCAL16 beef"), "{output:?}");
}

/// §§18.5, 18.8, 18.12, 18.13: cached declarations must not capture mutable
/// constraint operands or modes, or consume/reorder random draws. This digest
/// is the sequence from the pre-optimization simulator with object seed 1234.
#[test]
fn aggregate_cache_preserves_random_sequence_and_dynamic_constraints() {
    let sim = xezim::simulate(
        r#"typedef struct packed { bit [7:0] value; bit [3:0] tag; } record_t;
class packet;
 rand record_t r;
 int bound;
 constraint c { r.value <= bound; r.tag dist {0 := 1, 1 := 3}; }
endclass
module tb;
 initial begin
  packet p;
  int failures;
  int target;
  int unsigned digest;
  failures = 0;
  digest = 0;
  p = new;
  p.srandom(1234);
  for (int i=0; i<80; i++) begin
   p.bound = i % 7 + 1;
   target = i % (p.bound + 1);
   if (!p.randomize() with { r.value == local::target; }) failures++;
   if (p.r.value != target || p.r.value > p.bound || !(p.r.tag inside {0,1})) failures++;
   digest = digest * 33 + p.r;
  end
  p.r.rand_mode(0);
  p.bound = 255;
  target = p.r.value;
  if (!p.randomize() || p.r.value != target) failures++;
  p.r.rand_mode(1);
  p.c.constraint_mode(0);
  if (!p.randomize() with {r.value == 99; r.tag == 15;}) failures++;
  if (p.r.value != 99 || p.r.tag != 15) failures++;
  p.c.constraint_mode(1);
  if (!p.randomize() with {r.value == 2;}) failures++;
  if (!(p.r.tag inside {0,1})) failures++;
  $display("RANDOM_CACHE failures=%0d digest=%0d", failures, digest);
 end
endmodule
"#,
        10,
    )
    .expect("simulate repeated randomize");
    let output: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(
        output.contains(&"RANDOM_CACHE failures=0 digest=551618172"),
        "{output:?}"
    );
}
