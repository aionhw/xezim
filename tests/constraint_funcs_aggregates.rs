//! Constraints-engine gaps from issue #30 — §18.4, §18.5.12 and §18.12.
//!
//! §18.5.12 — system and user-defined FUNCTIONS in constraints. A function
//! call in a constraint is a state-dependent CONSTANT: its arguments are
//! solved first (an implicit `solve … before`), the function runs once against
//! that state, and its return value is what the constraint relates to. Three
//! things were broken: an unqualified call in a constraint body never resolved
//! to a method of the enclosing class (the constraint evaluator did not push
//! the class scope, so the call silently yielded 0); a fixed unpacked-array
//! actual was bound as a scalar by the class-method path, so an array-taking
//! function always returned 0; and `$size` on a dynamic array reported a
//! packed width instead of the element count.
//!
//! §11.8.1/§11.8.2 (which §18.5.12 defers to for operand rules) — an
//! expression is UNSIGNED as soon as ANY operand is unsigned, and each operand
//! is converted to the expression's signedness BEFORE width extension. So
//! `$signed(8'hFF) + 32'd1` is 256, not 0: the signed operand is ZERO-extended
//! because the unsigned literal makes the whole expression unsigned.
//!
//! §18.4 — rand aggregates. A `rand` OBJECT HANDLE is randomized recursively
//! (the handle is never overwritten) and its constraints are solved
//! concurrently with the enclosing object's. A packed struct/union is a raw
//! integral type — one bit pattern, members aliasing slices of it (a union's
//! members all alias offset 0), and "the rules in 18.3 restricting the random
//! values of an enum variable shall not apply" to an enum member of one. An
//! unpacked struct is solved member by member.
//!
//! §18.12 — `std::randomize()` with an EMPTY variable list is a pure CHECKER:
//! every name in the `with` block is a state variable, so the call must report
//! 0 when the constraint set does not hold.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// §18.5.12: a user-defined function (scalar AND fixed-array arguments) and
/// the `$size`/`$countones`/`$clog2` system functions used inside a constraint
/// must all resolve, and the constrained variable must equal the function's
/// value computed against the solved state. Plus §11.8.2: a `$signed(...)`
/// operand in an UNSIGNED expression is zero-extended.
#[test]
fn user_and_system_functions_in_constraints() {
    const SRC: &str = r#"
class Bus;
  rand bit [31:0] data_val;
  rand int        ones_count;
  rand bit [31:0] size_bytes;
  rand bit [4:0]  clog2_result;
  rand bit [7:0]  unsigned_byte;
  rand int        signed_sum;
  rand int        signed_sum_unsigned_ctx;
  rand bit [7:0]  dyn_array[];
  rand int        array_size;
  rand bit [15:0] seed_offset;
  rand bit [15:0] user_func_res;
  rand bit [7:0]  payload_bytes[4];
  rand bit [7:0]  array_parity_res;

  static function bit [15:0] calc_hash(bit [31:0] data, bit [15:0] offset);
    calc_hash = (data[31:16] ^ data[15:0]) + offset;
  endfunction

  static function bit [7:0] calc_parity(bit [7:0] arr[4]);
    bit [7:0] running_xor = 8'h00;
    foreach (arr[i]) running_xor ^= arr[i];
    calc_parity = running_xor;
  endfunction

  constraint ranges {
    data_val   inside {[32'd1000 : 32'd50000]};
    size_bytes inside {[32'd1 : 32'd1024]};
    unsigned_byte == 8'hFF;
    seed_offset inside {[16'h1000 : 16'h2000]};
    foreach (payload_bytes[i]) payload_bytes[i] inside {[8'h00 : 8'h7F]};
  }
  constraint sys_funcs {
    ones_count   == $countones(data_val);
    clog2_result == $clog2(size_bytes);
    array_size   == $size(dyn_array);
  }
  // §11.8.2: `$signed(unsigned_byte) + 32'sd1` is a SIGNED expression
  // (-1 + 1 == 0); swapping the literal for an unsigned one makes the whole
  // expression unsigned, so 8'hFF zero-extends to 255 and the sum is 256.
  constraint sign_ctx {
    signed_sum              == $signed(unsigned_byte) + 32'sd1;
    signed_sum_unsigned_ctx == $signed(unsigned_byte) + 32'd1;
  }
  constraint user_funcs {
    user_func_res    == calc_hash(data_val, seed_offset);
    array_parity_res == calc_parity(payload_bytes);
  }
  function void pre_randomize();
    dyn_array = new[10];
  endfunction
endclass

module tb;
  int failures = 0;
  initial begin
    Bus bus = new();
    repeat (20) begin
      if (bus.randomize() != 1) failures++;
      if (bus.ones_count  != $countones(bus.data_val))  failures++;
      if (bus.clog2_result != $clog2(bus.size_bytes))   failures++;
      if (bus.array_size  != 10)                        failures++;
      if (bus.signed_sum  != 32'd0)                     failures++;
      if (bus.signed_sum_unsigned_ctx != 32'd256)       failures++;
      if (bus.user_func_res != Bus::calc_hash(bus.data_val, bus.seed_offset))
        failures++;
      if (bus.array_parity_res != Bus::calc_parity(bus.payload_bytes))
        failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.5.12: system + user-defined functions in constraints must resolve \
         against the solved state (and §11.8.2 unsigned context must not sign-extend)"
    );
}

/// §18.4: a `rand` OBJECT HANDLE is randomized recursively — the handle value
/// itself must not change — and the sub-object's constraints are solved
/// CONCURRENTLY with the enclosing object's, so a cross-object constraint that
/// re-pins a sub-object variable may not violate the sub-object's own rules.
#[test]
fn rand_object_handle_solved_concurrently() {
    const SRC: &str = r#"
class Leaf;
  rand bit [7:0] leaf_val;
  constraint leaf_rule { leaf_val > 8'd50; }
endclass

typedef struct packed {
  bit [1:0] tag;
  bit [5:0] payload;
} pkt_s;

class Bus;
  rand Leaf   leaf_inst;
  rand pkt_s  pkt;
  function new(); leaf_inst = new(); endfunction
  constraint cross_rule { leaf_inst.leaf_val == pkt.payload + 8'd10; }
endclass

module tb;
  int failures = 0;
  initial begin
    Bus bus = new();
    Leaf orig = bus.leaf_inst;
    repeat (20) begin
      if (bus.randomize() != 1) failures++;
      // Handle must be untouched by randomize().
      if (bus.leaf_inst != orig) failures++;
      // The sub-object's own constraint must still hold ...
      if (!(bus.leaf_inst.leaf_val > 8'd50)) failures++;
      // ... together with the enclosing object's cross constraint.
      if (bus.leaf_inst.leaf_val != bus.pkt.payload + 8'd10) failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.4: a rand object handle is randomized recursively (handle unchanged) \
         and its constraints are solved concurrently with the enclosing object's"
    );
}

/// §18.4: a packed struct / untagged packed union rand property is a RAW
/// INTEGRAL — its members alias slices of one bit pattern (a union's members
/// all alias offset 0), and the §18.3 enum-value restriction does NOT apply to
/// an enum member of one, so a constraint may drive it to a non-member encoding.
#[test]
fn packed_struct_and_union_members_are_raw_integral() {
    const SRC: &str = r#"
typedef enum bit [1:0] { VAL_A = 2'b00, VAL_B = 2'b01 } legal_e;

typedef struct packed {
  legal_e   nested_enum;   // only VAL_A/VAL_B are declared members
  bit [5:0] payload;
} pstruct_s;

typedef union packed {
  legal_e   union_enum;
  bit [1:0] raw_bits;      // shares storage with union_enum
} punion_u;

typedef struct {
  rand  bit [7:0] uval;
  randc bit [1:0] ucyc;
} ustruct_s;

class Bus;
  rand pstruct_s ps;
  rand punion_u  pu;
  rand ustruct_s us;
  // §18.4: 2'b11 / 2'b10 are NOT enum members — inside a packed aggregate they
  // are still legal targets.
  constraint c {
    ps.nested_enum == 2'b11;
    pu.union_enum  == 2'b10;
    us.uval        == 8'd77;
  }
endclass

module tb;
  int failures = 0;
  initial begin
    Bus bus = new();
    repeat (20) begin
      if (bus.randomize() != 1) failures++;
      if (bus.ps.nested_enum != 2'b11) failures++;
      if (bus.pu.union_enum  != 2'b10) failures++;
      // The union's members alias one another (§7.3.1).
      if (bus.pu.raw_bits    != 2'b10) failures++;
      // The packed struct's whole value is the concatenation of its fields.
      if (bus.ps != {2'b11, bus.ps.payload}) failures++;
      // Unpacked struct: solved member by member.
      if (bus.us.uval != 8'd77) failures++;
      if (!(bus.us.ucyc inside {[0:3]})) failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.4: packed struct/union members alias one raw integral and enum \
         members inside one are not restricted to declared enum values"
    );
}

/// §18.12: `std::randomize(p_struct)` must treat a packed struct as a raw
/// integral (a whole-aggregate constraint drives every field), and
/// `std::randomize()` with an EMPTY variable list is a pure checker that must
/// report 0 when the `with` block does not hold under the current state.
#[test]
fn std_randomize_packed_struct_and_scope_checker() {
    const SRC: &str = r#"
module tb;
  typedef struct packed {
    bit [3:0] field_a;
    bit [3:0] field_b;
  } pstruct_s;

  int failures = 0;
  initial begin
    pstruct_s p;
    bit [7:0] num, den;
    int status;

    // §18.4 raw integral: one constraint on the whole aggregate drives both
    // fields through the packed layout.
    status = std::randomize(p) with { p == 8'hA5; };
    if (status != 1) failures++;
    if (p.field_a != 4'hA || p.field_b != 4'h5) failures++;

    num = 8'd50;
    den = 8'd10;

    // §18.12 scope checker: no random variables — evaluate the payload only.
    status = std::randomize() with { num / den == 8'd5; };
    if (status != 1) failures++;

    status = std::randomize() with { num / den == 8'd99; };
    if (status != 0) failures++;
  end
endmodule
"#;
    let sim = simulate(SRC, 100_000).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.12: std::randomize on a packed struct resolves the raw integral, and \
         an empty-argument std::randomize() must flag an invalid comparison state"
    );
}
