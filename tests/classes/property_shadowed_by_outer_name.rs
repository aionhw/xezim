//! IEEE 1800-2023 §23.9, §8.11, §6.16.1, §13.5.5: a name referenced inside a
//! class method resolves in the method's own locals/formals, then the class's
//! properties (including inherited properties via `this`), before any enclosing
//! module-scope variable — and never searches a caller's frame.
//!
//! Previously:
//! - `get_local_signal_or_static` checked module-scope signals before `this`
//!   properties and class statics, so an initial-block local or unrelated
//!   module `int` of the same name hijacked `.len()` / `.substr()` / etc.;
//! - `class_of_var` fell through to the flat `var_class_types` map for
//!   non-class properties;
//! - `task_frame_string_flag` and `class_member_is_string` scanned all of
//!   `local_stack` instead of bounding to `method_local_base`, letting a
//!   caller's same-named local shadow `this`'s `string` property.

use xezim::simulate;

const CLASS_DEFS: &str = r#"
class Base;
  string value;
  function new(string s); value = s; endfunction
  function int len();          return value.len; endfunction
  function int a_np();         return value.len; endfunction
  function int a_p();          return value.len(); endfunction
  function int a_this();       return this.value.len; endfunction
  function int a_idx();        return value[0] == "/"; endfunction
  function int a_cmp();        return value == "/a/b"; endfunction
  function string a_cat();     return {value, "!"}; endfunction
  function string a_substr();  return value.substr(1, 2); endfunction
  function string a_toupper(); return value.toupper(); endfunction
  function int a_local();
    string value = "/x/y/z";
    return (value.len == 6) && (value.len() == 6) && (this.value.len == 4)
        && (value[1] == "x") && (value.substr(1, 2) == "x/")
        && (value.toupper() == "/X/Y/Z");
  endfunction
endclass

class S extends Base;
  function new(string s); super.new(s); endfunction
endclass

`define CHECK(tag, h) \
  if ((h.len() == 4) && (h.a_np() == 4) && (h.a_p() == 4) && (h.a_this() == 4) \
      && (h.a_idx() == 1) && (h.a_cmp() == 1) && (h.a_cat() == "/a/b!") \
      && (h.a_substr() == "a/") && (h.a_toupper() == "/A/B") && (h.a_local() == 1)) \
    $display("PASS %s", tag); \
  else \
    $display("FAIL %s len=%0d np=%0d p=%0d this=%0d idx=%0d cmp=%0d cat=%s sub=%s up=%s loc=%0d", \
      tag, h.len(), h.a_np(), h.a_p(), h.a_this(), h.a_idx(), h.a_cmp(), \
      h.a_cat(), h.a_substr(), h.a_toupper(), h.a_local());
"#;

fn run_case(label: &str, body: &str, expected_tags: &[&str]) {
    let src = format!("{CLASS_DEFS}\n{body}");
    let sim = simulate(&src, 1000).expect("simulate failed");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("PASS ") || l.starts_with("FAIL "))
        .collect();
    let expected: Vec<String> = expected_tags.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(lines, expected, "case {label}:\n{lines:#?}");
}

#[test]
fn property_shadowed_by_outer_name() {
    let callers_body = r#"
class K;
  function void m_other(string p); S v = new(p);     `CHECK("class-other", v) endfunction
  function void m_arg(string p);   S value = new(p); `CHECK("class-arg",   value) endfunction
  function void m_lit();           S value = new("/a/b"); `CHECK("class-lit", value) endfunction
endclass

module top;
  K k = new();
  function automatic void f_arg(string p); S value = new(p);      `CHECK("mod-func-arg", value) endfunction
  function automatic void f_lit();         S value = new("/a/b"); `CHECK("mod-func-lit", value) endfunction
  task automatic t_arg(string p);          S value = new(p);      `CHECK("mod-task-arg", value) endtask
  initial begin
    k.m_other("/a/b");
    k.m_arg("/a/b");
    k.m_lit();
    f_arg("/a/b");
    f_lit();
    t_arg("/a/b");
    begin
      S value = new("/a/b");
      `CHECK("init-local", value)
    end
    $finish;
  end
endmodule
"#;

    let mod_var_body = r#"
module top;
  int value = 5;
  S s;
  initial begin
    s = new("/a/b");
    `CHECK("mod-int", s)
    if (value != 5) $display("FAIL mod-int-clobbered val=%0d", value);
    $finish;
  end
endmodule
"#;

    let loop_body = r#"
class K;
  function void m_arg(string p);
    S value = new(p);
    `CHECK("loop-arg", value)
  endfunction
endclass
module top;
  int value = 5;
  K k = new();
  initial begin
    for (int i = 0; i < 25; i++) begin
      S value = new("/a/b");
      if ((value.len() != 4) || (value.a_np() != 4) || (value.a_p() != 4)
          || (value.a_this() != 4) || (value.a_idx() != 1) || (value.a_cmp() != 1)
          || (value.a_cat() != "/a/b!") || (value.a_substr() != "a/")
          || (value.a_toupper() != "/A/B") || (value.a_local() != 1)) begin
        $display("FAIL loop iter=%0d", i);
        $finish;
      end
    end
    $display("PASS loop-init");
    for (int i = 0; i < 25; i++) begin
      k.m_arg("/a/b");
    end
    $finish;
  end
endmodule
"#;

    let mut failures = Vec::new();
    for (label, body, expected) in [
        (
            "callers",
            callers_body,
            vec![
                "PASS class-other",
                "PASS class-arg",
                "PASS class-lit",
                "PASS mod-func-arg",
                "PASS mod-func-lit",
                "PASS mod-task-arg",
                "PASS init-local",
            ],
        ),
        ("mod-var", mod_var_body, vec!["PASS mod-int"]),
        (
            "loop",
            loop_body,
            [vec!["PASS loop-init"], vec!["PASS loop-arg"; 25]].concat(),
        ),
    ] {
        if let Err(e) = std::panic::catch_unwind(|| run_case(label, body, &expected)) {
            let msg = if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = e.downcast_ref::<&str>() {
                (*s).to_string()
            } else {
                format!("{label} panicked")
            };
            failures.push(msg);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
