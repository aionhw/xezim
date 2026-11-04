//! IEEE 1800-2023 §7.8 / §7.9 / §13.5.2: an integer-keyed associative
//! array stays integer-keyed through an `output` formal whose callee
//! declares a local with the actual's name. The caller's registration is
//! saved with its storage when the callee local shadows it and restored on
//! return; re-registering it as string-keyed made `%p` print `"-5"` and
//! `foreach` walk the keys as text. A local `int a[int]` also records its
//! key's width and signedness, so `foreach` binds a negative key as
//! negative. Expected lines are the reference simulator's output.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn int_keyed_actual_shadowed_by_callee_local() {
    let out = t_lines(
        r#"
module top;
  function automatic void fill(output int p[int]);
    int A[int];
    A[1] = 0;
    p[10] = 1; p[-5] = 2; p[2] = 3;
  endfunction
  task automatic tfill(output int p[int]);
    int A[int];
    #1;
    p[10] = 1; p[-5] = 2; p[2] = 3;
  endtask
  initial begin
    int A[int];
    int k;
    fill(A);
    $display("T| c1 A=%p num=%0d", A, A.num());
    foreach (A[i]) $display("T| c1 key %0d=%0d", i, A[i]);
    if (A.first(k)) $display("T| c1 first=%0d", k);
    A.delete();
    tfill(A);
    $display("T| c1 tA=%p", A);
    foreach (A[i]) $display("T| c1 tkey %0d=%0d", i, A[i]);
  end
endmodule
"#,
    );
    let want = [
        "T| c1 A='{-5:2, 2:3, 10:1} num=3",
        "T| c1 key -5=2",
        "T| c1 key 2=3",
        "T| c1 key 10=1",
        "T| c1 first=-5",
        "T| c1 tA='{-5:2, 2:3, 10:1}",
        "T| c1 tkey -5=2",
        "T| c1 tkey 2=3",
        "T| c1 tkey 10=1",
    ];
    assert_eq!(out, want, "{out:?}");
}

/// A class method, a blocking task and a nested call whose callee declares
/// a same-named int-keyed local.
#[test]
fn int_keyed_actual_through_method_task_and_nested_call() {
    let out = t_lines(
        r#"
class C;
  function void fill(output int p[int]);
    int A[int];
    A[7] = 7;
    p[-3] = 30; p[4] = 40;
  endfunction
  function void run();
    int A[int];
    fill(A);
    foreach (A[k]) $display("T| e1 m key %0d=%0d", k, A[k]);
  endfunction
endclass
module top;
  function automatic void inner();
    int A[int];
    A[99] = 1;
  endfunction
  function automatic void fill2(output int p[int]);
    p[-1] = 5; p[3] = 6;
    inner();
  endfunction
  task automatic tf(output int p[int]);
    int A[int];
    A[0] = 0;
    #1;
    p[-8] = 8; p[1] = 9;
  endtask
  initial begin
    int A[int];
    string S[string];
    C c = new;
    c.run();
    tf(A);
    foreach (A[k]) $display("T| e1 t key %0d=%0d", k, A[k]);
    fill2(A);
    $display("T| e1 n A=%p", A);
    inner();
    A[-20] = 1;
    begin int k; void'(A.first(k)); $display("T| e1 n2 first=%0d num=%0d", k, A.num()); end
  end
endmodule
"#,
    );
    let want = [
        "T| e1 m key -3=30",
        "T| e1 m key 4=40",
        "T| e1 t key -8=8",
        "T| e1 t key 1=9",
        "T| e1 n A='{-1:5, 3:6}",
        "T| e1 n2 first=-20 num=3",
    ];
    assert_eq!(out, want, "{out:?}");
}

#[test]
fn local_int_keyed_foreach_binds_negative_keys() {
    let out = t_lines(
        r#"
module top;
  int M[int];
  initial begin
    int B[int];
    M[-5] = 2; M[10] = 1;
    foreach (M[i]) $display("T| M key %0d=%0d", i, M[i]);
    B[-5] = 2;
    foreach (B[j]) begin $display("T| B key %0d=%0d", j, B[j]); end
    begin int k; void'(B.first(k)); $display("T| B first %0d", k); end
  end
endmodule
"#,
    );
    let want = [
        "T| M key -5=2",
        "T| M key 10=1",
        "T| B key -5=2",
        "T| B first -5",
    ];
    assert_eq!(out, want, "{out:?}");
}
