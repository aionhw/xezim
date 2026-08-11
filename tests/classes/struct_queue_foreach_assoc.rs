//! Three bugs uncovered while bringing up the UVM `03data/12setcfg/30enumarray`
//! test (config-db enum/queue/sarray field application). Each has a minimal
//! pure-SV reproducer that fails on xezim and passes on the reference
//! simulator, verified byte-for-byte identical.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_pass(sim: &xezim::compiler::Simulator, tag: &str) {
    let msgs = messages(sim);
    let pass = msgs.iter().any(|m| m.contains(&format!("{}_PASS", tag)));
    let fail = msgs.iter().find(|m| m.contains(&format!("{}_FAIL", tag)));
    assert!(
        pass,
        "expected {}_PASS in output\nfail line: {:?}\nfull output: {:?}",
        tag, fail, msgs
    );
}

/// **Bug 1 — struct-element queue pop via a method wrapper.**
///
/// `return q.pop_front()` inside a user method (the `uvm_queue::pop_front`
/// shape) evaluated to a packed ZERO: a queue element's aggregate carries no
/// member leaves, so the struct's `name`/`regex` fields were lost. The fix
/// packs the popped element's member leaves BEFORE `eval_expr` shifts the
/// queue, then runs the pop for its side effect only.
const STRUCT_QUEUE_POP: &str = r#"
typedef struct {
    string name;
    string regex;
} ns_t;

class Qc;
    ns_t q[$];
    function void push(ns_t item);
        q.push_back(item);
    endfunction
    function ns_t pop_front_raw();
        return q.pop_front();
    endfunction
    function ns_t pop_back_raw();
        return q.pop_back();
    endfunction
endclass

module top;
    initial begin
        ns_t p_front;
        ns_t p_back;
        int ok = 0;
        Qc hq;
        hq = new;
        hq.push('{"alpha", "a.*"});
        hq.push('{"beta",  "b.*"});
        hq.push('{"gamma", "g.*"});
        p_front = hq.pop_front_raw();
        if (p_front.name == "alpha" && p_front.regex == "a.*") ok++;
        p_back = hq.pop_back_raw();
        if (p_back.name == "gamma" && p_back.regex == "g.*") ok++;
        if (ok == 2)
            $display("SQP_PASS");
        else
            $display("SQP_FAIL ok=%0d", ok);
    end
endmodule
"#;

#[test]
fn struct_queue_pop_via_method() {
    let sim = simulate(STRUCT_QUEUE_POP, 200).expect("simulate failed");
    assert_pass(&sim, "SQP");
}

/// **Bug 2 — `foreach` on a local `string [$]` queue.**
///
/// A local string-element queue's name was in `string_signals`, so the
/// `foreach` handler treated it as a scalar STRING and tried to iterate
/// its characters (0 iterations). The fix guards the `string_signals`
/// check to skip names that are actually queues / dynamic arrays.
const FOREACH_STR_QUEUE: &str = r#"
module top;
    initial begin
        string q[$];
        int count = 0;
        q.push_back("aaa");
        q.push_back("bbb");
        q.push_back("ccc");
        foreach(q[i])
            count++;
        if (count == 3)
            $display("FSQ_PASS");
        else
            $display("FSQ_FAIL count=%0d", count);
    end
endmodule
"#;

#[test]
fn foreach_local_string_queue() {
    let sim = simulate(FOREACH_STR_QUEUE, 200).expect("simulate failed");
    assert_pass(&sim, "FSQ");
}

/// **Bug 4 — method-local assoc array of CLASS references: method-call on an
/// element lost its mutation.**
///
/// `all[k].push_back(v)` where `all` is a method-LOCAL associative array of
/// (class-handle) objects. The receiver `all[k]` resolves to the element
/// handle via `eval_handle_expr`, whose `Index` arm looked the element up
/// under the BARE name (`all[k]`) in the global signal table — but a
/// method-local dynamic/assoc array is renamed to a process-unique storage
/// key (`@all#0[k]`) by `declare_local_dyn`. The miss returned handle 0 →
/// null receiver → the user method (Q::push_back) was never called. Fix:
/// resolve the base through `dyn_name_lookup` before indexing. Matches the
/// class-member case (which already worked) and the reference simulator.
const ASSOC_LOCAL_METHOD: &str = r#"
class Q;
  int count;
  int items[$];
  function void push_back(int v); items.push_back(v); count++; endfunction
  function int size(); return count; endfunction
endclass

class C;
  function void test();
    Q all[string];
    all["a"] = new;
    all["a"].push_back(5);
    all["a"].push_back(7);
    if (all["a"].size() == 2)
      $display("ALM_PASS");
    else
      $display("ALM_FAIL size=%0d", all["a"].size());
  endfunction
endclass

module top;
  initial begin
    C c;
    c = new;
    c.test();
  end
endmodule
"#;

#[test]
fn assoc_local_method_elem_call() {
    let sim = simulate(ASSOC_LOCAL_METHOD, 200).expect("simulate failed");
    assert_pass(&sim, "ALM");
}

/// **Bug 3 — `foreach` over a string-keyed assoc array whose key contains `]`.**
///
/// The key extraction used `find(']')` (first `]`) to delimit the key from
/// the signal name `<array>[<key>]`. When the key itself contains `]`
/// (e.g. `"sa_num[0]"`), the first `]` truncated the key to `"sa_num[0"`.
/// The fix uses `rsplit_once(']')` (last `]`) for string-keyed arrays.
const ASSOC_BRACKET_KEY: &str = r#"
module top;
    int rtab [string];
    initial begin
        rtab["sa_num[0]"] = 42;
        if (rtab.exists("sa_num[0]") && rtab["sa_num[0]"] == 42)
            $display("ABK_PASS");
        else
            $display("ABK_FAIL");
    end
endmodule
"#;

#[test]
fn foreach_assoc_bracket_key() {
    let sim = simulate(ASSOC_BRACKET_KEY, 200).expect("simulate failed");
    assert_pass(&sim, "ABK");
}
