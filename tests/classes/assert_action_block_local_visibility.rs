//! IEEE 1800-2023 §16.3, §8.18: a block-local declared in an immediate
//! assertion's fail statement is typed like any block-local, so a member
//! access through it is checked for visibility. A `local` member read through
//! such a local from another class used to pass unchecked, while the same
//! access in an `if` block was rejected.

const SRC: &str = r#"
class secret;
  local int hidden = 1;
  int shown = 2;
endclass
class user;
  task t();
    int a = 0;
    assert (a == 1) else begin secret s = new; $display("T|%0d", s.ACCESS); end
  endtask
endclass
module top;
  initial begin user u = new; u.t(); end
endmodule
"#;

#[test]
fn local_member_through_an_assert_else_block_local_is_rejected() {
    let err = xezim::simulate(&SRC.replace("ACCESS", "hidden"), 100)
        .err()
        .expect("a local member read from another class must be rejected");
    assert!(
        err.contains("Illegal access to local member 'hidden'"),
        "{err}"
    );
}

#[test]
fn public_member_through_an_assert_else_block_local_is_read() {
    let sim = xezim::simulate(&SRC.replace("ACCESS", "shown"), 100).expect("legal access");
    let got: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(got, ["T|2"]);
}
