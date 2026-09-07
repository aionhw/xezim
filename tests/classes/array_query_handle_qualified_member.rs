//! §20.7: an array query on a HANDLE-QUALIFIED class member — `$size(a.M)` —
//! must report the member's declared element count, exactly as the in-method
//! `$size(M)` on the same member does.
//!
//! The query path only ever resolved a plain `Ident`. The qualified spelling
//! parses either as a `MemberAccess`, which never entered that arm, or as a
//! two-segment `Ident` that resolves to a name with no registered shape — so
//! no bounds were found and the query fell through to the operand's evaluated
//! BIT WIDTH. Every `$size(a.M)` reported 32, and `$left`/`$high` reported 31.
//!
//! The shape was known all along: `new` registers each fixed-size array
//! property under `<handle>#<member>` in the `arrays*` tables, which is why
//! `foreach (a.M[i])` iterates the member correctly. `handle_member_array_name`
//! resolves the operand to that same store, and the ordinary declared-bounds
//! path answers the query.
//!
//! Expected values are the reference simulator's, with one exception noted on
//! the descending case below.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn query_on_handle_qualified_member_array() {
    const SRC: &str = r#"
module top;
  class C;
    int A[0:2];
    int M[2:0];
    int W[0:1][0:3];
    function void in_method();
      $display("IN %0d %0d %0d %0d", $size(A), $left(A), $right(A), $size(M));
    endfunction
  endclass
  initial begin
    automatic C a = new;
    int n = 0;
    a.in_method();
    $display("Q %0d %0d %0d %0d", $size(a.A), $left(a.A), $right(a.A), $size(a.M));
    $display("N %0d %0d", $size(a.W, 1), $size(a.W, 2));
    foreach (a.A[i]) n++;
    $display("F %0d", n);
    $finish;
  end
endmodule
"#;
    let o = out(SRC);
    // The in-method form is the behaviour the qualified form has to match.
    assert!(o.contains("IN 3 0 2 3"), "in-method queries:\n{}", o);
    assert!(
        o.contains("Q 3 0 2 3"),
        "handle-qualified queries must match the in-method ones, not report \
         the operand's 32-bit width:\n{}",
        o
    );
    assert!(o.contains("N 2 4"), "per-dimension query on a 2-D member:\n{}", o);
    assert!(o.contains("F 3"), "foreach over the same member still iterates 3:\n{}", o);
}

/// `this.M` resolves through the current handle, and an inherited member
/// resolves by walking the runtime class's ancestry.
#[test]
fn query_on_this_and_inherited_member() {
    const SRC: &str = r#"
module top;
  class B;
    int M[0:4];
  endclass
  class D extends B;
    function void probe();
      $display("T %0d", $size(this.M));
    endfunction
  endclass
  initial begin
    automatic D d = new;
    d.probe();
    $display("I %0d", $size(d.M));
    $finish;
  end
endmodule
"#;
    let o = out(SRC);
    assert!(o.contains("T 5"), "$size(this.M):\n{}", o);
    assert!(o.contains("I 5"), "$size on an INHERITED member through a handle:\n{}", o);
}

/// A non-array member keeps the bit-width answer: `$size` of a packed field is
/// its width, and the resolution must not hijack a name that is not an array.
#[test]
fn query_on_non_array_member_is_unchanged() {
    const SRC: &str = r#"
module top;
  class C;
    bit [88:0] big;
  endclass
  initial begin
    automatic C c = new;
    $display("B %0d", $size(c.big));
    $finish;
  end
endmodule
"#;
    let o = out(SRC);
    assert!(o.contains("B 89"), "packed member keeps its bit width:\n{}", o);
}
