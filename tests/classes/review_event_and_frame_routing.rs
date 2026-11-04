//! Review regressions of #795 and #799. A process already blocked on an
//! event when another event is assigned to it stays blocked: only the old
//! event wakes it (§15.5.5.1). A free function's class-handle formal `s`
//! takes stores `s.f = v` while its caller has an enum local `s` (§23.9).
//! Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn a_waiter_blocked_before_an_event_assignment_keeps_its_event() {
    let src = r#"
class C; event e1, e2, e3; endclass
module top;
  C c = new;
  initial begin
    fork
      begin @(c.e2); $display("T|%0t merged", $time); end
      begin @(c.e3); $display("T|%0t kept", $time); end
    join_none
    #1 c.e2 = c.e1;
    c.e3 = c.e1;
    #1 -> c.e1;
    #1 $display("T|%0t end", $time);
  end
endmodule
"#;
    assert_eq!(lines(src), ["T|3 end"]);
}

#[test]
fn a_waiter_on_the_old_event_wakes_through_another_name_for_it() {
    let src = r#"
class C; event e1, e2, e3; endclass
module top;
  C c = new;
  initial begin
    c.e3 = c.e2;
    fork
      begin @(c.e2); $display("T|%0t old", $time); end
    join_none
    #1 c.e2 = c.e1;
    #1 -> c.e3;
    #1 -> c.e1;
    #1 $display("T|%0t end", $time);
  end
endmodule
"#;
    assert_eq!(lines(src), ["T|2 old", "T|4 end"]);
}

#[test]
fn a_free_function_handle_formal_hides_the_callers_enum_local() {
    let src = r#"
typedef enum { INFO, WARNING, ERROR } sev_e;
class slave; string tag = "none"; int id; endclass
function automatic void setf(slave s); s.tag = "free"; s.id = 1; endfunction
class reporter;
  function void run(slave x);
    sev_e s;
    s = ERROR;
    setf(x);
  endfunction
endclass
module top;
  initial begin
    reporter r = new;
    slave x = new;
    r.run(x);
    $display("T|%s %0d", x.tag, x.id);
  end
endmodule
"#;
    assert_eq!(lines(src), ["T|free 1"]);
}
