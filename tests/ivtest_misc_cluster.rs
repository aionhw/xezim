//! Language/behavioral fixes recovered from the ivtest regression suite
//! (integer types, enums, dynamic arrays, var-init, and misc clusters).
//! Each case embeds a representative reduction of the ivtest source and
//! asserts the self-checking "PASSED" marker. Sources themselves are not
//! vendored.

use xezim::simulate;

fn passes(src: &str) -> bool {
    match simulate(src, 200_000) {
        Ok(sim) => {
            let out: String = sim
                .output
                .iter()
                .map(|o| o.message.clone())
                .collect::<Vec<_>>()
                .join("\n");
            out.contains("PASSED") && !out.contains("FAILED")
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Integer 2-state types (§6.11): a 2-state destination drops X/Z on every
// write, including a continuous-assign copy driven by a 4-state (X at time 0)
// source. Covers ibit_test / ibyte_test / iint_test / i*/s* pairs.
// ---------------------------------------------------------------------------
#[test]
fn two_state_contassign_copy_drops_xz() {
    assert!(passes(
        r#"
module t;
  reg [14:0] ar;
  bit unsigned [14:0] bu;
  int unsigned iu;
  reg [31:0] r32;
  assign bu = ar;
  assign iu = r32;
  initial begin
    // time-0: 2-state nets read 0 even though their 4-state driver is X
    if (bu !== 15'b0 || iu !== 32'b0) begin
      $display("FAILED time0 %b %b", bu, iu); $finish;
    end
    ar = 15'h1234; r32 = 32'hdead_beef; #1;
    if (bu === 15'h1234 && iu === 32'hdead_beef) $display("PASSED");
    else $display("FAILED got %h %h", bu, iu);
    #1 $finish;
  end
endmodule
"#
    ));
}

// ---------------------------------------------------------------------------
// Enum methods (§6.19.6): num/first/last/next/prev on both typedef'd and
// anonymous enums, written with AND without parentheses; enum element ranges
// (`P[5] = 12, Q, S[3] = 88`). Covers enum_elem_ranges / enum_next /
// enum_value_expr.
// ---------------------------------------------------------------------------
#[test]
fn enum_methods_ranges_and_no_paren() {
    assert!(passes(
        r#"
module t;
  enum { P[5] = 12, Q, S[3] = 88 } par_enum;      // anonymous, ranged
  enum { RED, GREEN = 2, BLUE } color1;           // for next/prev walk
  initial begin
    // element ranges elaborate
    if (P0 != 12 || P4 != 16 || Q != 17 || S0 != 88 || S2 != 90) begin
      $display("FAILED ranges P0=%0d P4=%0d Q=%0d S0=%0d", P0, P4, Q, S0); $finish;
    end
    // num, no parens
    if (par_enum.num != 9) begin $display("FAILED num %0d", par_enum.num); $finish; end
    // first/last no parens
    if (par_enum.first != 12 || par_enum.last != 90) begin
      $display("FAILED first/last %0d %0d", par_enum.first, par_enum.last); $finish;
    end
    // next/prev walk with wrap
    color1 = RED;
    color1 = color1.next;
    if (color1 != GREEN) begin $display("FAILED next %0d", color1); $finish; end
    color1 = color1.next;
    if (color1 != BLUE || color1 != color1.last) begin $display("FAILED next2"); $finish; end
    color1 = color1.prev;
    if (color1 != GREEN) begin $display("FAILED prev"); $finish; end
    $display("PASSED");
    #1 $finish;
  end
endmodule
"#
    ));
}

// ---------------------------------------------------------------------------
// Dynamic arrays (§7.5.1): `new[n](arg)` where arg is an assignment pattern
// (array literal) or a scalar broadcast, and passing a dynamic array as a
// function argument. Covers sv_darray_args2/2b/3/4.
// ---------------------------------------------------------------------------
#[test]
fn dynamic_array_new_with_pattern_and_scalar_args() {
    assert!(passes(
        r#"
program main;
  function real sum_array(real array[]);
    int idx;
    sum_array = 0.0;
    for (idx = 0; idx < array.size(); idx = idx+1) sum_array = sum_array + array[idx];
  endfunction
  real obj[];
  real foo;
  initial begin
    obj = new[3] ('{4.0, 5.0, 6.0});   // pattern arg
    foo = sum_array(obj);
    if (foo != 15.0) begin $display("FAILED pattern %0f", foo); $finish; end
    obj = new[3] (3.0);                // scalar broadcast
    foo = sum_array(obj);
    if (foo != 9.0) begin $display("FAILED scalar %0f", foo); $finish; end
    $display("PASSED");
  end
endprogram
"#
    ));
}

// ---------------------------------------------------------------------------
// §12.7/§6.21: a for-loop-declared variable is automatic and shadows a
// same-named outer signal (it must not clobber it or borrow its width).
// Covers sv_for_variable.
// ---------------------------------------------------------------------------
#[test]
fn for_loop_local_var_shadows_outer() {
    assert!(passes(
        r#"
program main;
  int sum;
  logic idx;               // outer, 1-bit
  initial begin
    sum = 0;
    idx = 1'bx;
    for (int idx = 0; idx < 8; idx += 1) sum += idx;   // local int idx
    if (sum != 28) begin $display("FAILED sum=%0d", sum); $finish; end
    if (idx !== 1'bx) begin $display("FAILED outer idx=%b", idx); $finish; end
    $display("PASSED");
  end
endprogram
"#
    ));
}

// ---------------------------------------------------------------------------
// §6.12.2: an integral actual bound to a `real` formal converts to floating
// point so real arithmetic in the body is done in the real domain.
// ---------------------------------------------------------------------------
#[test]
fn real_formal_converts_integral_actual() {
    assert!(passes(
        r#"
module t;
  function real div2(input real x);
    x /= 2;
    return x;
  endfunction
  initial begin
    if (div2(5) == 2.5) $display("PASSED");
    else $display("FAILED got %0f", div2(5));
  end
endmodule
"#
    ));
}
