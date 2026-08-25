//! NESTED typedef expansion of a parameterized-class specialization used as
//! a static method/qualifier base must collapse ALL the way through the
//! enclosing class's type params.
//!
//! In `tester#(T)::do_it()` the local typedef `cbs` aliases
//! `Calls#(event_type, cb_type)`, where `event_type`/`cb_type` are THEMSELVES
//! typedef to `uvm_event#(T)`/`uvm_event_callback#(T)` — i.e. a specialization
//! whose inner args still name the enclosing class's type param `T`. For
//! `T=string`/`T=int`, `cbs::put` must key the SAME per-spec static cell as
//! the explicit `Calls#(uvm_event#(string), uvm_event_callback#(string))`,
//! not a cell under a symbolic `T` or the inner class's DEFAULT type
//! (09callbacks/90Mantis/6033: `tester#(string)::do_it()` dispatched zero
//! callbacks because `get_all` keyed a different `uvm_callbacks#(...)` cell
//! than `add` did). Verified byte-for-byte against a commercial simulator:
//! `string-cell=[HI] int-cell=[HI]`. Without the nested-typedef fix the
//! `string`/`int` cells stay `unset`.

use xezim::simulate_multi;

#[test]
fn t_nested_typedef_spec_resolution() {
    let src = r#"
class Obj;
endclass
class Cb;
endclass
class uvm_event_callback #(type T=Obj) extends Cb;
endclass
class uvm_event #(type T=Obj);
endclass
class uvm_callbacks #(type T=Obj, type CB=Cb);
  static string mark = "unset";
  static function void put(string v); mark = v; endfunction
endclass
class tester #(type T=Obj);
  typedef uvm_event#(T) event_type;
  typedef uvm_event_callback#(T) cb_type;
  typedef uvm_callbacks#(event_type, cb_type) cbs;
  static task do_it();
    cbs::put("HI");
  endtask
endclass
module top;
  initial begin
    tester#(string)::do_it();
    tester#(int)::do_it();
    if (uvm_callbacks#(uvm_event#(string), uvm_event_callback#(string))::mark == "HI"
        && uvm_callbacks#(uvm_event#(int), uvm_event_callback#(int))::mark == "HI")
      $display("NESTPASS string-cell=%s int-cell=%s",
        uvm_callbacks#(uvm_event#(string), uvm_event_callback#(string))::mark,
        uvm_callbacks#(uvm_event#(int), uvm_event_callback#(int))::mark);
    else
      $display("NESTFAIL string-cell=%s int-cell=%s",
        uvm_callbacks#(uvm_event#(string), uvm_event_callback#(string))::mark,
        uvm_callbacks#(uvm_event#(int), uvm_event_callback#(int))::mark);
  end
endmodule
"#;
    let out: Vec<String> = simulate_multi(
        &[src.to_string()],
        1000,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
        None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let text = out.join("\n");
    assert!(
        text.contains("NESTPASS"),
        "nested typedef must collapse through the enclosing spec; got {:?}",
        out
    );
    assert!(!text.contains("NESTFAIL"), "got {:?}", out);
}