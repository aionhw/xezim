//! IEEE 1800-2023 §7.4.5, §6.18: a class property whose element type is an
//! unpacked-array typedef and which has dimensions of its own
//! (`q_t qq[$];`) holds rows of the typedef's shape, from a class, module,
//! `$unit`, imported or package (`p::q_t`, §26.3) typedef, in a plain or a
//! parameterized class, static or not (aionhw/xezim-core#216). The values
//! were checked on the reference simulator.
use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let result = simulate(src, 1_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn class_typedef_rows_take_pushed_queues() {
    let lines = run(r#"
class c;
  typedef int q_t[$];
  q_t qq[$];
  function void run();
    q_t tmp;
    tmp = {1, 2};
    qq.push_back(tmp);
    qq.push_back('{3, 4, 5});
    $display("R n=%0d s0=%0d s1=%0d v=%0d %0d", qq.size(), qq[0].size(), qq[1].size(), qq[0][1], qq[1][2]);
  endfunction
endclass
module t;
  initial begin automatic c o = new(); o.run(); end
endmodule
"#);
    assert_eq!(lines, ["R n=2 s0=2 s1=3 v=2 5"]);
}

#[test]
fn package_unit_and_parameterized_class_typedef_rows() {
    let lines = run(r#"
package p;
  typedef int q_t[$];
endpackage
typedef int uq_t[$];
class a_c;
  p::q_t pq[$];
  function void run();
    pq.push_back('{6, 7});
    $display("R a n=%0d s0=%0d v=%0d", pq.size(), pq[0].size(), pq[0][1]);
  endfunction
endclass
class b_c #(type T = int);
  typedef T row_t[$];
  row_t rows[$];
  static row_t srows[$];
  p::q_t pq[$];
  uq_t uq[2];
  function void run();
    row_t tmp;
    tmp = {1, 2};
    rows.push_back(tmp);
    srows.push_back('{3, 4, 5});
    pq.push_back('{6, 7});
    uq[1].push_back(8);
    $display("R b rows n=%0d s0=%0d v=%0d", rows.size(), rows[0].size(), rows[0][1]);
    $display("R b srows n=%0d s0=%0d v=%0d", srows.size(), srows[0].size(), srows[0][2]);
    $display("R b pq n=%0d s0=%0d v=%0d", pq.size(), pq[0].size(), pq[0][1]);
    $display("R b uq s1=%0d v=%0d", uq[1].size(), uq[1][0]);
  endfunction
endclass
import p::*;
class c_c;
  q_t iq[$];
  function void run();
    iq.push_back('{8, 9});
    $display("R c n=%0d s0=%0d v=%0d", iq.size(), iq[0].size(), iq[0][1]);
  endfunction
endclass
module t;
  initial begin
    a_c a = new; b_c #(int) b = new; c_c c = new;
    a.run(); b.run(); c.run();
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R a n=1 s0=2 v=7",
            "R b rows n=1 s0=2 v=2",
            "R b srows n=1 s0=3 v=5",
            "R b pq n=1 s0=2 v=7",
            "R b uq s1=1 v=8",
            "R c n=1 s0=2 v=9",
        ]
    );
}
