//! §12.7.3/§7.4.5: `foreach` over ONE ELEMENT of a class property that is a
//! fixed array of collections (`foreach (q[i][j][k])` on `int q[3][3][$]`).
//! The loop must walk that element collection's own size or keys. It walked
//! the outer fixed bound instead (3 phantom iterations per element), or a
//! single iteration through a handle (`foreach (h.q[1][k])`). Expected values
//! from the reference simulator.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

#[test]
fn foreach_over_queue_element_of_class_array() {
    let msgs = messages(
        "class item; int v; function new(int x); v = x; endfunction endclass
class sb;
  item expected [3][3][$];
  function void go();
    int n, s; item t;
    t = new(7);  expected[0][1].push_back(t);
    t = new(9);  expected[2][0].push_back(t);
    t = new(10); expected[2][0].push_back(t);
    n = 0; s = 0;
    foreach (expected[i, o]) foreach (expected[i][o][k]) begin n++; s += expected[i][o][k].v; end
    $display(\"nested n=%0d s=%0d\", n, s);
    n = 0; foreach (expected[2][0][k]) n++;
    $display(\"const20 n=%0d\", n);
    n = 0; foreach (expected[0][1][k]) n++;
    $display(\"const01 n=%0d\", n);
    n = 0; foreach (expected[1][1][k]) n++;
    $display(\"empty n=%0d\", n);
  endfunction
endclass
module tb;
  initial begin
    sb s = new;
    s.go();
    $finish;
  end
endmodule",
    );
    assert_line(&msgs, "nested n=3 s=26");
    assert_line(&msgs, "const20 n=2");
    assert_line(&msgs, "const01 n=1");
    assert_line(&msgs, "empty n=0");
}

#[test]
fn foreach_over_element_through_handle_and_inherited() {
    let msgs = messages(
        "class base; int q [2][3][$]; endclass
class c extends base;
  int q1 [3][$];
  function void fill();
    q[1][2].push_back(10); q[1][2].push_back(20); q[0][0].push_back(5);
    q1[1].push_back(5); q1[1].push_back(6);
  endfunction
  function int inh();
    int s = 0;
    foreach (q[1][2][k]) s += q[1][2][k] * (k + 1);
    return s;
  endfunction
endclass
module tb;
  initial begin
    c h = new; int n;
    h.fill();
    n = 0; foreach (h.q1[1][k]) n++;
    $display(\"handle1 n=%0d\", n);
    n = 0; foreach (h.q[1][2][k]) n++;
    $display(\"handle2 n=%0d\", n);
    $display(\"inherited s=%0d\", h.inh());
    $finish;
  end
endmodule",
    );
    assert_line(&msgs, "handle1 n=2");
    assert_line(&msgs, "handle2 n=2");
    assert_line(&msgs, "inherited s=50");
}

#[test]
fn foreach_over_dyn_and_assoc_elements_of_class_array() {
    let msgs = messages(
        "class c;
  int d [3][];
  int a [3][int];
  function void go();
    int n, s;
    d[1] = new[4];
    foreach (d[1][k]) d[1][k] = k + 1;
    n = 0; s = 0; foreach (d[1][k]) begin n++; s += d[1][k]; end
    $display(\"dyn n=%0d s=%0d\", n, s);
    a[2][7] = 70; a[2][3] = 30; a[0][1] = 1;
    n = 0; s = 0; foreach (a[2][key]) begin n++; s += a[2][key] + key; end
    $display(\"assoc n=%0d s=%0d\", n, s);
    n = 0; foreach (a[1][key]) n++;
    $display(\"assoc_empty n=%0d\", n);
  endfunction
endclass
module tb;
  initial begin
    c h = new;
    h.go();
    $finish;
  end
endmodule",
    );
    assert_line(&msgs, "dyn n=4 s=10");
    assert_line(&msgs, "assoc n=2 s=110");
    assert_line(&msgs, "assoc_empty n=0");
}

#[test]
fn foreach_over_element_with_blocking_body() {
    let msgs = messages(
        "class c;
  int q [3][3][$];
  task run();
    int n;
    q[2][0].push_back(1); q[2][0].push_back(2); q[0][1].push_back(3);
    n = 0; foreach (q[2][0][k]) begin #1; n++; end
    $display(\"blk n=%0d t=%0t\", n, $time);
    n = 0; foreach (q[i, o]) foreach (q[i][o][k]) begin #1; n += q[i][o][k]; end
    $display(\"blk_nested s=%0d t=%0t\", n, $time);
  endtask
endclass
module tb;
  initial begin
    c h = new;
    h.run();
    $finish;
  end
endmodule",
    );
    assert_line(&msgs, "blk n=2 t=2");
    assert_line(&msgs, "blk_nested s=6 t=5");
}
