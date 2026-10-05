//! §18.6.2 / §18.8 / §18.9: `randomize()` calls `pre_randomize()` first and
//! only then computes the new values, so a `rand_mode` / `constraint_mode`
//! change made in `pre_randomize()` applies to that same call (#244). The
//! active variables and constraints were taken before `pre_randomize()` ran,
//! so the change took effect only from the next call: a variable disabled
//! there was still randomized, and a constraint disabled there still applied
//! (and could fail the call). Every check below is on the FIRST call. The
//! preset values are ones no object handle takes.

use xezim::simulate;

const SRC: &str = r#"
class rm_item;
  rand bit [31:0] id;
  function void pre_randomize();
    id.rand_mode(0);
  endfunction
endclass

class cm_item;
  rand int x;
  constraint c_one { x == 1; }
  constraint c_two { x == 2; }
  function void pre_randomize();
    c_one.constraint_mode(0);
  endfunction
endclass

// An inherited variable and constraint, disabled by the derived class after
// `super.pre_randomize()`.
class base_item;
  rand bit [31:0] id;
  rand int        len;
  constraint c_len { len == 1; }
  function void pre_randomize();
  endfunction
endclass
class derived_item extends base_item;
  constraint c_len2 { len == 2; }
  function void pre_randomize();
    super.pre_randomize();
    id.rand_mode(0);
    c_len.constraint_mode(0);
  endfunction
endclass

// No pre_randomize() of its own: the one inherited from `cm_base`, one and
// two levels up.
class cm_base;
  rand bit [31:0] id;
  rand int x;
  constraint c_one { x == 1; }
  constraint c_two { x == 2; }
  function void pre_randomize();
    id.rand_mode(0);
    c_one.constraint_mode(0);
  endfunction
endclass
class cm_inherits extends cm_base;
endclass
class cm_inherits2 extends cm_inherits;
endclass

// Re-enabling a variable that was disabled before the call.
class on_item;
  rand int v;
  constraint c_v { v inside {[100:200]}; }
  function void pre_randomize();
    v.rand_mode(1);
  endfunction
endclass

// A rand member object's own `pre_randomize()`.
class child;
  rand bit [31:0] id;
  function void pre_randomize();
    id.rand_mode(0);
  endfunction
endclass
class parent;
  rand child c;
  function new(); c = new(); endfunction
endclass

module top;
  initial begin
    rm_item r = new();
    cm_item m = new();
    cm_item mi = new();
    derived_item d = new();
    cm_inherits ci = new();
    cm_inherits2 ci2 = new();
    on_item o = new();
    parent p = new();
    int ok;

    r.id = 'hcafe0001;
    ok = r.randomize();
    $display("rand_mode ok=%0d id=%h", ok, r.id);

    ok = m.randomize();
    $display("constraint_mode ok=%0d x=%0d", ok, m.x);

    ok = mi.randomize() with { x > 0; };
    $display("inline ok=%0d x=%0d", ok, mi.x);

    d.id = 'hcafe0002;
    ok = d.randomize();
    $display("derived ok=%0d id=%h len=%0d", ok, d.id, d.len);

    ci.id = 'hcafe0004;
    ok = ci.randomize();
    $display("inherited ok=%0d id=%h x=%0d", ok, ci.id, ci.x);

    ci2.id = 'hcafe0005;
    ok = ci2.randomize();
    $display("inherited2 ok=%0d id=%h x=%0d", ok, ci2.id, ci2.x);

    o.v = -1;
    o.v.rand_mode(0);
    ok = o.randomize();
    $display("re-enabled ok=%0d in_range=%0d", ok, o.v inside {[100:200]});

    p.c.id = 'hcafe0003;
    ok = p.randomize();
    $display("member ok=%0d id=%h", ok, p.c.id);
  end
endmodule
"#;

#[test]
fn pre_randomize_mode_changes_apply_to_the_same_call() {
    let out: Vec<String> = simulate(SRC, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(
        out,
        [
            "rand_mode ok=1 id=cafe0001",
            "constraint_mode ok=1 x=2",
            "inline ok=1 x=2",
            "derived ok=1 id=cafe0002 len=2",
            "inherited ok=1 id=cafe0004 x=2",
            "inherited2 ok=1 id=cafe0005 x=2",
            "re-enabled ok=1 in_range=1",
            "member ok=1 id=cafe0003",
        ],
        "{out:?}"
    );
}
