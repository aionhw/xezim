//! §6.21, §23.6: two blocks of one process declare a handle under the same
//! name, so each is renamed to storage of its own. The rename gave every
//! rewritten reference the declaration's source span, and a cache keyed by
//! span answered `p.b.size()` with `p.a`'s member: both arrays read one
//! length, and a class whose three arrays were sized 3/5/4 by `randomize()`
//! read 3/3/3. Expected values are the reference simulator's.

use xezim::simulate;

const MEMBERS: &str = r#"
class Q; int a[]; int b[]; string s = "q"; endclass
class R; int a[]; int q[$]; endclass
module tb;
  initial begin
    begin
      Q p = new;
      p.a = new[3]; p.b = new[5];
      $display("T|one %0d %0d %s", p.a.size(), p.b.size(), p.s);
    end
    begin
      R p = new;
      p.a = new[2]; p.q = '{1, 2, 3, 4, 5, 6, 7};
      $display("T|two %0d %0d", p.a.size(), p.q.size());
    end
  end
endmodule
"#;

const RANDOMIZED: &str = r#"
class C3; rand bit [7:0] a[], b[], cc[]; constraint s { a.size() == 3; b.size() == 5; cc.size() == 4; } endclass
class M2; rand bit [7:0] a[], q[$]; constraint c { a.size() == 3; q.size() == 5; } endclass
module tb;
  initial begin
    begin
      C3 p = new();
      void'(p.randomize());
      $display("T|three %0d %0d %0d", p.a.size(), p.b.size(), p.cc.size());
    end
    begin
      M2 p = new();
      void'(p.randomize());
      $display("T|mixed %0d %0d", p.a.size(), p.q.size());
    end
  end
endmodule
"#;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn renamed_handles_read_each_member() {
    assert_eq!(lines(MEMBERS), ["T|one 3 5 q", "T|two 2 7"]);
}

#[test]
fn renamed_handles_read_each_randomized_size() {
    assert_eq!(lines(RANDOMIZED), ["T|three 3 5 4", "T|mixed 3 5"]);
}
