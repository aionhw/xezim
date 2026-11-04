//! IEEE 1800-2017 §10.4.2, §4.9.4: a nonblocking assignment to a member
//! or element evaluates its target when the statement executes. The object
//! the root names then is updated, whether the root is a class property, a
//! module or block variable, or a formal, and whatever it names afterwards
//! (#613, reported by git-hevangel).
use xezim::simulate;

#[test]
fn nba_root_object_is_fixed_at_execution() {
    let out: Vec<String> = simulate(
        r#"// IEEE 1800-2017 10.4.2: a nonblocking assignment evaluates its target (an
// index expression, a class handle, a virtual interface reference) when the
// statement executes; only the update waits for the NBA region (4.9.4). So
// `h.x <= v;` updates the object `h` named at that moment, even if `h` is
// assigned another handle (or null) before the update is applied. Every line
// should print PASS.
class pkt;
  bit [7:0] sc;
  bit [7:0] arr [0:3];
endclass

class holder;
  pkt held;
  // the handle is a class property, reassigned right after the NBA
  task by_property(pkt other);
    held.sc <= 8'h91;
    held = other;
  endtask
  // the same with a delayed update
  task by_property_delayed(pkt other);
    held.sc <= #3 8'h92;
    held = other;
  endtask
  // a part-select of an element
  task by_property_part(pkt other);
    held.arr[2][3:0] <= 4'ha;
    held = other;
  endtask
  // the handle is a formal, reassigned right after the NBA
  task by_formal(pkt tr, pkt other);
    tr.sc <= 8'h93;
    tr = other;
  endtask
endclass

module t;
  pkt  mv;                                  // a module variable
  pkt  a, b, c;
  holder h = new;
  int fails = 0;

  task automatic chk(string what, logic [7:0] got, logic [7:0] exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %h, expected %h", what, got, exp); fails++; end
  endtask

  initial begin
    a = new; b = new; c = new;

    // 1: a class property reassigned after the NBA
    h.held = a;  h.by_property(b);
    #1;
    chk("property: old object updated",       a.sc, 8'h91);
    chk("property: new object untouched",     b.sc, 8'h00);

    // 2: the same with a delayed update
    h.held = a;  a.sc = 0;  h.by_property_delayed(b);
    #5;
    chk("property, <= #3: old object updated", a.sc, 8'h92);
    chk("property, <= #3: new object untouched", b.sc, 8'h00);

    // 3: a part-select of an element
    h.held = a;  h.by_property_part(c);
    #1;
    chk("property, part-select: old object",  a.arr[2], 8'h0a);
    chk("property, part-select: new object",  c.arr[2], 8'h00);

    // 4: a module variable reassigned to a new object
    a = new;  mv = a;
    mv.sc <= 8'h33;
    mv.arr[1] <= 8'h44;
    mv = new;
    #1;
    chk("module variable: old object sc",     a.sc,     8'h33);
    chk("module variable: old object arr[1]", a.arr[1], 8'h44);
    chk("module variable: new object sc",     mv.sc,    8'h00);
    chk("module variable: new object arr[1]", mv.arr[1], 8'h00);

    // 5: a module variable set to null
    a = new;  mv = a;
    mv.sc <= 8'h55;
    mv = null;
    #1;
    chk("module variable set to null: old object", a.sc, 8'h55);

    // 6: a variable declared in the block
    begin
      pkt p, q;
      p = new;  q = p;
      p.sc <= 8'h66;
      p = new;
      #1;
      chk("block variable: old object",  q.sc, 8'h66);
      chk("block variable: new object",  p.sc, 8'h00);
    end

    // 7: a formal reassigned after the NBA, in a task that has not waited
    a = new;  b = new;
    h.by_formal(a, b);
    #1;
    chk("formal: old object updated",         a.sc, 8'h93);
    chk("formal: new object untouched",       b.sc, 8'h00);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        1000,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.contains("PASS") || m.contains("FAIL"))
    .collect();
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}

/// The root's declared class picks the property (§8.14): `hb.f <= 7` with
/// `hb` a base handle to a derived object that redeclares `f` writes the
/// base's `f`; array and queue element roots are untouched.
#[test]
fn nba_root_uses_the_declared_class() {
    let out: Vec<String> = simulate(
        r#"class base; int f; endclass
class derived extends base; int f; endclass
class box; int f; endclass
module t;
  box objs[3];
  box q[$];
  base hb;
  derived d;
  initial begin
    foreach (objs[i]) objs[i] = new;
    begin box b0, b1; b0 = new; b1 = new; q.push_back(b0); q.push_back(b1); end
    objs[1].f <= 5;
    q[1].f = 6;
    d = new; hb = d;
    hb.f <= 7;
    #1;
    $display("R %0d %0d %0d %0d %0d %0d", objs[0].f, objs[1].f, q[0].f, q[1].f, hb.f, d.f);
  end
endmodule
"#,
        100,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.starts_with("R "))
    .collect();
    assert_eq!(out, ["R 0 5 0 6 7 0"]);
}
