//! §6.21, §6.16: a `string` local of one subroutine does not make an
//! unrelated variable of the same name a string. String-ness is tracked by
//! bare name in a flat set; a function's fresh `string state` local stayed
//! in it after the function returned (so did a task's, once the task had
//! suspended), and a later task's enum formal `state` then compared as text
//! (`256 >= 16` was false, so `wait (m_state >= state)` never
//! released) and bit-selected as a character. Expected values are the
//! reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn string_local_does_not_retype_a_same_named_formal() {
    const SRC: &str = r#"
typedef enum logic [8:0] { EXECUTING = 16, DONE = 256 } phase_state;
class string_user;
  function string describe(int i);
    string state;
    state = {"ab", $sformatf("%0d", i)};
    return state;
  endfunction
  task note();
    string state;
    state = "zz";
    #1 $display("T|task local %s len=%0d", state, state.len());
  endtask
endclass
class phase;
  phase_state m_state = DONE;
  task wait_for_state(phase_state state);
    bit [17:0] c;
    c = {state, 9'd1};
    $display("T|ge=%0b lt=%0b bit4=%0b cat=%0h", m_state >= state, state < m_state, state[4], c);
    wait (m_state >= state);
    $display("T|released t=%0t", $time);
  endtask
endclass
module top;
  initial begin
    string_user u = new();
    phase p = new();
    $display("T|%s %s", u.describe(1), u.describe(2));
    u.note();
    p.wait_for_state(EXECUTING);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|ab1 ab2",
            "T|task local zz len=2",
            "T|ge=1 lt=1 bit4=1 cat=2001",
            "T|released t=1",
        ]
    );
}
