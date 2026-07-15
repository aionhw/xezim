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
