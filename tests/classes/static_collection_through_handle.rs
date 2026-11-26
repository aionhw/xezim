//! §8.9: a static property is one variable shared by every object of the
//! class, and an object reaches it "in the usual way": `h.m`, `this.m`,
//! `super.m`, an inherited static handle `sh.m` and `Cls::m` name the same
//! associative array, queue or dynamic array, with its elements and methods
//! (§7.5, §7.8, §7.9, §7.10), and a by-value actual copies it (§13.5.1).
//! Through a handle such a collection named no store: writes were lost,
//! `first`/`last`/`next`/`prev` found nothing and a by-value actual was
//! empty (#1253). Expected values are the reference simulator's.

use xezim::simulate;

const SHARED: &str = r#"
class store_t;
  static bit  seen[string];     // static associative array
  static int  log[$];           // static queue
  static int  dyn[];            // static dynamic array
  static int  ia[int];          // static associative array of integers
  function void add(string k);  // bare name in a method
    seen[k] = 1;
    log.push_back(k.len());
  endfunction
endclass

class base_t;
  static store_t shared = new;  // an inherited static handle
endclass

class user_t extends base_t;
  function int walk(bit m[string]);          // by-value actual: walk the copy
    string k;
    int n = 0;
    if (m.last(k)) begin
      do n++; while (m.prev(k));
    end
    return n;
  endfunction
  function void touch();                     // through the inherited handle
    shared.seen["c"] = 1;
  endfunction
  function void fill();                      // element writes through the inherited handle
    shared.log.push_back(5);
    shared.dyn = new[2];
    shared.dyn[1] = 4;
    shared.ia[3] = 11;
  endfunction
endclass

function automatic int sum_q(int a[$]);
  int t = 0;
  foreach (a[i]) t += a[i];
  return t;
endfunction

function automatic int sum_da(int a[]);
  int t = 0;
  foreach (a[i]) t += a[i];
  return t;
endfunction

function automatic int free_walk(bit m[string]);
  string k;
  int n = 0;
  if (m.first(k)) begin
    do n++; while (m.next(k));
  end
  return n;
endfunction

module top;
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask

  initial begin
    user_t  u = new;
    store_t s = new;
    string  k;
    s.add("a");                          // writes the shared store
    u.shared.seen["b"] = 1;              // through the inherited static handle
    u.touch();
    check("size through a handle",           u.shared.seen.size(), 3);
    check("size through the class scope",    store_t::seen.size(), 3);
    check("size through another handle",     s.seen.size(), 3);
    check("exists through a handle",         s.seen.exists("b"), 1);
    check("last() through a handle",         u.shared.seen.last(k), 1);
    check("last() key",                      (k == "c"), 1);
    check("by-value actual, handle",         u.walk(u.shared.seen), 3);
    check("by-value actual, class scope",    u.walk(store_t::seen), 3);
    check("by-value actual, free function",  free_walk(u.shared.seen), 3);
    u.shared.seen.delete("a");
    check("delete through a handle",         store_t::seen.size(), 2);
    // a static queue
    s.log.push_back(7);
    check("queue size through a handle",     u.shared.log.size(), 2);
    check("queue size through the class",    store_t::log.size(), 2);
    check("queue element through a handle",  u.shared.log[1], 7);
    // a static dynamic array
    s.dyn = new[3];
    s.dyn[2] = 9;
    check("dynamic array size",              u.shared.dyn.size(), 3);
    check("dynamic array element",           store_t::dyn[2], 9);
    check("queue by value through a handle", sum_q(u.shared.log), 8);
    check("dynamic array by value through a handle", sum_da(u.shared.dyn), 9);
    // element writes through the inherited handle, in a method
    u.fill();
    check("queue element written in a method",   store_t::log[2], 5);
    check("dynamic array element written in a method", store_t::dyn[1], 4);
    check("integer-keyed element written in a method", store_t::ia[3], 11);
    check("integer-keyed element read through a handle", s.ia[3], 11);
    $finish;
  end
endmodule
"#;

const SHAPES: &str = r#"
class pc #(type T = int);
  static T  vals[$];
  static int cnt[string];
  function void add(T v); vals.push_back(v); cnt["n"]++; endfunction
endclass

class sc;
  static int    aa[int];
  static string sq[$];
  static real   rq[$];
  static int    fa[3];
  static int    dq[];
  int           inst_aa[int];
  function int this_read(int k);        // `this.aa[k]` in a method
    return this.aa[k];
  endfunction
  function int this_fa(int i);
    return this.fa[i];
  endfunction
  function int whole_fa();
    return sum3(fa) + sum3(this.fa);
  endfunction
  static function int sum3(int a[3]);
    return a[0] + a[1] + a[2];
  endfunction
  function void pat();
    this.aa = '{1: 10, 2: 20};
  endfunction
  function int this_size();
    return this.aa.size() + this.sq.size();
  endfunction
endclass

class dsc extends sc;
  function int super_size();
    return super.aa.size();
  endfunction
  function int super_actual();
    return count(super.aa);
  endfunction
  function int count(int m[int]);
    return m.size();
  endfunction
endclass

function automatic void fill_ref(ref int m[int]);
  m[100] = 1;
endfunction
function automatic void fill_out(output int m[int]);
  m[200] = 2;
endfunction
function automatic void fill_inout(inout int m[int]);
  m[300] = 3;
endfunction
function automatic int qsum(string q[$]);
  return q.size();
endfunction

module top;
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask
  initial begin
    sc  h = new;
    dsc d = new;
    pc #(int)    pi = new;
    pc #(string) ps = new;
    int k, n;
    h.aa[5] = 50;
    check("element write through handle", sc::aa[5], 50);
    check("this.aa[k] in method", h.this_read(5), 50);
    void'(h.aa.first(k));
    check("first() from module code", k, 5);
    h.aa[7] = 70;
    void'(h.aa.last(k));
    check("last() from module code", k, 7);
    n = 0;
    foreach (h.aa[i]) n += i;
    check("foreach through handle", n, 12);
    h.pat();
    check("pattern through this", sc::aa.size(), 2);
    h.aa = '{3: 30};
    check("pattern through handle", sc::aa[3], 30);
    sc::aa = '{4: 40, 6: 60};
    check("pattern through class scope", h.aa.size(), 2);
    fill_ref(h.aa);
    check("ref actual through handle", sc::aa[100], 1);
    fill_inout(h.aa);
    check("inout actual through handle", sc::aa[300], 3);
    fill_out(sc::aa);
    check("output actual class scope", h.aa.size(), 1);
    fill_out(h.aa);
    check("output actual through handle", sc::aa[200], 2);
    check("super.aa.size()", d.super_size(), 1);
    check("super.aa actual", d.super_actual(), 1);
    check("derived handle", d.aa[200], 2);
    h.sq.push_back("a");
    d.sq.push_back("b");
    check("string queue shared", sc::sq.size(), 2);
    check("string queue by value", qsum(h.sq), 2);
    h.sq = {h.sq, "c"};
    check("string queue concat through handle", sc::sq.size(), 3);
    check("this.size()", h.this_size(), 4);
    h.rq.push_back(1.5);
    check("real queue", $rtoi(sc::rq[0] * 2), 3);
    h.fa[1] = 4;
    check("fixed through handle", sc::fa[1], 4);
    check("this.fa[i]", h.this_fa(1), 4);
    check("whole fa", h.whole_fa(), 8);
    h.dq = new[2];
    h.dq[1] = 9;
    check("dynamic through handle", sc::dq[1], 9);
    h.inst_aa[1] = 1;
    check("instance first()", h.inst_aa.first(k), 1);
    pi.add(3);
    ps.add("x");
    ps.add("y");
    check("spec int vals", pi.vals.size(), 1);
    check("spec string vals", ps.vals.size(), 2);
    check("spec int scope", pc#(int)::vals.size(), 1);
    check("spec string cnt", ps.cnt["n"], 2);
    $finish;
  end
endmodule
"#;

const NAMES: &str = r#"
typedef enum {LO, MID, HI} lvl_e;
class item_t;
  int id;
  function new(int i); id = i; endfunction
endclass
class a_t;
  static int aa[int];
  static item_t hq[$];
  static int ek[lvl_e];
  static byte bq[$];
endclass
class b_t;
  static int aa[int];
endclass
class base_t;
  static int rq[$];
endclass
class der_t extends base_t;
  static int rq[$];             // redeclared: its own store
endclass
class inh_t extends base_t;
endclass

module top;
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask
  initial begin
    a_t a = new;
    b_t b = new;
    der_t d = new;
    inh_t i = new;
    item_t it;
    a.aa[1] = 10;
    b.aa[1] = 20;
    b.aa[2] = 30;
    check("collision a", a_t::aa.size(), 1);
    check("collision b", b_t::aa.size(), 2);
    check("collision a value", a.aa[1], 10);
    it = new(7);
    a.hq.push_back(it);
    check("handle queue", a_t::hq[0].id, 7);
    a.ek[HI] = 3;
    check("enum key", a_t::ek[HI], 3);
    a.bq.push_back(8'h41);
    check("byte queue", a_t::bq[0], 65);
    d.rq.push_back(1);
    i.rq.push_back(2);
    i.rq.push_back(3);
    check("redeclared", der_t::rq.size(), 1);
    check("inherited", base_t::rq.size(), 2);
    $finish;
  end
endmodule
"#;

fn checks(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect()
}

#[test]
fn static_collection_through_handle_is_shared() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    assert_eq!(
        checks(SHARED),
        [
            "PASS size through a handle = 3",
            "PASS size through the class scope = 3",
            "PASS size through another handle = 3",
            "PASS exists through a handle = 1",
            "PASS last() through a handle = 1",
            "PASS last() key = 1",
            "PASS by-value actual, handle = 3",
            "PASS by-value actual, class scope = 3",
            "PASS by-value actual, free function = 3",
            "PASS delete through a handle = 2",
            "PASS queue size through a handle = 2",
            "PASS queue size through the class = 2",
            "PASS queue element through a handle = 7",
            "PASS dynamic array size = 3",
            "PASS dynamic array element = 9",
            "PASS queue by value through a handle = 8",
            "PASS dynamic array by value through a handle = 9",
            "PASS queue element written in a method = 5",
            "PASS dynamic array element written in a method = 4",
            "PASS integer-keyed element written in a method = 11",
            "PASS integer-keyed element read through a handle = 11",
        ]
    );
}

#[test]
fn static_collection_through_handle_shapes() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    assert_eq!(
        checks(SHAPES),
        [
            "PASS element write through handle = 50",
            "PASS this.aa[k] in method = 50",
            "PASS first() from module code = 5",
            "PASS last() from module code = 7",
            "PASS foreach through handle = 12",
            "PASS pattern through this = 2",
            "PASS pattern through handle = 30",
            "PASS pattern through class scope = 2",
            "PASS ref actual through handle = 1",
            "PASS inout actual through handle = 3",
            "PASS output actual class scope = 1",
            "PASS output actual through handle = 2",
            "PASS super.aa.size() = 1",
            "PASS super.aa actual = 1",
            "PASS derived handle = 2",
            "PASS string queue shared = 2",
            "PASS string queue by value = 2",
            "PASS string queue concat through handle = 3",
            "PASS this.size() = 4",
            "PASS real queue = 3",
            "PASS fixed through handle = 4",
            "PASS this.fa[i] = 4",
            "PASS whole fa = 8",
            "PASS dynamic through handle = 9",
            "PASS instance first() = 1",
            "PASS spec int vals = 1",
            "PASS spec string vals = 2",
            "PASS spec int scope = 1",
            "PASS spec string cnt = 2",
        ]
    );
}

#[test]
fn static_collection_names_and_element_types() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    assert_eq!(
        checks(NAMES),
        [
            "PASS collision a = 1",
            "PASS collision b = 2",
            "PASS collision a value = 10",
            "PASS handle queue = 7",
            "PASS enum key = 3",
            "PASS byte queue = 65",
            "PASS redeclared = 1",
            "PASS inherited = 2",
        ]
    );
}
