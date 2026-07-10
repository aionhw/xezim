//! `%p` on associative arrays (IEEE 1800-2017 §21.2.1.7): they print as
//! `'{key:value, ...}` over the populated keys, not as a single element.
//! Integer keys sort numerically, string keys are quoted, and struct elements
//! recurse.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef enum {ON, OFF} toggle_e;
  typedef struct { toggle_e tgl; string str; } combo_t;

  int     ai[int];
  int     as[string];
  combo_t cs[int];

  initial begin
    // Insert out of order: integer keys must print in numeric order.
    ai[20] = 200; ai[3] = 30; ai[10] = 100;
    as["b"] = 2;  as["a"] = 1;
    cs[5].tgl = OFF; cs[5].str = "five";

    $display("AI=%p", ai);
    $display("AS=%p", as);
    $display("CS=%p", cs);
  end
endmodule
"#;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn p_format_assoc_arrays_print_key_value_pairs() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // Integer keys: numeric order, not lexical ("10" < "20" < "3" lexically).
    assert_eq!(line(&sim, "AI="), "AI='{3:30, 10:100, 20:200}");
    // String keys are quoted.
    assert_eq!(line(&sim, "AS="), r#"AS='{"a":1, "b":2}"#);
    // Struct elements recurse (enum label + quoted string).
    assert_eq!(line(&sim, "CS="), r#"CS='{5:'{tgl:OFF, str:"five"}}"#);
}
