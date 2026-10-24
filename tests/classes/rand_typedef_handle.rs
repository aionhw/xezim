//! §18.4: `randomize()` does not replace a rand class handle. A typedef
//! of a class (`typedef inner#(.W(194)) inner_t`, `typedef plain plain_t`)
//! is not a key in the class table, so the member was drawn as an integer
//! and the object created in `new` was lost. The pointed-to object is
//! randomized instead, including its own constraints.

use xezim::simulate;

const SRC: &str = r#"
class inner #(int W = 1);
  int tag;
  rand bit [7:0] n;
  constraint c { n inside {[1:3]}; }
  function new();
    tag = W;
  endfunction
endclass

typedef inner#(.W(194)) inner_t;

class plain;
  int tag;
  rand bit [7:0] n;
  constraint c { n inside {[4:6]}; }
  function new();
    tag = 7;
  endfunction
endclass

typedef plain plain_t;

class cfg;
  rand inner_t p;
  rand plain q;
  rand plain_t r;
  function new();
    p = new;
    q = new;
    r = new;
  endfunction
endclass

module t;
  initial begin
    cfg c = new;
    int ok;
    ok = c.randomize();
    $display("ok=%0d ptag=%0d pn=%0d qtag=%0d qn=%0d rtag=%0d rn=%0d",
      ok, c.p.tag, c.p.n, c.q.tag, c.q.n, c.r.tag, c.r.n);
  end
endmodule
"#;

#[test]
fn rand_typedef_handle_keeps_its_object() {
    let out: Vec<String> = simulate(SRC, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(out.len(), 1, "{out:?}");
    let line = &out[0];
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(fields[0], "ok=1", "{line}");
    assert_eq!(fields[1], "ptag=194", "{line}");
    let pn: i32 = fields[2].strip_prefix("pn=").unwrap().parse().unwrap_or(-1);
    assert!((1..=3).contains(&pn), "{line}");
    assert_eq!(fields[3], "qtag=7", "{line}");
    let qn: i32 = fields[4].strip_prefix("qn=").unwrap().parse().unwrap_or(-1);
    assert!((4..=6).contains(&qn), "{line}");
    assert_eq!(fields[5], "rtag=7", "{line}");
    let rn: i32 = fields[6].strip_prefix("rn=").unwrap().parse().unwrap_or(-1);
    assert!((4..=6).contains(&rn), "{line}");
}
