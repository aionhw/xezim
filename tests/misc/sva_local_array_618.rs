//! IEEE 1800-2017 §16.10, A.2.4: a property or sequence local may carry
//! unpacked dimensions (`bit flags[3:0];`, `logic [3:0] words[2];`,
//! `bit a[1:0], b[7:0];`). They were a parse error (#618, reported by
//! git-hevangel).
use xezim::simulate;

#[test]
fn local_array_declarations() {
    let out: Vec<String> = simulate(
        r#"// IEEE 1800-2017 16.10: assertion_variable_declaration ::= var_data_type
// list_of_variable_decl_assignments ; and, per A.2.4, a variable_decl_assignment is
//   variable_identifier { variable_dimension } [ = expression ]
// so a local variable of a property or sequence may be declared with unpacked
// dimensions, as any variable may: `bit flags[3:0];`, `logic [3:0] words[2];`.
//
// go is pulsed three times; done follows go by one cycle twice and is missing once.
// The property must fail once and the sequence must match twice. (The matches
// of the sequence are counted through `not`: the property `not s` fails once for
// every match of `s`.)
//
// Every line should print PASS. With the bug the file does not compile:
//   error: expected Semicolon, found LBracket '['
module t;
  logic clk = 0;
  logic go = 0, done = 0;
  int   fail_cnt = 0;     // failures of the property
  int   seq_hits = 0;     // matches of the sequence (failures of `not s`)
  int   fails = 0;

  always #5 clk = ~clk;   // posedges at 5, 15, 25, ...

  property p_go_then_done;
    bit         flags[3:0];       // unpacked range
    logic [3:0] words[2];         // unpacked size, element with a packed dimension
    @(posedge clk) go |-> ##1 done;
  endproperty

  sequence s_go_then_done;
    int counts[4];                // integer atom, unpacked size
    bit a[1:0], b[7:0];           // a list
    @(posedge clk) go ##1 done;
  endsequence

  a_go:  assert property (p_go_then_done) else fail_cnt++;
  a_seq: assert property (@(posedge clk) not s_go_then_done) else seq_hits++;

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  initial begin
    // drive on negedges. go at the posedges 25, 55, 85; done at 35 and 95, not at 65
    repeat (2) @(negedge clk);          // 10, 20
    go = 1; @(negedge clk); go = 0;     // go seen at 25
    done = 1; @(negedge clk); done = 0; // done seen at 35
    repeat (2) @(negedge clk);          // 40, 50
    go = 1; @(negedge clk); go = 0;     // go seen at 55
    @(negedge clk);                     // no done at 65
    repeat (2) @(negedge clk);          // 70, 80
    go = 1; @(negedge clk); go = 0;     // go seen at 85
    done = 1; @(negedge clk); done = 0; // done seen at 95
    repeat (3) @(negedge clk);

    check("the property failed once, for the go without done", fail_cnt == 1);
    check("the sequence matched twice",                        seq_hits == 2);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        1000,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.contains("PASS") || m.contains("FAIL"))
    .collect();
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}
