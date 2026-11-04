//! IEEE 1800-2017 §18.8: `rand_mode` on an element of an unpacked rand
//! array (`obj.arr[i].rand_mode(0)`, or `arr[i]` inside the class) makes that
//! element a state variable: `randomize()` keeps its value, the other
//! elements stay random, and the constraints see the kept value. The query
//! form reads the element's mode, and `arr.rand_mode(1)` enables every
//! element again. The element form aborted the run with a null-handle error
//! (#769).

use xezim::simulate;

fn tagged(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T "))
        .collect()
}

#[test]
fn an_element_whose_rand_mode_is_off_keeps_its_value() {
    let out = tagged(
        r#"
class fc;
  rand bit [7:0] arr[4];
  function void pin(int i); arr[i].rand_mode(0); endfunction
endclass
class oc;
  rand bit [7:0] arr[4];
  constraint up { foreach (arr[i]) if (i > 0) arr[i] > arr[i-1]; }
endclass
class dc;
  rand bit [7:0] arr[];
  constraint sz { arr.size() == 3; }
endclass
module tb;
  initial begin
    fc f = new; oc o = new; dc d = new;
    int same, moved, ok;
    foreach (f.arr[i]) f.arr[i] = 8'h40 + i;
    f.arr[2].rand_mode(0);
    $display("T q %0d %0d", f.arr[2].rand_mode(), f.arr[1].rand_mode());
    for (int k = 0; k < 20; k++) begin
      void'(f.randomize());
      if (f.arr[2] == 8'h42) same++;
      if (f.arr[0] != 8'h40 || f.arr[1] != 8'h41 || f.arr[3] != 8'h43) moved++;
    end
    $display("T pinned %0d others moved %0d", same, moved > 0);
    f.arr.rand_mode(1);
    $display("T reenabled %0d", f.arr[2].rand_mode());
    moved = 0;
    for (int k = 0; k < 20; k++) begin void'(f.randomize()); if (f.arr[2] != 8'h42) moved++; end
    $display("T reenabled moves %0d", moved > 0);
    f.arr[3] = 8'h07; f.pin(3); same = 0;
    for (int k = 0; k < 20; k++) begin void'(f.randomize()); if (f.arr[3] == 8'h07) same++; end
    $display("T in-class pin %0d", same);
    o.arr[2] = 8'h80; o.arr[2].rand_mode(0);
    for (int k = 0; k < 20; k++) if (o.randomize() && o.arr[2] == 8'h80 && o.arr[1] < 8'h80 && o.arr[3] > 8'h80) ok++;
    $display("T ordered %0d", ok);
    void'(d.randomize()); d.arr[1] = 8'h77; d.arr[1].rand_mode(0); same = 0;
    for (int k = 0; k < 20; k++) begin void'(d.randomize()); if (d.arr[1] == 8'h77 && d.arr.size() == 3) same++; end
    $display("T dynamic %0d", same);
  end
endmodule
"#,
    );
    let want = [
        "T q 0 1",
        "T pinned 20 others moved 1",
        "T reenabled 1",
        "T reenabled moves 1",
        "T in-class pin 20",
        "T ordered 20",
        "T dynamic 20",
    ];
    assert_eq!(out, want, "{out:#?}");
}
