//! IEEE 1800-2017 §21.2.1.7 — `%m` in a MODULE-SCOPE variable initializer.
//! A non-constant initializer (here a class `new()` whose argument is
//! `$sformatf("%m...")`) is deferred to a time-0 static-init assignment that
//! must run under ITS OWN instance's scope. Otherwise every instance of a
//! multiply-instantiated module reports the same name (the top module's),
//! instead of the distinct per-instance hierarchical path. Verified against a
//! commercial simulator.

use xezim::simulate;

fn line(src: &str, top: &str) -> Vec<String> {
    xezim::simulate_multi(
        &[src.to_string()], 1000, Some(top), &[], &[], None, false, None, None,
        &[], &[], None, &[], 0, u64::MAX, None, &[], None, None, None, None, false, None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect()
}

#[test]
fn percent_m_in_deferred_class_init_is_per_instance() {
    let src = r#"
class A;
  string hname;
  function new(string nm);
    hname = nm;
  endfunction
endclass

module mod;
  A aaa = new ($sformatf("%m.aaa"));
endmodule

module test;
  mod mod1();
  mod mod2();
  initial begin
    $display("PASS=%s", mod1.aaa.hname);
    $display("PASS=%s", mod2.aaa.hname);
  end
endmodule
"#;
    let out = line(src, "test");
    assert!(
        out.iter().any(|l| l == "PASS=test.mod1.aaa"),
        "mod1 missing; got {:?}",
        out
    );
    assert!(
        out.iter().any(|l| l == "PASS=test.mod2.aaa"),
        "mod2 missing; got {:?}",
        out
    );
}