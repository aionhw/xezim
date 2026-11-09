//! §8.25 / §7.4.1 — packed class properties: parameter-dependent widths, and
//! bit/part selects on them. Reference-validated.
//!
//! Three defects, all silent:
//!
//! 1. A property whose packed range references a class PARAMETER was seeded
//!    from the class's elaborated signature — i.e. at the class's DEFAULT
//!    parameter value — so `bit [W-1:0] pw` in a `P#(16)` was created 8 bits
//!    wide. A later WHOLE assignment re-widened it, so the damage was confined
//!    to bit/part-select writes above the default width, which vanished, and
//!    to `$bits` until the first whole write.
//! 2. A single-bit WRITE to any packed property was dropped entirely: the
//!    element resolver handled only multi-dimensional properties, so a plain
//!    `bit [15:0]` had no handler. Reads fell through to a generic bit-select
//!    and were fine, and the part-select `v[15:8] = ...` beside it worked,
//!    which is what made this hard to see.
//! 3. An ASCENDING property (`bit [0:15]`) labels its bits from the MSB end,
//!    but a part-select WRITE spliced the bits a descending vector would have
//!    — while a single-bit write to the same property was correct.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// A parameter-sized property is allocated at the SPECIALIZATION's width, so
/// selects above the class's default width land.
#[test]
fn parameter_sized_property_uses_the_specialization_width() {
    let src = r#"
class P #(int W = 8);
  bit [W-1:0] pw;
endclass
module tb;
  P #(16) a, b, c;
  P #(4)  d;
  int part, bit_hi, bits16, bits4;
  initial begin
    a = new(); b = new(); c = new(); d = new();
    a.pw = '0; a.pw[15:8] = 8'hAA; a.pw[7:0] = 8'h55;
    b.pw = '0; b.pw[15] = 1'b1;
    part   = a.pw;
    bit_hi = b.pw;
    bits16 = $bits(c.pw);
    bits4  = $bits(d.pw);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "part"),
        0xAA55,
        "part-writes above the default width land"
    );
    assert_eq!(u(&sim, "bit_hi"), 0x8000, "so does a bit-write above it");
    assert_eq!(u(&sim, "bits16"), 16, "$bits before any write");
    assert_eq!(
        u(&sim, "bits4"),
        4,
        "a narrower specialization is independent"
    );
}

/// Bit and part selects across descending, ascending and multi-dimensional
/// properties.
#[test]
fn packed_property_bit_and_part_selects() {
    let src = r#"
class P;
  bit [15:0]     d;
  bit [0:15]     a;
  bit [1:0][7:0] m;
endclass
module tb;
  P p;
  int r_d, r_a, r_m1, r_m0;
  initial begin
    p = new();
    p.d = '0; p.a = '0;
    p.d[15]  = 1'b1;
    p.d[3:0] = 4'hF;
    p.a[0]   = 1'b1;      // ascending: label 0 is the MSB
    p.a[4:7] = 4'hF;      // labels 4..7 from the MSB end
    p.m[1]   = 8'hAB;
    p.m[0]   = 8'hCD;
    #1;
    r_d = p.d; r_a = p.a; r_m1 = p.m[1]; r_m0 = p.m[0];
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r_d"), 0x800F, "descending bit and part writes");
    assert_eq!(u(&sim, "r_a"), 0x8F00, "ascending writes map by label");
    assert_eq!(
        (u(&sim, "r_m1"), u(&sim, "r_m0")),
        (0xAB, 0xCD),
        "multi-dim elements"
    );
}

/// §6.18 / §7.4.1: a property declared through a TYPEDEF is the same packed
/// vector as its target, so a bit/part select on it must address the
/// property's bits. It did not: `class_prop_packed_shape` matched only a
/// `DataType::IntegerVector` / `Implicit` declaration, and an alias keeps the
/// `TypeReference` in `property_types` (a class-local alias too). Every
/// select fell through the class arms and was silently dropped, while the
/// identical write on a `bit [7:0]` property landed — so the property read
/// back its pre-write value and only the alias-typed spelling disagreed.
/// Both parse shapes (`p.tx[3]` at module scope and `this.tx[3]` in a
/// method) take the same path.
#[test]
fn typedef_packed_property_selects() {
    let src = r#"
package pkg;
  typedef bit [7:0] byte_t;
  typedef bit [0:7] ascend_t;   // ascending labels mirror the bit order
endpackage
import pkg::*;

class holder;
  typedef bit [7:0] loc_t;      // class-local alias
  byte_t   tx;
  pkg::byte_t qualified;
  loc_t     loc;
  ascend_t  asc;
  bit [7:0] plain;              // the built-in control
  function void via_this();
    this.tx = 8'h00; this.tx[3] = 1'b1;
    this.asc = 8'h00; this.asc[1] = 1'b1;   // ascending: label 1 is bit 6
  endfunction
endclass

module tb;
  import pkg::*;
  holder h;
  int r_this, r_asc, r_handle, r_qualified, r_local, r_plain, r_part;
  initial begin
    h = new();
    h.via_this();
    r_this = h.tx;
    r_asc  = h.asc;
    h.tx = 8'h00; h.tx[3] = 1'b1;
    r_handle = h.tx;
    h.qualified = 8'h00; h.qualified[7] = 1'b1;
    r_qualified = h.qualified;
    h.loc = 8'h00; h.loc[0] = 1'b1;
    r_local = h.loc;
    h.plain = 8'h00; h.plain[3] = 1'b1;
    r_plain = h.plain;
    h.tx = 8'h00; h.tx[7:4] = 4'hA;
    r_part = h.tx;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r_this"), 0x08, "this-qualified write on an alias");
    assert_eq!(u(&sim, "r_asc"), 0x40, "an ascending alias maps by label");
    assert_eq!(
        u(&sim, "r_handle"),
        0x08,
        "handle-qualified write on an alias"
    );
    assert_eq!(u(&sim, "r_qualified"), 0x80, "package-qualified alias");
    assert_eq!(u(&sim, "r_local"), 0x01, "class-local alias");
    assert_eq!(u(&sim, "r_plain"), 0x08, "the built-in control still lands");
    assert_eq!(u(&sim, "r_part"), 0xA0, "a part-select on an alias");
}

/// §6.19.1 / §7.4.2: an ENUM is a packed vector of its base type, and a
/// packed array of an enum indexes its elements first. A property declared
/// `e_t [1:0] v` therefore has two 1-bit elements, and `this.v[1] = e1` must
/// write the second — the shape UVM's `uvm_config_db#(T)::get(..., this.v[i])`
/// writes through an `inout` formal. Reporting no shape (the alias case
/// above) also reported no element width, so the element write vanished.
#[test]
fn enum_typedef_packed_array_property_selects() {
    let src = r#"
package pkg;
  typedef enum bit {E_ZERO, E_ONE} e_t;
  typedef enum bit [2:0] {C_A = 3'd1, C_B = 3'd5} code_t;
endpackage
import pkg::*;

class cfg;
  e_t    [1:0] pvec;   // packed: two 1-bit elements
  e_t    uvec [1:0];   // unpacked: the workaround spelling
  code_t       code;   // 3-bit base, single element
  function void set_packed();
    this.pvec = 2'b00;
    this.pvec[0] = E_ONE;
    this.pvec[1] = E_ONE;
  endfunction
endclass

module tb;
  import pkg::*;
  cfg c;
  int r_packed, r_code, r_unpacked0, r_unpacked1, r_whole;
  initial begin
    c = new();
    c.set_packed();
    r_packed = c.pvec;
    c.uvec[0] = E_ONE;
    c.uvec[1] = E_ONE;
    r_unpacked0 = c.uvec[0];
    r_unpacked1 = c.uvec[1];
    c.code = C_A; c.code[2] = 1'b1;   // 3'b001 -> 3'b101
    r_code = c.code;
    c.pvec = 2'b00; c.pvec[1] = E_ONE;
    r_whole = c.pvec;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r_packed"), 0b11, "both enum elements written");
    assert_eq!(u(&sim, "r_unpacked0"), 1, "unpacked element 0");
    assert_eq!(u(&sim, "r_unpacked1"), 1, "unpacked element 1");
    assert_eq!(
        u(&sim, "r_code"),
        0b101,
        "the enum base is the element width"
    );
    assert_eq!(u(&sim, "r_whole"), 0b10, "element 1 alone");
}
