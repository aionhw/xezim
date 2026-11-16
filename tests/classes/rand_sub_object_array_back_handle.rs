//! §18.5.9: the elements of a rand queue of handles are rand sub-objects of
//! the randomized object, and their constraints are solved together with
//! its own. A constraint of such an element may read the object's rand
//! variables through a handle back to it (`item.addr`, the policy classes
//! of constraint layering). The element's own solve cannot move those
//! variables, so the item is left to the enclosing solve, which solves it
//! jointly and judges it on the final values. Each element used to be
//! solved alone against the value the variable held before the call:
//! randomize() failed, or, with the item under a `foreach`, returned 1 with
//! values that broke the constraint.

fn run(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .unwrap()
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .collect()
}

/// Two policy objects in a rand queue constrain the owner's `addr` and
/// `size` through `item`. The window is 2^-24 of the address space, so only
/// a joint solve finds it.
#[test]
fn queue_elements_constrain_owner_through_back_handle() {
    let lines = run(r#"
typedef class txn;
class policy_base;
  txn item;
endclass
class permit extends policy_base;
  constraint c { item.addr inside {[32'h0000_1000 : 32'h0000_10FF - item.size]}; }
endclass
class prohibit extends policy_base;
  constraint c { !(item.addr inside {[32'h0000_1000 : 32'h0000_107F]}); }
endclass
class txn;
  rand bit [31:0] addr;
  rand int size;
  rand policy_base policy[$];
  constraint c_size { size inside {1, 2, 4}; }
  function new();
    permit p = new;
    prohibit q = new;
    policy = {p, q};
  endfunction
  function void pre_randomize();
    foreach (policy[i]) policy[i].item = this;
  endfunction
endclass
module t;
  initial begin
    txn x = new;
    int ok = 0, bad = 0;
    repeat (20) begin
      x.addr = 0;
      if (x.randomize()) begin
        ok++;
        bad += !(x.size inside {1, 2, 4})
            || !(x.addr inside {[32'h0000_1080 : 32'h0000_10FF - x.size]});
      end
    end
    $display("ok %0d bad %0d", ok, bad);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["ok 20 bad 0"]);
}

/// The policies sit in a policy list that is itself an element of the
/// owner's queue (a queue of handles inside a sub-object), their class is a
/// specialization of a class with a type parameter, and the ranges are
/// state the items loop over with `foreach`. Unsolved, this form returned 1
/// with addresses outside every permitted range.
#[test]
fn nested_policy_list_foreach_over_state_ranges() {
    let lines = run(r#"
class policy_base #(type ITEM = int);
  ITEM item;
  virtual function void set_item(ITEM item);
    this.item = item;
  endfunction
endclass
class policy_list #(type ITEM = int) extends policy_base #(ITEM);
  rand policy_base #(ITEM) policy[$];
  function void add(policy_base #(ITEM) p);
    policy.push_back(p);
  endfunction
  function void set_item(ITEM item);
    foreach (policy[i]) policy[i].set_item(item);
  endfunction
endclass
class addr_range;
  bit [31:0] min, max;
  function new(bit [31:0] min, bit [31:0] max);
    this.min = min;
    this.max = max;
  endfunction
endclass
typedef class txn;
class addr_policy extends policy_base #(txn);
  addr_range ranges[$];
  function void add(bit [31:0] min, bit [31:0] max);
    addr_range r = new(min, max);
    ranges.push_back(r);
  endfunction
endclass
class permit_policy extends addr_policy;
  rand int selection;
  constraint c {
    selection inside {[0 : ranges.size() - 1]};
    foreach (ranges[i]) {
      if (selection == i) {
        item.addr inside {[ranges[i].min : ranges[i].max - item.size]};
      }
    }
  }
endclass
class prohibit_policy extends addr_policy;
  constraint c {
    foreach (ranges[i]) {
      !(item.addr inside {[ranges[i].min : 1 + ranges[i].max - item.size]});
    }
  }
endclass
class txn;
  rand bit [31:0] addr;
  rand int size;
  rand policy_base #(txn) policy[$];
  constraint c_size { size inside {1, 2, 4}; }
  function new();
    permit_policy p = new;
    prohibit_policy q = new;
    policy_list #(txn) l = new;
    p.add(32'h0000_0000, 32'h0000_00FF);
    p.add(32'h1000_0000, 32'h1000_00FF);
    q.add(32'h0000_0000, 32'h0000_007F);
    l.add(p);
    l.add(q);
    policy = {l};
  endfunction
  function void pre_randomize();
    foreach (policy[i]) policy[i].set_item(this);
  endfunction
endclass
module t;
  initial begin
    txn x = new;
    int ok = 0, bad = 0, low = 0, high = 0;
    repeat (40) begin
      x.addr = 32'hdead_beef;
      if (x.randomize()) begin
        ok++;
        if (x.addr inside {[32'h0000_0080 : 32'h0000_00FF - x.size]}) low++;
        else if (x.addr inside {[32'h1000_0000 : 32'h1000_00FF - x.size]}) high++;
        else bad++;
        bad += !(x.size inside {1, 2, 4});
      end
    end
    $display("ok %0d bad %0d both %0d", ok, bad, low > 0 && high > 0);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["ok 40 bad 0 both 1"]);
}

/// §18.6.2: a policy's `post_randomize` reads the owner's variable through
/// the back handle, so it runs once per call, after the call has its final
/// values: the history it keeps holds exactly the addresses returned, and
/// the item that reads the history keeps every call off an earlier one.
#[test]
fn back_handle_post_randomize_sees_final_values() {
    let lines = run(r#"
typedef class txn;
class policy_base;
  txn item;
endclass
class evict_policy extends policy_base;
  bit [31:0] hist[$];
  int index = 5;
  constraint c {
    !((item.addr & 32'hFFFF_F000) inside {this.hist});
    (item.addr & 32'h0000_0FC0) == (this.index << 6);
    item.addr < 32'h0001_0000;
  }
  function void post_randomize();
    hist.push_back(item.addr & 32'hFFFF_F000);
  endfunction
endclass
class txn;
  rand bit [31:0] addr;
  rand policy_base policy[$];
  evict_policy ev;
  function new();
    ev = new;
    ev.item = this;
    policy = {ev};
  endfunction
endclass
module t;
  initial begin
    txn x = new;
    bit [31:0] seen[$];
    int ok = 0, bad = 0;
    repeat (16) begin
      if (x.randomize()) begin
        ok++;
        bad += (x.addr & 32'h0000_0FC0) != (5 << 6) || x.addr >= 32'h0001_0000;
        bad += (x.addr & 32'hFFFF_F000) inside {seen};
        seen.push_back(x.addr & 32'hFFFF_F000);
        bad += x.ev.hist.size() != ok || x.ev.hist[ok - 1] != (x.addr & 32'hFFFF_F000);
      end
    end
    // All 16 tags below 32'h0001_0000 are used up: no solution is left.
    $display("ok %0d bad %0d then %0d", ok, bad, x.randomize());
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["ok 16 bad 0 then 0"]);
}

/// Policies that contradict each other leave no solution: randomize()
/// returns 0 and the owner keeps its values, rather than returning 1 with
/// an item unenforced.
#[test]
fn contradicting_back_handle_policies_fail() {
    let lines = run(r#"
typedef class txn;
class policy_base;
  txn item;
  bit [31:0] lo[$], hi[$];
endclass
class permit extends policy_base;
  constraint c { foreach (lo[i]) item.addr inside {[lo[i] : hi[i]]}; }
endclass
class prohibit extends policy_base;
  constraint c { foreach (lo[i]) !(item.addr inside {[lo[i] : hi[i]]}); }
endclass
class txn;
  rand bit [31:0] addr;
  rand policy_base policy[$];
  function new();
    permit p = new;
    prohibit q = new;
    p.lo = {32'h100}; p.hi = {32'h1FF};
    q.lo = {32'h000}; q.hi = {32'hFFF};
    p.item = this;
    q.item = this;
    policy = {p, q};
  endfunction
endclass
module t;
  initial begin
    txn x = new;
    int ok = 0;
    x.addr = 32'h55;
    repeat (3) ok += x.randomize();
    $display("ok %0d addr %0h", ok, x.addr);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["ok 0 addr 55"]);
}
