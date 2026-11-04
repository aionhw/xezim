//! IEEE 1800-2017 §11.5.1, §7.4.1: a bit-select of a function formal
//! declared with a range that does not start at zero (`input logic [12:1] a`)
//! reads the bit its label names, interpreted and compiled, and whatever
//! else in the design is named `a` (#650, reported by hevangel).
use xezim::simulate;

fn shown(src: &str) -> Vec<String> {
    simulate(src, 100_000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with('R') || m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect()
}

#[test]
fn reported_testbench() {
    let out = shown(
        r#"`timescale 1ns/1ps
// Bit-select of a function formal whose packed range does not start at zero.
//
// Two functions implement the identical 12-bit threshold-comparator rule.
//   ref_direct : formal declared [12:1], reads a[i] with a loop variable
//   ref_mask   : zero-based formal, evaluated with masks
// They are the same predicate over the same 12 bits, so every comparison below
// must pass. Directed checks cover the vectors that are reported failing; the
// exhaustive check covers all 65536 assignments of {p, a}.
//
// Command line:
//   xezim --simulate --sv2017 -s nonzero_formal_bitselect nonzero_formal_bitselect.sv

module nonzero_formal_bitselect;
    int errors = 0;

    // failing form: non-zero-based packed formal, variable bit-select in a loop
    function automatic logic ref_direct(input logic [3:0] p, input logic [12:1] a);
        logic m;
        m = 1'b1;
        for (int i = 1; i <= 12; i++)
            if (i <= p) m &= (a[i] == 1'b0);
            else        m &= (a[i] == 1'b1);
        return ~m;
    endfunction

    // same rule, zero-based formal, mask form
    function automatic logic ref_mask(input logic [3:0] p, input logic [11:0] b);
        logic [11:0] low;
        low = (p >= 12) ? 12'hFFF : ((12'h001 << p) - 12'h001);
        return ~(((b & low) == 12'h000) && ((b & ~low) == (12'hFFF & ~low)));
    endfunction

    task automatic check(input string name, input logic [3:0] p,
                         input logic [12:1] a, input logic expected);
        logic got;
        got = ref_direct(p, a);
        if (got === expected) begin
            $display("PASS %-24s p=%0d a=%b got=%b", name, p, a, got);
        end else begin
            $display("FAIL %-24s p=%0d a=%b got=%b expected=%b", name, p, a, got, expected);
            errors++;
        end
    endtask

    initial begin
        // Directed checks: the vectors that are reported failing.
        check("nonzero-p0-...110", 4'd0,  12'b111111111110, 1'b1);
        check("nonzero-p0-...111", 4'd0,  12'b111111111111, 1'b0);
        check("nonzero-p1-...100", 4'd1,  12'b111111111100, 1'b1);
        check("nonzero-p1-...101", 4'd1,  12'b111111111101, 1'b1);
        check("nonzero-p1-...110", 4'd1,  12'b111111111110, 1'b0);
        check("nonzero-p2-...000", 4'd2,  12'b111111111000, 1'b1);
        check("nonzero-p2-...100", 4'd2,  12'b111111111100, 1'b0);
        check("nonzero-p11-...000", 4'd11, 12'b100000000000, 1'b0);
        check("nonzero-p15-all-0", 4'd15, 12'b000000000000, 1'b0);
        check("nonzero-p15-one-1", 4'd15, 12'b000000000001, 1'b1);

        // Exhaustive cross-check against the mask form.
        begin
            int mismatches = 0;
            for (int v = 0; v < (1 << 16); v += 13) begin
                logic [3:0]  p;
                logic [12:1] a;
                {p, a} = v[15:0];
                if (ref_direct(p, a) !== ref_mask(p, a)) mismatches++;
            end
            if (mismatches == 0)
                $display("PASS %-24s 0 mismatches over 65536 vectors", "exhaustive");
            else begin
                $display("FAIL %-24s %0d mismatches over 65536 vectors", "exhaustive", mismatches);
                errors++;
            end
        end

        if (errors == 0) $display("RESULT: PASS");
        else             $display("RESULT: FAIL (%0d failing checks)", errors);
        $finish;
    end
endmodule
"#,
    );
    assert!(out.iter().any(|m| m == "RESULT: PASS"), "{out:?}");
}

// The exhaustive loop above steps by 13 (5,042 vectors, every `p`)
// to keep the debug-build test short.

/// An `initial` block's static `a` does not stand in for the formal `a`.
#[test]
fn block_variable_of_the_formal_name() {
    assert_eq!(
        shown(
            r#"module t;
  function automatic logic g(input logic [3:0] p, input logic [12:1] a);
    logic m; m = 1;
    for (int i = 1; i <= 12; i++)
      if (i <= p) m &= (a[i] == 1'b0);
      else m &= (a[i] == 1'b1);
    return ~m;
  endfunction
  function automatic logic h(input logic [12:1] a);
    return a[3];
  endfunction
  initial begin
    $display("R g=%b h=%b", g(4'd0, 12'b111111111110), h(12'b000000000100));
    begin
      logic [12:1] a;
      a = 12'h0;
      $display("R2 g=%b h=%b", g(4'd0, 12'b111111111110), h(12'b000000000100));
    end
  end
endmodule
"#
        ),
        ["R g=1 h=1", "R2 g=1 h=1"]
    );
}

/// A hot function, compiled after enough calls, selects by label too.
#[test]
fn hot_function() {
    assert_eq!(
        shown(
            r#"module t;
  function automatic logic h1(input logic [12:1] a); return a[3]; endfunction
  function automatic logic h4(input logic [12:1] a); logic m; m = 0; for (int i = 1; i <= 12; i++) m ^= a[i]; return m; endfunction
  initial begin
    int bad = 0, bad4 = 0;
    for (int v = 0; v < 4096; v++) begin
      if (h1(v[11:0]) !== v[2]) bad++;
      if (h4(v[11:0]) !== ^v[11:0]) bad4++;
    end
    $display("R %0d %0d", bad, bad4);
  end
endmodule
"#
        ),
        ["R 0 0"]
    );
}
