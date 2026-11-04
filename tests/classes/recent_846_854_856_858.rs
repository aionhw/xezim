//! IEEE 1800 §§6.16, 8.27, 13.5.2, 18.8, 23.9: forward classes,
//! selected ref rows, rand-handle modes and scalar/collection shadowing.

use xezim::simulate;

fn policies() -> bool {
    super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0"), ("1", "1000")])
}

fn check(source: &str, expected: &[&str]) {
    let sim = simulate(source, 100).expect("simulation failed");
    let lines = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .collect::<Vec<_>>();
    assert!(!lines.iter().any(|s| s.contains("FAIL")), "{lines:#?}");
    for expected in expected {
        assert!(lines.contains(expected), "missing {expected:?}: {lines:#?}");
    }
    assert_eq!(lines.last().copied(), Some("ALL PASS"), "{lines:#?}");
}

/// §6.16, §8.27 (#858): the completed class supplies string property types,
/// including through aliases and queue indices, in every execution tier.
#[test]
fn forward_class_string_properties_compare_as_strings() {
    if !policies() {
        return;
    }
    check(
        r#"
typedef class item_c;
class list_c;
  item_c items[$];
  function logic empty_at(int i); return items[i].label == ""; endfunction
endclass
class item_c; string label; endclass
typedef item_c item_alias_t;
class alias_list_c; item_alias_t items[$]; endclass
module top;
  initial begin
    list_c l = new();
    alias_list_c aliases = new();
    item_c a = new();
    item_c b = new();
    logic [3:0] unknown = 4'bxxxx;
    b.label = "x";
    l.items.push_back(a); l.items.push_back(b); aliases.items.push_back(a);
    $display("empty=%b named=%b", l.items[0].label == "", l.items[1].label == "");
    if ((l.items[0].label == "") !== 1'b1) $display("FAIL empty equality");
    if ((l.items[0].label != "") !== 1'b0) $display("FAIL empty inequality");
    if ((l.items[1].label != "") !== 1'b1) $display("FAIL named inequality");
    if ((aliases.items[0].label == "") !== 1'b1) $display("FAIL alias property");
    if ((unknown == 4'b0000) !== 1'bx) $display("FAIL integral X comparison");
    repeat (2005) begin
      if (l.empty_at(0) !== 1'b1 || l.empty_at(1) !== 1'b0)
        $display("FAIL method property comparison");
    end
    $display("ALL PASS");
    $finish;
  end
endmodule
"#,
        &["empty=1 named=0"],
    );
}

/// §13.5.2 (#854): selected rows share ref storage; input arrays are copies.
#[test]
fn ref_dynamic_array_formal_aliases_fixed_array_row() {
    if !policies() {
        return;
    }
    check(
        r#"
class item_c;
  string nm;
  function new(string n = ""); nm = n; endfunction
endclass
class gen_c;
  item_c rows[2][];
  int index_calls;
  function int selected_row(); index_calls++; return 1; endfunction
  function void fill(ref item_c sp[], input int n);
    sp = new[n];
    foreach (sp[i]) sp[i] = new($sformatf("e%0d", i));
  endfunction
  function void input_copy(input item_c sp[]);
    sp = new[1]; sp[0] = new("copy");
  endfunction
  function void run();
    gen_c other = new();
    fill(rows[selected_row()], 3);
    if (index_calls != 1) $display("FAIL repeated index evaluation");
    if (rows[0].size() != 0 || rows[1].size() != 3) $display("FAIL bare row size");
    else if (rows[1][2].nm != "e2") $display("FAIL bare row contents");
    input_copy(rows[1]);
    if (rows[1].size() != 3) $display("FAIL input formal changed caller size");
    else if (rows[1][2].nm != "e2") $display("FAIL input formal changed caller contents");
    fill(this.rows[1], 2);
    if (rows[0].size() != 0 || rows[1].size() != 2) $display("FAIL explicit this row");
    fill(other.rows[1], 4);
    if (other.rows[0].size() != 0 || other.rows[1].size() != 4) $display("FAIL other row size");
    else if (other.rows[1][3].nm != "e3") $display("FAIL other row contents");
    if (rows[0].size() != 0 || rows[1].size() != 2) $display("FAIL receiver rows changed");
  endfunction
endclass
module top;
  initial begin
    gen_c g = new(); g.run();
    $display("ALL PASS"); $finish;
  end
endmodule
"#,
        &[],
    );
}

/// §18.8 (#846): only a rand-handle variable receives its own mode; non-rand
/// handles, local receivers and constraint_mode retain object semantics.
#[test]
fn bare_rand_handle_mode_matches_explicit_this() {
    if !policies() {
        return;
    }
    check(
        r#"
class sub_c;
  rand bit [7:0] a, b;
  constraint gt_c { a > b; }
  function int enabled(); return a.rand_mode() && b.rand_mode(); endfunction
  function int constraints_enabled(); return gt_c.constraint_mode(); endfunction
endclass
class outer_c;
  rand sub_c sub;
  sub_c plain;
  function new(); sub = new(); plain = new(); sub.a = 3; sub.b = 200; endfunction
  function void mode_bare(int on); sub.rand_mode(on); endfunction
  function void mode_this(int on); this.sub.rand_mode(on); endfunction
  function void mode_local(sub_c sub, int on); sub.rand_mode(on); endfunction
  function void mode_plain(int on); plain.rand_mode(on); endfunction
  function void mode_constraints(int on); sub.constraint_mode(on); endfunction
endclass
class inherited_c extends outer_c; endclass
module top;
  initial begin
    outer_c o1 = new(); outer_c o2 = new(); outer_c o3 = new();
    inherited_c inherited = new();
    sub_c shadow = new();
    int ok;
    o1.mode_bare(0); o2.mode_this(0); o3.sub.rand_mode(0); inherited.mode_bare(0);
    ok = o1.randomize();
    $display("bare ok=%0d handle=%0d", ok, o1.sub.rand_mode());
    if (ok != 1 || o1.sub.enabled() != 1 || o1.sub.a != 3 || o1.sub.b != 200)
      $display("FAIL bare rand handle");
    ok = o2.randomize();
    $display("this ok=%0d handle=%0d", ok, o2.sub.rand_mode());
    if (ok != 1 || o2.sub.enabled() != 1 || o2.sub.a != 3 || o2.sub.b != 200)
      $display("FAIL explicit this rand handle");
    ok = o3.randomize();
    $display("external ok=%0d handle=%0d", ok, o3.sub.rand_mode());
    if (ok != 1 || o3.sub.enabled() != 1 || o3.sub.a != 3 || o3.sub.b != 200)
      $display("FAIL external rand handle");
    if (inherited.sub.rand_mode() != 0 || inherited.sub.enabled() != 1)
      $display("FAIL inherited rand handle");
    o1.mode_bare(1);
    if (o1.sub.rand_mode() != 1 || o1.sub.enabled() != 1) $display("FAIL handle reenable");
    o1.mode_local(shadow, 0);
    if (shadow.enabled() != 0 || o1.sub.rand_mode() != 1 || o1.sub.enabled() != 1)
      $display("FAIL local receiver shadow");
    o1.mode_plain(0);
    if (o1.plain.enabled() != 0 || o1.sub.enabled() != 1) $display("FAIL non-rand receiver");
    o1.mode_constraints(0);
    if (o1.sub.constraints_enabled() != 0 || o1.sub.enabled() != 1 || o1.sub.rand_mode() != 1)
      $display("FAIL constraint mode receiver");
    $display("ALL PASS"); $finish;
  end
endmodule
"#,
        &[
            "bare ok=1 handle=0",
            "this ok=1 handle=0",
            "external ok=1 handle=0",
        ],
    );
}

/// §23.9 (#856): subroutine scalars hide caller collections and runtime
/// subclass collection properties. Crossing the tier boundary must agree.
#[test]
fn scalar_locals_and_formals_hide_unrelated_collections() {
    if !policies() {
        return;
    }
    check(
        r#"
function automatic int free_len(string str); return str.len(); endfunction
class util_c;
  function int local_len(); string str = "abcde"; return str.len(); endfunction
  function int formal_len(string str); return str.len(); endfunction
  function int recurse(int n, string str);
    if (n == 0) return str.len();
    return recurse(n - 1, str);
  endfunction
endclass
class caller_c;
  function void run();
    string str[$] = '{"x", "y"};
    util_c u = new();
    $display("local=%0d formal=%0d", u.local_len(), u.formal_len("abc"));
    repeat (2005) begin
      if (u.local_len() != 5 || u.formal_len("abc") != 3) $display("FAIL method scalar");
      if (free_len("abcd") != 4 || u.recurse(2, "abcdef") != 6) $display("FAIL nested scalar");
    end
    if (str.size() != 2 || str[0] != "x" || str[1] != "y") $display("FAIL caller queue changed");
  endfunction
endclass
class base_c;
  function int fn(); int v = 5; v = v + 1; return v; endfunction
  function int formal(int v); return v + 2; endfunction
endclass
class sub_c extends base_c; int v[$]; endclass
module top;
  initial begin
    caller_c c = new(); sub_c s = new(); base_c b = new();
    s.v.push_back(41); c.run();
    $display("sub=%0d base=%0d", s.fn(), b.fn());
    repeat (2005) begin
      if (s.fn() != 6 || b.fn() != 6 || s.formal(5) != 7) $display("FAIL subclass collection");
    end
    if (s.v.size() != 1 || s.v[0] != 41) $display("FAIL subclass queue changed");
    $display("ALL PASS"); $finish;
  end
endmodule
"#,
        &["local=5 formal=3", "sub=6 base=6"],
    );
}

/// §6.21/§23.9 (#856): a nested scalar declaration temporarily hides a
/// same-frame collection and restores its storage on leaving the block.
#[test]
fn scalar_block_scope_hides_and_restores_collection() {
    if !policies() {
        return;
    }
    check(
        r#"
class c;
  function void run();
    string str[$] = '{"first", "second"};
    int v[$] = '{11, 22};
    begin
      string str = "abcde";
      int v = 5;
      if (str.len() != 5) $display("FAIL inner string");
      v++;
      if (v != 6) $display("FAIL inner integer");
    end
    if (str.size() != 2 || str[0] != "first" || str[1] != "second")
      $display("FAIL outer string queue");
    if (v.size() != 2 || v[0] != 11 || v[1] != 22) $display("FAIL outer integer queue");
  endfunction
endclass
module top;
 initial begin c o = new(); o.run(); $display("ALL PASS"); $finish; end
endmodule
"#,
        &[],
    );
}

/// §11.4.1/§13.5.2 (#854): binding a selected row evaluates both the
/// object receiver and row selector once, for fixed and dynamic outers.
#[test]
fn ref_row_receiver_and_selector_are_evaluated_once() {
    if !policies() {
        return;
    }
    for (shape, constructor) in [
        ("[2][]", ""),
        ("[][]", "function new(); rows = new[2]; endfunction"),
    ] {
        let source = format!(
            r#"
class c;
  int rows{shape};
  int receiver_calls;
  int selector_calls;
  {constructor}
  function c get_self(); receiver_calls++; return this; endfunction
  function int row_index(); selector_calls++; return 1; endfunction
  function void fill(ref int sp[]);
    sp = new[3]; sp[0] = 11; sp[2] = 33;
  endfunction
  function void run(); fill(get_self().rows[row_index()]); endfunction
endclass
module top;
  initial begin
    c o = new();
    o.run();
    $display("RESULT receiver=%0d selector=%0d row0=%0d row1=%0d first=%0d",
             o.receiver_calls, o.selector_calls, o.rows[0].size(),
             o.rows[1].size(), o.rows[1].size() ? o.rows[1][0] : -1);
    if (o.receiver_calls != 1 || o.selector_calls != 1
        || o.rows[0].size() != 0 || o.rows[1].size() != 3)
      $display("FAIL selected ref row");
    else if (o.rows[1][0] != 11 || o.rows[1][2] != 33)
      $display("FAIL selected row contents");
    $display("ALL PASS"); $finish;
  end
endmodule
"#
        );
        check(
            &source,
            &["RESULT receiver=1 selector=1 row0=0 row1=3 first=11"],
        );
    }
}
