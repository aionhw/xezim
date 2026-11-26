//! Compiled `CallMethod`: queue-literal actuals on unpacked collection formals.

use xezim::simulate;

fn show_lines(body: &str) -> Vec<String> {
    let _ = super::compiled_method_test_env::eager();
    let src = format!(
        r#"
module top;
  typedef string sq_t[$];
  class C;
    function void show(string s[$]);
      $display("T|sz=%0d first=%s last=%s", s.size(), s[0], s[$]);
    endfunction
    function void show_t(sq_t s);
      show(s);
    endfunction
    task run();
      string q[$];
      {body}
    endtask
  endclass
  initial begin
    automatic C c = new;
    c.run();
  end
endmodule
"#
    );
    simulate(&src, 50)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn compiled_method_queue_string_literal_arg() {
    assert_eq!(
        show_lines(r#"show({"nop"});"#),
        vec!["T|sz=1 first=nop last=nop"]
    );
}

#[test]
fn compiled_method_queue_multi_literal_arg() {
    assert_eq!(
        show_lines(r#"show({"a", "bc"});"#),
        vec!["T|sz=2 first=a last=bc"]
    );
}

#[test]
fn compiled_method_queue_typedef_literal_arg() {
    assert_eq!(
        show_lines(r#"show_t({"a", "bc"});"#),
        vec!["T|sz=2 first=a last=bc"]
    );
}

#[test]
fn compiled_method_queue_string_variable_arg() {
    assert_eq!(
        show_lines(r#"q.push_back("one"); q.push_back("two"); show(q);"#),
        vec!["T|sz=2 first=one last=two"]
    );
}
