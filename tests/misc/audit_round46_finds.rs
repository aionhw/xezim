//! Audit round 46 — reference-verified divergences (scratchpad/audit46).

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 10_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// §6.16.6: compare returns the BYTE DIFFERENCE at the first mismatch
/// ("Hello".compare("hello") = 'H'-'h' = -32), ±1 by length when one
/// string is a prefix of the other. The reference does exactly this;
/// sign-only -1/0/1 is not enough.
#[test]
fn string_compare_returns_char_difference() {
    let msgs = out(r#"
module test;
  initial begin
    string s = "Hello";
    $display("T|%0d %0d %0d %0d", s.compare("hello"), s.compare("Hellp"),
             s.compare("Hell"), s.icompare("HELLO"));
  end
endmodule
"#);
    assert!(
        msgs.iter().any(|m| m == "T|-32 -1 1 0"),
        "strcmp-style differences; got {:?}",
        msgs
    );
}

/// §13.5.2: a ref array formal named like the caller's actual is an
/// IDENTITY binding — post-call cleanup must not purge the shared
/// storage (the caller's array read back all-x).
#[test]
fn ref_array_identity_binding_survives_call() {
    let msgs = out(r#"
module test;
  task automatic bump(ref int arr[3]);
    for (int i = 0; i < 3; i++) arr[i] += 10;
  endtask
  initial begin
    int arr[3] = '{1, 2, 3};
    bump(arr);
    $display("T|%0d %0d %0d p=%p", arr[0], arr[1], arr[2], arr);
  end
endmodule
"#);
    assert!(
        msgs.iter().any(|m| m == "T|11 12 13 p='{11, 12, 13}"),
        "identity ref-array binding; got {:?}",
        msgs
    );
}

/// §11.8.2: both literal and variable operands divide as integers before
/// their result is compared to a real.
#[test]
fn real_context_literal_subtree_folding() {
    let msgs = out(r#"
module test;
  initial begin
    int a = 7, b = 2;
    real x = 3.5;
    $display("T|%0d %0d %g", 3.5 == 7/2, x == a/b, x + a/b);
  end
endmodule
"#);
    assert!(
        msgs.iter().any(|m| m == "T|0 0 6.5"),
        "integer division before real conversion; got {:?}",
        msgs
    );
}
