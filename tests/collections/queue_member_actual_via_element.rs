//! §13.5: a queue member of an object reached through an element of a
//! queue formal (`v[0].kids` inside a function whose `v` is a queue) passed
//! as a queue actual. Such a dotted name with a select on its head had no
//! storage resolution, so the callee saw an empty queue; a recursive walk
//! of an object tree saw only its first level.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
class obj; string name; obj kids[$]; endclass
module top;
  int seen = 0, deepest = 0, via_formal = -1;
  function automatic int count(obj v[$]); return v.size(); endfunction
  function automatic void first_kids(obj v[$]); via_formal = count(v[0].kids); endfunction
  function automatic void walk(obj v[$], int d);
    if (d > deepest) deepest = d;
    for (int i = 0; i < v.size(); i++) begin
      seen++;
      walk(v[i].kids, d + 1);
    end
  endfunction
  initial begin
    obj a = new, b = new, c = new, e = new;
    obj tops[$];
    a.kids.push_back(b);
    a.kids.push_back(c);
    c.kids.push_back(e);
    tops.push_back(a);
    first_kids(tops);
    walk(tops, 0);
  end
endmodule
"#;

#[test]
fn member_queue_of_formal_element_is_passed() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "via_formal"), 2);
    assert_eq!(u(&sim, "seen"), 4, "every object of the tree");
    assert_eq!(u(&sim, "deepest"), 3);
}
