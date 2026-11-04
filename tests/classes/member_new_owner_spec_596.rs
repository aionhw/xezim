//! IEEE 1800-2017 §8.25: `other.h = new()`, `h` declared `T h` in
//! `class W #(type T)`, builds the class `other`'s specialization binds for
//! `T`, not the one of the specialization running the method (#596,
//! reported by akashlevy). Also an element `other.arr[0] = new()`, a
//! derived class of a specialization, and `T` bound to a parameterized
//! class.
use xezim::simulate;

fn shown(src: &str, prefixes: &[&str]) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| prefixes.iter().any(|p| m.starts_with(p)))
        .collect()
}

#[test]
fn reported_testbench() {
    let src = r#"// §8.25: `other.h = new()` builds the class `other`'s specialization binds
// for T, not the one of the specialization running the method.
class A;
  int x;
  function new(); x = 1; endfunction
endclass
class B;
  int x;
  function new(); x = 2; endfunction
endclass
class W #(type T = A);
  T h;
  function void build(W #(B) other);
    other.h = new();
  endfunction
endclass
module t;
  initial begin
    W #(A) wa = new;
    W #(B) wb = new;
    wa.build(wb);
    $display("wb.h.x=%0d expect 2", wb.h.x);
  end
endmodule

"#;
    assert_eq!(shown(src, &["wb."]), ["wb.h.x=2 expect 2"]);
}

#[test]
fn member_targets_bind_through_their_owner() {
    let src = r#"class A; int x; function new(); x = 1; endfunction endclass
class B; int x; function new(); x = 2; endfunction endclass
class P #(int N = 5); int x; function new(); x = N; endfunction endclass
class W #(type T = A);
  T h; T arr[2];
  function void self_build(); h = new(); this.arr[1] = new(); endfunction
  function void build(W #(B) other); other.h = new(); other.arr[0] = new(); endfunction
  function void build_p(W #(P#(7)) other); other.h = new(); endfunction
endclass
class X extends W #(B); endclass
module t;
  initial begin
    W #(A) wa = new; W #(B) wb = new; X xx = new; W #(P#(7)) wp = new;
    wa.self_build(); wb.self_build();
    $display("A %0d %0d %0d %0d", wa.h.x, wa.arr[1].x, wb.h.x, wb.arr[1].x);
    wa.build(wb); $display("B %0d %0d", wb.h.x, wb.arr[0].x);
    wa.build(xx); $display("C %0d %0d", xx.h.x, xx.arr[0].x);
    wa.build_p(wp); $display("D %0d", wp.h.x);
    wb.h = new(); wa.h = new(); $display("E %0d %0d", wb.h.x, wa.h.x);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["A ", "B ", "C ", "D ", "E "]),
        ["A 1 1 2 2", "B 2 2", "C 2 2", "D 7", "E 2 1"]
    );
}
