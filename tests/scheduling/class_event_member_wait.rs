//! §15.5 event controls and triggers on event CLASS PROPERTIES reached
//! through receivers with runtime selects — reference-validated.
//!
//! `@(m_events[obj].all_dropped)` inside a class method must PARK the
//! process until `-> m_events[obj].all_dropped` fires; it previously fell
//! into the delta-yield and woke at t=0. That spurious wakeup is exactly
//! the UVM objection spin (GitHub #109): `uvm_objection::wait_for` re-ran
//! forever inside `uvm_phase::wait_for_self_and_siblings_to_drop`, so a
//! 1800.2-2017 UVM test never left time 0. The trigger side was equally
//! broken: the `->` parser flattened the target to a dotted string that
//! cannot express a runtime select, so it fired a phantom name.
//!
//! Wake times are stamped in MODULE scope right after each task returns —
//! hierarchical `tb.x` writes from class methods are a separate open gap.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// All four receiver shapes — plain property, assoc keyed by class handle,
/// assoc keyed by int, queue element — park and wake at the trigger time.
/// Reference: direct=10, by_obj=20, by_int=30, queue=40.
#[test]
fn member_event_waits_park_until_triggered() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  int t_direct = -1, t_by_obj = -1, t_by_int = -1, t_queue = -1;

  class events_t;
    event ev;
  endclass

  class holder_t;
    events_t direct;
    events_t by_obj[holder_t];
    events_t by_int[int];
    events_t q[$];

    function void init(holder_t k);
      events_t tmp;
      direct = new;
      by_obj[k] = new;
      by_int[7] = new;
      tmp = new; q.push_back(tmp);
    endfunction

    task wait_direct;             @(direct.ev);    endtask
    task wait_by_obj(holder_t k); @(by_obj[k].ev); endtask
    task wait_by_int;             @(by_int[7].ev); endtask
    task wait_q;                  @(q[0].ev);      endtask

    task fire_all(holder_t k);
      #10 ->direct.ev;
      #10 ->by_obj[k].ev;
      #10 ->by_int[7].ev;
      #10 ->q[0].ev;
    endtask
  endclass

  holder_t h, key;

  initial begin
    h = new; key = new;
    h.init(key);
    fork
      begin h.wait_direct();     t_direct = $time; end
      begin h.wait_by_obj(key);  t_by_obj = $time; end
      begin h.wait_by_int();     t_by_int = $time; end
      begin h.wait_q();          t_queue  = $time; end
      h.fire_all(key);
    join
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "t_direct"), 10, "@(direct.ev) parks until ->direct.ev");
    assert_eq!(u(&sim, "t_by_obj"), 20, "@(by_obj[k].ev): assoc keyed by class handle");
    assert_eq!(u(&sim, "t_by_int"), 30, "@(by_int[7].ev): assoc keyed by int");
    assert_eq!(u(&sim, "t_queue"), 40, "@(q[0].ev): queue element");
}

/// §15.5.2 nonblocking `->>` on the same member shapes. The NBA-region
/// flush gate previously tested only `pending_nba_triggers`, so a slot
/// whose ONLY pending NBA work was an instance trigger never flushed and
/// the waiter hung forever. Reference: same wake times as the blocking form.
#[test]
fn member_event_nonblocking_triggers_flush_with_nba() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  int t_direct = -1, t_by_obj = -1, t_queue = -1;

  class events_t;
    event ev;
  endclass

  class holder_t;
    events_t direct;
    events_t by_obj[holder_t];
    events_t q[$];

    function void init(holder_t k);
      events_t tmp;
      direct = new;
      by_obj[k] = new;
      tmp = new; q.push_back(tmp);
    endfunction

    task wait_direct;             @(direct.ev);    endtask
    task wait_by_obj(holder_t k); @(by_obj[k].ev); endtask
    task wait_q;                  @(q[0].ev);      endtask

    task fire_all(holder_t k);
      #10 ->>direct.ev;
      #10 ->>by_obj[k].ev;
      #10 ->>q[0].ev;
    endtask
  endclass

  holder_t h, key;

  initial begin
    h = new; key = new;
    h.init(key);
    fork
      begin h.wait_direct();    t_direct = $time; end
      begin h.wait_by_obj(key); t_by_obj = $time; end
      begin h.wait_q();         t_queue  = $time; end
      h.fire_all(key);
    join
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "t_direct"), 10, "->>direct.ev flushes in the NBA region");
    assert_eq!(u(&sim, "t_by_obj"), 20, "->>by_obj[k].ev: instance trigger alone must flush");
    assert_eq!(u(&sim, "t_queue"), 30, "->>q[0].ev: queue element");
}

/// The UVM objection wait/drop protocol in miniature (uvm_objection::wait_for
/// against `m_events[obj]`): exists → new → waiters++ → @ → waiters-- →
/// delete. The @ must consume the whole wait (reference: woke_at=10), not
/// return in the same time step — the t=0 spurious wake was #109's spin.
#[test]
fn objection_wait_for_protocol_parks_once() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  int woke_at = -1;

  class obj_t;
    int id;
  endclass

  class events_t;
    event all_dropped;
    int   waiters;
  endclass

  class objection_t;
    events_t m_events[obj_t];

    task wait_for_drop(obj_t o);
      if (!m_events.exists(o)) begin
        m_events[o] = new;
      end
      m_events[o].waiters++;
      @(m_events[o].all_dropped);
      m_events[o].waiters--;
      if (m_events[o].waiters == 0)
        m_events.delete(o);
    endtask

    function void drop(obj_t o);
      if (m_events.exists(o))
        ->m_events[o].all_dropped;
    endfunction
  endclass

  objection_t obj;
  obj_t       top_h;

  initial begin
    obj = new; top_h = new;
    fork
      begin
        obj.wait_for_drop(top_h);
        woke_at = $time;
      end
      #10 obj.drop(top_h);
    join
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "woke_at"), 10, "the @ parks until the drop trigger (reference: 10)");
}
