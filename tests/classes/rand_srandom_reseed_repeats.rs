//! §18.13.3, §18.14.3, §18.15: an object's RNG is seeded by `srandom()` and is
//! used exclusively by that object's `randomize()`, so reseeding the same
//! object with the same seed repeats its sequence even when other random
//! functions (`$urandom`, another object's `randomize()`) ran in between.
//! Several solver paths drew from the simulator-wide stream instead of the
//! object's stream: the joint-domain rejection pre-pass (small scalar classes
//! with a conditional constraint), unpacked-struct members, and sized or
//! `unique` dynamic arrays. Each form below reseeds and compares 10 draws.
#[test]
fn srandom_reseed_repeats_the_object_sequence() {
    let result = xezim::simulate(
        r#"
class c_cond;
  rand bit [3:0] a, b;
  constraint c { (a == 0) -> (b == 0); }
endclass
typedef struct { rand bit [7:0] x; rand bit [7:0] y; } s_t;
class c_struct;
  rand s_t s;
endclass
class c_darr;
  rand bit [7:0] q[];
  constraint k { q.size() inside {[2:6]}; foreach (q[i]) q[i] inside {[1:200]}; unique {q}; }
endclass
module t;
  c_cond   o1 = new, x1 = new;
  c_struct o2 = new, x2 = new;
  c_darr   o3 = new, x3 = new;
  string first[3][10];
  int junk;
  task automatic draw(int pass);
    for (int i = 0; i < 10; i++) begin
      string s1, s2, s3;
      void'(o1.randomize()); s1 = $sformatf("%0d %0d", o1.a, o1.b);
      void'(o2.randomize()); s2 = $sformatf("%0d %0d", o2.s.x, o2.s.y);
      void'(o3.randomize()); s3 = $sformatf("%p", o3.q);
      if (pass == 0) begin
        first[0][i] = s1; first[1][i] = s2; first[2][i] = s3;
      end else begin
        if (s1 != first[0][i]) $display("FAIL cond %0d: %s vs %s", i, s1, first[0][i]);
        if (s2 != first[1][i]) $display("FAIL struct %0d: %s vs %s", i, s2, first[1][i]);
        if (s3 != first[2][i]) $display("FAIL darray %0d: %s vs %s", i, s3, first[2][i]);
      end
    end
  endtask
  initial begin
    o1.srandom(42); o2.srandom(43); o3.srandom(44);
    draw(0);
    junk = $urandom;
    void'(x1.randomize()); void'(x2.randomize()); void'(x3.randomize());
    o1.srandom(42); o2.srandom(43); o3.srandom(44);
    draw(1);
    $display("done");
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["done"]);
}
