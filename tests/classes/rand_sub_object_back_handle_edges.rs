//! Regression tests for edge cases in constraints acting through a non-rand
//! back-reference handle (`item.*`): arrays, foreach/unique, sibling sub-objects,
//! nested container name collisions, >64-bit bit-selects, bottom-up
//! `post_randomize` ordering across nested containers, and non-CSP (`!csp_ok`)
//! fallback (`randc`, associative policy containers).

fn check(src: &str) {
    let sim = xezim::simulate(src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}

#[test]
fn back_handle_constrains_item_array_elements_and_struct_slice() {
    check(
        r#"
typedef struct packed {
  bit [15:0] hi;
  bit [15:0] lo;
} hdr_t;

typedef class txn;
class policy_c;
  txn item;
  constraint c {
    item.arr[0] == 32'h1234;
    item.arr[1] inside {[32'h5670 : 32'h567F]};
    item.hdr.lo == 16'hABCD;
    item.hdr.hi[7:0] == 8'h5A;
  }
endclass

class txn;
  rand hdr_t hdr;
  rand bit [31:0] arr[4];
  rand policy_c p;
  function new(); p = new; endfunction
  function void pre_randomize(); p.item = this; endfunction
endclass

module top;
  initial begin
    txn t = new;
    for (int s = 1; s <= 10; s++) begin
      t.srandom(s);
      if (!t.randomize()) $fatal(1, "randomize failed seed=%0d", s);
      if (t.arr[0] !== 32'h1234) $fatal(1, "arr[0]=%0h", t.arr[0]);
      if (!(t.arr[1] >= 32'h5670 && t.arr[1] <= 32'h567F)) $fatal(1, "arr[1]=%0h", t.arr[1]);
      if (t.hdr.lo !== 16'hABCD) $fatal(1, "hdr.lo=%0h", t.hdr.lo);
      if (t.hdr.hi[7:0] !== 8'h5A) $fatal(1, "hdr.hi=%0h", t.hdr.hi);
    end
    $display("T|PASS");
  end
endmodule
"#,
    );
}

#[test]
fn back_handle_foreach_and_unique_over_item_array() {
    check(
        r#"
typedef class txn;
class policy_c;
  txn item;
  constraint c {
    foreach (item.arr[i]) {
      item.arr[i] inside {[1 : 5]};
    }
    unique { item.arr };
  }
endclass

class txn;
  rand bit [31:0] arr[3];
  rand policy_c p;
  function new(); p = new; endfunction
  function void pre_randomize(); p.item = this; endfunction
endclass

module top;
  initial begin
    txn t = new;
    for (int s = 1; s <= 10; s++) begin
      t.srandom(s);
      if (!t.randomize()) $fatal(1, "randomize failed seed=%0d", s);
      foreach (t.arr[i]) begin
        if (!(t.arr[i] >= 1 && t.arr[i] <= 5)) $fatal(1, "out of range arr[%0d]=%0d", i, t.arr[i]);
      end
      if (t.arr[0] == t.arr[1] || t.arr[0] == t.arr[2] || t.arr[1] == t.arr[2])
        $fatal(1, "not unique: %0d %0d %0d", t.arr[0], t.arr[1], t.arr[2]);
    end
    $display("T|PASS");
  end
endmodule
"#,
    );
}

#[test]
fn back_handle_reaches_sibling_rand_sub_object() {
    check(
        r#"
typedef class txn;
class sibling_c;
  rand bit [31:0] x;
endclass

class policy_c;
  txn item;
  constraint c {
    item.other.x inside {[32'h1000 : 32'h100F]};
    item.base == item.other.x + 32'd1;
  }
endclass

class txn;
  rand bit [31:0] base;
  rand sibling_c other;
  rand policy_c p;
  function new();
    other = new;
    p = new;
  endfunction
  function void pre_randomize(); p.item = this; endfunction
endclass

module top;
  initial begin
    txn t = new;
    for (int s = 1; s <= 10; s++) begin
      t.srandom(s);
      if (!t.randomize()) $fatal(1, "randomize failed seed=%0d", s);
      if (!(t.other.x >= 32'h1000 && t.other.x <= 32'h100F)) $fatal(1, "other.x=%0h", t.other.x);
      if (t.base !== t.other.x + 32'd1) $fatal(1, "base=%0h other.x=%0h", t.base, t.other.x);
    end
    $display("T|PASS");
  end
endmodule
"#,
    );
}

#[test]
fn back_handle_nested_member_name_clash_with_owner_scalar() {
    check(
        r#"
typedef class txn;
class policy_base;
  txn item;
  virtual function void set_item(txn it); item = it; endfunction
endclass

class policy_list extends policy_base;
  rand policy_base q[$];
  function void set_item(txn it);
    foreach (q[i]) q[i].set_item(it);
  endfunction
endclass

class policy_a extends policy_base;
  constraint c {
    item.addr inside {[32'h1000 : 32'h100F]};
    item.q == 2;
  }
endclass

class txn;
  rand bit [31:0] addr;
  rand int q;
  rand policy_list pl;
  function new();
    policy_a pa = new;
    pl = new;
    pl.q = {pa};
  endfunction
  function void pre_randomize(); pl.set_item(this); endfunction
endclass

module top;
  initial begin
    txn t = new;
    for (int s = 1; s <= 10; s++) begin
      t.srandom(s);
      if (!t.randomize()) $fatal(1, "randomize failed seed=%0d", s);
      if (!(t.addr >= 32'h1000 && t.addr <= 32'h100F)) $fatal(1, "addr=%0h", t.addr);
      if (t.q !== 2) $fatal(1, "q=%0d", t.q);
    end
    $display("T|PASS");
  end
endmodule
"#,
    );
}

#[test]
fn back_handle_wide_bit_select() {
    check(
        r#"
typedef class txn;
class policy_c;
  txn item;
  constraint c {
    item.wide[95:64] == 32'hCAFE_BABE;
    item.wide[31:0] inside {[32'd1 : 32'd15]};
  }
endclass

class txn;
  rand bit [127:0] wide;
  rand policy_c p;
  function new(); p = new; endfunction
  function void pre_randomize(); p.item = this; endfunction
endclass

module top;
  initial begin
    txn t = new;
    for (int s = 1; s <= 10; s++) begin
      t.srandom(s);
      if (!t.randomize()) $fatal(1, "randomize failed seed=%0d", s);
      if (t.wide[95:64] !== 32'hCAFE_BABE) $fatal(1, "wide[95:64]=%0h", t.wide[95:64]);
      if (!(t.wide[31:0] >= 1 && t.wide[31:0] <= 15)) $fatal(1, "wide[31:0]=%0h", t.wide[31:0]);
    end
    $display("T|PASS");
  end
endmodule
"#,
    );
}

#[test]
fn back_handle_post_randomize_bottom_up_three_levels() {
    check(
        r#"
typedef class txn;
class policy_leaf;
  txn item;
  bit [31:0] seen;
  bit [31:0] hist[$];
  constraint c { item.addr inside {[32'h1000 : 32'h10FF]}; }
  function void post_randomize();
    seen = item.addr;
    hist.push_back(item.addr);
  endfunction
endclass

class policy_list;
  txn item;
  rand policy_leaf leaf;
  bit [31:0] mid_seen;
  function new(); leaf = new; endfunction
  function void set_item(txn it);
    item = it;
    leaf.item = it;
  endfunction
  function void post_randomize();
    if (leaf.seen !== item.addr) $fatal(1, "mid saw leaf.seen=%0h expected %0h", leaf.seen, item.addr);
    mid_seen = leaf.seen + 32'd1;
  endfunction
endclass

class txn;
  rand bit [31:0] addr;
  rand policy_list pl;
  bit [31:0] top_seen;
  function new(); pl = new; endfunction
  function void pre_randomize(); pl.set_item(this); endfunction
  function void post_randomize();
    if (pl.leaf.seen !== addr) $fatal(1, "top saw leaf.seen=%0h expected %0h", pl.leaf.seen, addr);
    if (pl.mid_seen !== addr + 32'd1) $fatal(1, "top saw mid_seen=%0h expected %0h", pl.mid_seen, addr + 1);
    top_seen = pl.mid_seen;
  endfunction
endclass

module top;
  initial begin
    txn t = new;
    t.srandom(7);
    for (int i = 0; i < 5; i++) begin
      if (!t.randomize()) $fatal(1, "randomize failed iter=%0d", i);
      if (t.pl.leaf.hist.size() != i + 1) $fatal(1, "hist size=%0d", t.pl.leaf.hist.size());
      if (t.pl.leaf.hist[i] !== t.addr) $fatal(1, "hist[%0d]=%0h addr=%0h", i, t.pl.leaf.hist[i], t.addr);
      if (t.top_seen !== t.addr + 32'd1) $fatal(1, "top_seen=%0h", t.top_seen);
    end
    $display("T|PASS");
  end
endmodule
"#,
    );
}

#[test]
fn back_handle_non_csp_randc_and_assoc_map_fallback() {
    check(
        r#"
typedef class txn_randc;
class sibling_r;
  rand bit [31:0] x;
endclass

class policy_randc;
  txn_randc item;
  randc bit [3:0] k;
  constraint c {
    item.addr inside {[32'h1000_0000 : 32'h1000_00FF]};
    item.addr[3:0] == k;
    item.other.x inside {[32'd10 : 32'd20]};
  }
endclass

class txn_randc;
  randc bit [3:0] kind;
  rand bit [31:0] addr;
  rand sibling_r other;
  rand policy_randc p;
  function new();
    other = new;
    p = new;
  endfunction
  function void pre_randomize(); p.item = this; endfunction
endclass

typedef class txn_assoc;
class policy_assoc;
  txn_assoc item;
  rand bit [7:0] k;
  constraint c {
    item.addr inside {[32'h2000 : 32'h20FF]};
    item.addr[7:0] == k;
    foreach (item.arr[i]) {
      item.arr[i] inside {[32'd1 : 32'd4]};
    }
    unique { item.arr };
  }
endclass

class txn_assoc;
  rand bit [31:0] addr;
  rand bit [31:0] arr[3];
  rand policy_assoc pm[string];
  function new();
    policy_assoc q = new;
    pm["a"] = q;
  endfunction
  function void pre_randomize();
    foreach (pm[idx]) pm[idx].item = this;
  endfunction
endclass

module top;
  initial begin
    txn_randc tr = new;
    txn_assoc ta = new;
    tr.srandom(11);
    ta.srandom(22);
    for (int i = 0; i < 10; i++) begin
      if (!tr.randomize()) $fatal(1, "tr.randomize failed iter=%0d", i);
      if (!(tr.addr >= 32'h1000_0000 && tr.addr <= 32'h1000_00FF)) $fatal(1, "tr.addr=%0h", tr.addr);
      if (tr.addr[3:0] !== tr.p.k) $fatal(1, "tr.addr[3:0]=%0h k=%0h", tr.addr[3:0], tr.p.k);
      if (!(tr.other.x >= 10 && tr.other.x <= 20)) $fatal(1, "tr.other.x=%0d", tr.other.x);

      if (!ta.randomize()) $fatal(1, "ta.randomize failed iter=%0d", i);
      if (!(ta.addr >= 32'h2000 && ta.addr <= 32'h20FF)) $fatal(1, "ta.addr=%0h", ta.addr);
      if (ta.addr[7:0] !== ta.pm["a"].k) $fatal(1, "ta.addr[7:0]=%0h k=%0h", ta.addr[7:0], ta.pm["a"].k);
      foreach (ta.arr[j]) begin
        if (!(ta.arr[j] >= 1 && ta.arr[j] <= 4)) $fatal(1, "ta.arr[%0d]=%0d", j, ta.arr[j]);
      end
      if (ta.arr[0] == ta.arr[1] || ta.arr[0] == ta.arr[2] || ta.arr[1] == ta.arr[2])
        $fatal(1, "ta.arr not unique");
    end
    $display("T|PASS");
  end
endmodule
"#,
    );
}
