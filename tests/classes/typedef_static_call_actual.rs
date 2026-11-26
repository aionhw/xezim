//! §6.18, §8.23, §8.25.1, §13.5.2: a typedef of a class (or of a class
//! specialization) names that class, a static method is called through it,
//! and its `output`, `inout` and `ref` actuals are written back when it
//! returns. A compiled method's call through a typedef, a typedef of a
//! typedef, a class-scoped typedef or a class type parameter ran the callee
//! but bound every actual as a constant, so the caller's variable kept its
//! old value (#1255); a module-variable actual went nowhere. Expected values
//! are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
typedef enum {RED, GREEN, BLUE} color_e;

class item_c;
  int id;
  function new(int i = 0); id = i; endfunction
endclass

class db_c #(type T = int);
  static T stored;
  static function void put(T v); stored = v; endfunction
  static function bit get(output T v); v = stored; return 1; endfunction
  static function bit get4(int a, string b, bit c, output T v); v = stored; return a > 0; endfunction
  static function void get2(output T v, output T w); v = stored; w = stored; endfunction
  static function bit clear(output T v); T t; v = t; return 1; endfunction
  static function bit swap(inout T v); T t = stored; stored = v; v = t; return 1; endfunction
  static function void fetch(ref T v); v = stored; endfunction
  static task load(output T v); v = stored; endtask
endclass

class plain_c;
  static int sv = 41;
  static function void get(output int v); v = sv; endfunction
endclass

typedef db_c #(item_c)        item_db_t;
typedef item_db_t             item_db2_t;
typedef db_c #(int)           int_db_t;
typedef db_c #(string)        str_db_t;
typedef db_c #(logic [11:0])  vec_db_t;
typedef db_c #(color_e)       col_db_t;
typedef plain_c               plain_t;

class holder_c;
  typedef db_c #(int) inner_db_t;
endclass

class user_c #(type DB = int_db_t);
  int sink;
  int mi;
  function int via_param(int v);
    int x;
    DB::put(v);
    void'(DB::get(x));
    for (int i = 0; i < 2; i++) sink += i;
    return x;
  endfunction
endclass

int gmod;

function automatic item_c mk(int i);
  item_c o = new(i);
  return o;
endfunction

class runner_c;
  int mem;
  item_c mh;
  task run();
    item_c o, o2;
    int x, y, z;
    string s;
    logic [11:0] vv;
    color_e c;
    bit ok;
    user_c #(int_db_t) up = new;
    user_c #(db_c #(int)) us = new;
    item_db_t::put(mk(4));
    void'(item_db_t::get(o));
    $display("T|statement %0d", o == null ? -1 : o.id);
    o = mk(99);
    item_db_t::put(mk(6));
    void'(item_db_t::swap(o));
    $display("T|inout %0d", o == null ? -1 : o.id);
    o = null;
    item_db_t::put(mk(7));
    item_db_t::fetch(o);
    $display("T|ref %0d", o == null ? -1 : o.id);
    o = null;
    item_db_t::put(mk(8));
    item_db_t::load(o);
    $display("T|task %0d", o == null ? -1 : o.id);
    o = null;
    item_db_t::put(mk(5));
    if (item_db_t::get(o)) $display("T|if-cond %0d", o.id);
    o = null;
    ok = item_db_t::get(o) && o != null;
    $display("T|operand %0d %0d", ok, o.id);
    o = null;
    void'(item_db_t::get4(1, "x", 1'b1, o));
    $display("T|fourth %0d", o == null ? -1 : o.id);
    o = null;
    item_db_t::get2(o, o2);
    $display("T|two %0d %0d", o.id, o2.id);
    o = new(9);
    void'(item_db_t::clear(o));
    $display("T|null-over %0d", o == null);
    o = null;
    void'(item_db2_t::get(o));
    $display("T|typedef-of-typedef %0d", o == null ? -1 : o.id);
    int_db_t::put(17);
    void'(int_db_t::get(x));
    $display("T|int %0d", x);
    void'(holder_c::inner_db_t::get(y));
    $display("T|class-scoped typedef %0d", y);
    plain_t::get(z);
    $display("T|plain typedef %0d", z);
    str_db_t::put("hey");
    void'(str_db_t::get(s));
    $display("T|string %s", s);
    vec_db_t::put(12'habc);
    void'(vec_db_t::get(vv));
    $display("T|vector %h", vv);
    col_db_t::put(BLUE);
    void'(col_db_t::get(c));
    $display("T|enum %s", c.name());
    $display("T|type param typedef %0d", up.via_param(23));
    $display("T|type param spec %0d", us.via_param(24));
    int_db_t::put(31);
    void'(int_db_t::get(gmod));
    $display("T|module var %0d", gmod);
    int_db_t::put(32);
    void'(int_db_t::get(mem));
    $display("T|member %0d", mem);
    item_db_t::put(mk(33));
    void'(item_db_t::get(mh));
    $display("T|member handle %0d", mh == null ? -1 : mh.id);
  endtask
endclass

module top;
  initial begin
    runner_c r = new;
    int x;
    item_c o;
    r.run();
    int_db_t::put(44);
    void'(int_db_t::get(x));
    $display("T|module code %0d", x);
    item_db_t::put(mk(45));
    void'(item_db_t::get(o));
    $display("T|module code handle %0d", o.id);
    $finish;
  end
endmodule
"#;

#[test]
fn typedef_static_call_writes_back_actuals() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    let sim = simulate(SRC, 1_000).expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        [
            "T|statement 4",
            "T|inout 6",
            "T|ref 7",
            "T|task 8",
            "T|if-cond 5",
            "T|operand 1 5",
            "T|fourth 5",
            "T|two 5 5",
            "T|null-over 1",
            "T|typedef-of-typedef 5",
            "T|int 17",
            "T|class-scoped typedef 17",
            "T|plain typedef 41",
            "T|string hey",
            "T|vector abc",
            "T|enum BLUE",
            "T|type param typedef 23",
            "T|type param spec 24",
            "T|module var 31",
            "T|member 32",
            "T|member handle 33",
            "T|module code 44",
            "T|module code handle 45",
        ]
    );
}
