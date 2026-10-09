//! §6.16 — character-INDEX writes into strings surfaced through four more
//! receiver shapes (follow-up to the frame-local fix):
//!
//!   1. a string declared as a TASK formal (`task automatic t(ref string s)`),
//!   2. a class member string (`this.s[i]` / `obj.s[i]`; also
//!      `<module>.s[i]` at module scope via a dotted hierarchical ident),
//!   3. a module-level string written from an `always` (comb) block,
//!   4. an element of a string ARRAY / QUEUE (`sa[1][0]`, `q[0][1]`).
//!
//! Each must replace a CHARACTER BYTE, not bit-bash the packed byte-vector
//! container. Verified byte-identical to reference simulators.

use xezim::simulate;

#[test]
fn task_string_formal_char_write() {
    let src = r#"
module top;
  task automatic t(ref string s);
    s[2] = "r";
  endtask
  function void f(output string a);
    a = "abcdef";
    t(a);
  endfunction
  initial begin
    string a = "abcdef";
    t(a);
    $display("TAGTASK %0s '%s'", (a == "abrdef") ? "PASS" : "FAIL", a);
    a = "abcdef";
    f(a);
    $display("TAGFUNC %0s '%s'", (a == "abrdef") ? "PASS" : "FAIL", a);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for tag in ["TAGTASK", "TAGFUNC"] {
        let line = out
            .lines()
            .find(|l| l.starts_with(tag))
            .unwrap_or_else(|| panic!("no {tag} line:\n{out}"));
        assert!(
            line.contains("PASS"),
            "expected {tag} PASS, got:\n{line}\n\nfull:\n{out}"
        );
    }
}

#[test]
fn class_member_string_char_write() {
    let src = r#"
class C;
  string s;
  function void w();
    s = "abcdef";
    this.s[2] = "r";
  endfunction
endclass
module top;
  C c;
  initial begin
    C m;
    c = new();
    c.s = "abcdef";
    c.s[2] = "r";
    $display("TAGTHIS %0s '%s'", (c.s == "abrdef") ? "PASS" : "FAIL", c.s);
    m = new();
    m.w();
    $display("TAGMETH %0s '%s'", (m.s == "abrdef") ? "PASS" : "FAIL", m.s);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for tag in ["TAGTHIS", "TAGMETH"] {
        let line = out
            .lines()
            .find(|l| l.starts_with(tag))
            .unwrap_or_else(|| panic!("no {tag} line:\n{out}"));
        assert!(
            line.contains("PASS"),
            "expected {tag} PASS, got:\n{line}\n\nfull:\n{out}"
        );
    }
}

#[test]
fn module_string_from_always_block() {
    let src = r#"
module top;
  string s = "abcdef";
  reg en;
  always @(en) begin
    if (en) s[2] = "r";
  end
  initial begin
    en = 0;
    #1 en = 1;
    #1 $display("TAGALW %0s '%s'", (s == "abrdef") ? "PASS" : "FAIL", s);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let line = out
        .lines()
        .find(|l| l.starts_with("TAGALW"))
        .unwrap_or_else(|| panic!("no TAGALW line:\n{out}"));
    assert!(
        line.contains("PASS"),
        "expected TAGALW PASS, got:\n{line}\n\nfull:\n{out}"
    );
}

#[test]
fn string_array_and_queue_element_char_write() {
    let src = r#"
module top;
  string sa[4];
  string q[$];
  initial begin
    sa[0] = "abcXYZ";
    sa[1] = "defghi";
    sa[2] = "ghWxyz";
    sa[3] = "zzzzzz";
    q.push_back("jklmno");
    q.push_back("pqrstu");
    sa[1][0] = "R";
    q[1][2] = "Z";
    sa[2][2] = "W";
    $display("TAGARR0 %0s '%s'", (sa[0] == "abcXYZ") ? "PASS" : "FAIL", sa[0]);
    $display("TAGARR1 %0s '%s'", (sa[1] == "Refghi") ? "PASS" : "FAIL", sa[1]);
    $display("TAGQ0 %0s '%s'", (q[0] == "jklmno") ? "PASS" : "FAIL", q[0]);
    $display("TAGQ1 %0s '%s'", (q[1] == "pqZstu") ? "PASS" : "FAIL", q[1]);
    $display("TAGARR2 %0s '%s'", (sa[2] == "ghWxyz") ? "PASS" : "FAIL", sa[2]);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for tag in ["TAGARR0", "TAGARR1", "TAGQ0", "TAGQ1", "TAGARR2"] {
        let line = out
            .lines()
            .find(|l| l.starts_with(tag))
            .unwrap_or_else(|| panic!("no {tag} line:\n{out}"));
        assert!(
            line.contains("PASS"),
            "expected {tag} PASS, got:\n{line}\n\nfull:\n{out}"
        );
    }
}