// Regression test: a STATIC class method called through a NULL object handle
// must run and return its result, including once the method has been
// compiled.
//
// §8.7/§8.23/§8.25: a static method neither needs nor references `this`, so
// calling one through a null handle is legal. §8.20 resolves a call through a
// handle on the handle's DECLARED class for everything a subclass cannot
// override — a static method always.
//
// The compiled `CallMethod` lowering carries only the handle register, so the
// runtime dispatched on the LIVE object's class. With a null handle there is
// no object, and the runtime returned a silent 32-bit zero instead of running
// the method. The interpreter path (`XEZIM_METHOD_TIER` high enough that the
// body is not compiled) was already correct, so the bug only appeared once
// the caller had been compiled.

use std::process::Command;

fn xezim() -> String {
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, tag: &str, tier: &str) -> String {
    let path = format!("/tmp/null_handle_static_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .env("XEZIM_METHOD_TIER", tier)
        .output()
        .expect("xezim failed to start");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

const DIRECT: &str = r#"
module top;
  class item_c;
    static function logic [2:0] calc(logic [28:0] payload);
      bit [3:0] poly;
      logic [31:0] pad;
      poly = 4'b1011;
      pad  = {payload, 3'b000};
      for (int i = 31; i >= 3; i = i - 1) begin
        if (pad[i]) pad[i-:4] = pad[i-:4] ^ poly;
      end
      return pad[2:0];
    endfunction
  endclass

  class holder_c;
    item_c null_h;   // never constructed
    item_c real_h;   // constructed
  endclass

  holder_c h;
  initial begin
    h = new();
    h.real_h = new();
    $display("RESULT null=%b real=%b expect=011/011",
             h.null_h.calc(29'h1FFFFFFF), h.real_h.calc(29'h1FFFFFFF));
    $finish;
  end
endmodule
"#;

const INHERITED: &str = r#"
module top;
  class base_c;
    static function int twice(int v);
      return 2 * v;
    endfunction
  endclass
  class derived_c extends base_c;
  endclass
  class holder_c;
    derived_c h;   // never constructed
  endclass
  holder_c o;
  initial begin
    o = new();
    $display("RESULT v=%0d expect=42", o.h.twice(21));
    $finish;
  end
endmodule
"#;

/// The compiled path (`tier 0` compiles on first call) must agree with the
/// interpreted one.
#[test]
fn static_method_through_null_handle_returns_its_result() {
    for tier in ["0", "1000000"] {
        let out = run(DIRECT, "direct", tier);
        assert!(
            out.contains("RESULT null=011 real=011"),
            "tier {tier}: unexpected output:\n{out}"
        );
    }
}

/// A static method reached through a handle whose declared class only
/// INHERITS it must resolve on the declared chain.
#[test]
fn static_method_through_null_handle_walks_the_declared_chain() {
    let out = run(INHERITED, "inherited", "0");
    assert!(out.contains("RESULT v=42"), "unexpected output:\n{out}");
}

const LIVE: &str = r#"
module top;
  class base_c;
    static function int val();
      return 11;
    endfunction
  endclass
  class derived_c extends base_c;
    // A same-named static HIDES the base one; it does not override it
    // (§8.23: static methods are not virtual).
    static function int val();
      return 77;
    endfunction
  endclass
  class holder_c;
    base_c h;   // declared base_c
  endclass
  holder_c o;
  initial begin
    o = new();
    o.h = new();
    $display("RESULT live=%0d expect=11", o.h.val());
    $finish;
  end
endmodule
"#;

/// The same declared-class resolution must apply when the handle IS live:
/// a static method is bound at compile time, never to the dynamic class.
#[test]
fn static_method_through_live_handle_binds_the_declared_class() {
    for tier in ["0", "1000000"] {
        let out = run(LIVE, "live", tier);
        assert!(
            out.contains("RESULT live=11"),
            "tier {tier}: unexpected output:\n{out}"
        );
    }
}

const THIS_MEMBER: &str = r#"
module top;
  class helper_c;
    static function int twice(int v);
      return 2 * v;
    endfunction
  endclass
  class holder_c;
    helper_c aux;   // never constructed
    int v;
    function void run();
      v = this.aux.twice(21);
    endfunction
  endclass
  holder_c h;
  initial begin
    h = new();
    h.run();
    $display("RESULT this.member=%0d expect=42", h.v);
    $finish;
  end
endmodule
"#;

/// `this.aux.m()` parses the keyword receiver as its own node, so the
/// declared-class resolution has to accept that spelling too.
#[test]
fn static_method_through_null_this_member_returns_its_result() {
    for tier in ["0", "1000000"] {
        let out = run(THIS_MEMBER, "this_member", tier);
        assert!(
            out.contains("RESULT this.member=42"),
            "tier {tier}: unexpected output:\n{out}"
        );
    }
}
