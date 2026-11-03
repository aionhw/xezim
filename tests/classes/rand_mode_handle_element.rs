//! §18.8: `rand_mode(0)` on a rand class-handle property leaves the object
//! it holds as it is: when called on an inherited handle from a derived
//! class's method, randomize() still succeeds and the object keeps its
//! values; when called on one element of a fixed array of rand handles,
//! only that element's object is left out, and the other element is still
//! randomized under its own constraints. The derived class randomizes the
//! elements of the inherited array at all: looked up in the derived class
//! alone, the array was taken for a plain handle holding no object, so
//! randomize() failed on every call with the elements left as they were.
#[test]
fn rand_mode_off_on_inherited_handle_and_on_an_array_element() {
    let result = xezim::simulate(
        r#"
class sub_c;
  rand bit [7:0] a, b;
  constraint gt_c { a > b; }
endclass
class base_c;
  rand sub_c sub;
  rand sub_c subs[2];
  rand bit [7:0] x;
  constraint x_c { x inside {[1:10]}; }
  function new(); sub = new(); subs[0] = new(); subs[1] = new(); endfunction
  function void off_arr(); subs[1].rand_mode(0); endfunction
  function int q_arr(); return subs[1].rand_mode(); endfunction
endclass
class der_c extends base_c;
  rand sub_c q[$];
  function new(); sub_c e = new(); q.push_back(e); endfunction
  function void off_sub(); sub.rand_mode(0); endfunction
  function void on_sub(); sub.rand_mode(1); endfunction
  function int q_sub(); return sub.rand_mode(); endfunction
  function int q_this(); return this.sub.rand_mode(); endfunction
endclass
module t;
  initial begin
    der_c d = new();
    int ok, good = 0;
    repeat (5) begin
      d.subs[0].a = 0; d.subs[0].b = 0; d.subs[1].a = 0; d.subs[1].b = 0; d.q[0].a = 0; d.q[0].b = 0;
      if (d.randomize() && d.subs[0].a > d.subs[0].b && d.subs[1].a > d.subs[1].b && d.q[0].a > d.q[0].b)
        good++;
    end
    $display("inherited array good %0d", good);
    d.off_sub();
    $display("der sub %0d this %0d out %0d a %0d", d.q_sub(), d.q_this(), d.sub.rand_mode(), d.sub.a.rand_mode());
    d.sub.a = 0; d.sub.b = 0;
    ok = d.randomize();
    $display("der rand ok %0d a %0d b %0d xin %0d", ok, d.sub.a, d.sub.b, d.x inside {[1:10]});
    d.on_sub();
    $display("der on sub %0d a %0d", d.sub.rand_mode(), d.sub.a.rand_mode());
    d.off_arr();
    $display("arr s1 %0d s0 %0d q %0d a1 %0d", d.subs[1].rand_mode(), d.subs[0].rand_mode(), d.q_arr(), d.subs[1].a.rand_mode());
    d.subs[1].a = 0; d.subs[1].b = 0; d.subs[0].a = 0; d.subs[0].b = 0;
    ok = d.randomize();
    $display("arr rand ok %0d a1 %0d b1 %0d s0gt %0d", ok, d.subs[1].a, d.subs[1].b, d.subs[0].a > d.subs[0].b);
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
    assert_eq!(
        lines,
        [
            "inherited array good 5",
            "der sub 0 this 0 out 0 a 1",
            "der rand ok 1 a 0 b 0 xin 1",
            "der on sub 1 a 1",
            "arr s1 0 s0 1 q 0 a1 1",
            "arr rand ok 1 a1 0 b1 0 s0gt 1"
        ]
    );
}
