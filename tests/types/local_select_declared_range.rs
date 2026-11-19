//! §7.4.1, §11.5.1, §13.5: a bit-, part- or indexed part-select of a
//! task or function local, formal or return variable declared with an
//! ascending or offset range (`reg [0:7] n; n[0] = 1'b1;`, `reg [8:1] n;
//! n[1] = 1'b1;`) names the bits its range labels: the leftmost bound labels
//! the most significant bit, so `n[0]` of `[0:7]` is bit 7. The writes landed
//! on the raw bit position of the value (and part-selects of an ascending
//! local on the mirrored bits), a class method's vector formal read and wrote
//! the raw bits too, and a local beside a design variable of its name wrote
//! that variable (#1298). Each design prints PASS or FAIL per check; the
//! expected values are those of the LRM, which the reference simulator
//! gives for every check. Run with methods compiled (the default and on the
//! first call) and interpreted.

use std::process::Command;

fn run(src: &str, env: &[(&str, &str)]) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_local_select_range_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "-s",
        "top",
        path.to_str().unwrap(),
        "--no-cache",
    ])
    .env_remove("XEZIM_METHOD_CACHE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

/// Every check passes, in every mode, and there are `checks` of them.
fn all_pass(src: &str, checks: usize) {
    for env in [
        &[][..],
        &[("XEZIM_METHOD_TIER", "0")][..],
        &[("XEZIM_COMPILE_METHODS", "0")][..],
    ] {
        let text = run(src, env);
        let fails: Vec<&str> = text.lines().filter(|l| l.starts_with("FAIL")).collect();
        assert!(fails.is_empty(), "{env:?}: {fails:#?}\n{text}");
        let passes = text.lines().filter(|l| l.starts_with("PASS")).count();
        assert_eq!(passes, checks, "{env:?}:\n{text}");
        assert!(text.contains("DONE: 0 errors"), "{env:?}:\n{text}");
    }
}

const DECLARED_RANGE: &str = r##"
module top;
  int errors = 0;
  task automatic check(string what, logic [127:0] got, logic [127:0] want);
    if (got === want) $display("PASS %s", what);
    else begin
      errors++;
      $display("FAIL %s: got %h, expected %h", what, got, want);
    end
  endtask

  // writes into a local declared [0:7]
  function [7:0] asc_first();   reg [0:7] n; begin n = 0; n[0] = 1'b1; asc_first = n; end endfunction
  function [7:0] asc_last();    reg [0:7] n; begin n = 0; n[7] = 1'b1; asc_last = n; end endfunction
  function [7:0] asc_var(input int i);
                                reg [0:7] n; begin n = 0; n[i] = 1'b1; asc_var = n; end endfunction
  function [7:0] asc_part();    reg [0:7] n; begin n = 0; n[0:3] = 4'hF; asc_part = n; end endfunction
  function [7:0] asc_up();      reg [0:7] n; begin n = 0; n[2 +: 3] = 3'b111; asc_up = n; end endfunction
  function [7:0] asc_down();    reg [0:7] n; begin n = 0; n[4 -: 3] = 3'b111; asc_down = n; end endfunction
  function [7:0] asc_rmw(input [7:0] v);
                                reg [0:7] n; begin n = v; n[0] = ~n[0]; asc_rmw = n; end endfunction
  function automatic [7:0] asc_auto();
                                reg [0:7] n; begin n = 0; n[0] = 1'b1; asc_auto = n; end endfunction
  function [7:0] asc_formal(input [0:7] n);
                                begin n[0] = 1'b1; asc_formal = n; end endfunction
  task automatic asc_task(output [7:0] o);
                                reg [0:7] n; begin n = 0; n[0] = 1'b1; o = n; end endtask

  // writes into a local declared with a low bound that is not 0
  function [7:0] off_low();     reg [8:1] n; begin n = 0; n[1] = 1'b1; off_low = n; end endfunction
  function [7:0] off_high();    reg [8:1] n; begin n = 0; n[8] = 1'b1; off_high = n; end endfunction
  function [7:0] off_part();    reg [8:1] n; begin n = 0; n[8:5] = 4'hA; off_part = n; end endfunction
  function [7:0] off_asc();     reg [1:8] n; begin n = 0; n[1] = 1'b1; off_asc = n; end endfunction

  // wider than one word
  function [32:0]  asc_33();    reg [0:32]  n; begin n = 0; n[0] = 1'b1; asc_33 = n; end endfunction
  function [64:0]  asc_65();    reg [0:64]  n; begin n = 0; n[0] = 1'b1; asc_65 = n; end endfunction
  function [127:0] asc_128();   reg [0:127] n; begin n = 0; n[0] = 1'b1; asc_128 = n; end endfunction

  // controls
  function [7:0] desc_first();  reg [7:0] n; begin n = 0; n[0] = 1'b1; desc_first = n; end endfunction
  function [7:0] asc_read(input [7:0] v);
                                reg [0:7] n; begin n = v; asc_read = {7'b0, n[0]}; end endfunction
  reg [0:7] m;

  logic [7:0] o;
  initial begin
    check("[0:7] local, bit 0",              asc_first(), 8'h80);
    check("[0:7] local, bit 7",              asc_last(),  8'h01);
    check("[0:7] local, variable index",     asc_var(2),  8'h20);
    check("[0:7] local, part-select [0:3]",  asc_part(),  8'hF0);
    check("[0:7] local, [2 +: 3]",           asc_up(),    8'h38);
    check("[0:7] local, [4 -: 3]",           asc_down(),  8'h38);
    check("[0:7] local, read-modify-write",  asc_rmw(8'hA5), 8'h25);
    check("[0:7] automatic function local",  asc_auto(),  8'h80);
    check("[0:7] formal",                    asc_formal(8'h00), 8'h80);
    asc_task(o);
    check("[0:7] task local",                o,           8'h80);
    check("[8:1] local, bit 1",              off_low(),   8'h01);
    check("[8:1] local, bit 8",              off_high(),  8'h80);
    check("[8:1] local, part-select [8:5]",  off_part(),  8'hA0);
    check("[1:8] local, bit 1",              off_asc(),   8'h80);
    check("[0:32] local, bit 0",             asc_33(),    33'h1_0000_0000);
    check("[0:64] local, bit 0",             asc_65(),    65'h1_0000_0000_0000_0000);
    check("[0:127] local, bit 0",            asc_128(),   128'h8000_0000_0000_0000_0000_0000_0000_0000);
    check("control: [7:0] local, bit 0",     desc_first(), 8'h01);
    check("control: [0:7] local, read",      asc_read(8'h80), 8'h01);
    m = 0; m[0] = 1'b1;
    check("control: [0:7] module variable",  m,           8'h80);
    $display("DONE: %0d errors", errors);
    $finish;
  end
endmodule
"##;

const OTHER_FORMS: &str = r##"
module top;
  typedef logic [0:7] av_t;     // ascending
  typedef reg   [8:1] ov_t;     // offset
  int errors = 0;
  task automatic check(string what, logic [127:0] got, logic [127:0] want);
    if (got === want) $display("PASS %s", what);
    else begin
      errors++;
      $display("FAIL %s: got %h, expected %h", what, got, want);
    end
  endtask

  function [7:0] typedef_asc();  av_t n; begin n = 0; n[0] = 1'b1; typedef_asc = n; end endfunction
  function [7:0] typedef_off();  ov_t n; begin n = 0; n[1] = 1'b1; typedef_off = n; end endfunction
  function logic [0:7] ret_var(); begin ret_var = 0; ret_var[0] = 1'b1; end endfunction
  function [7:0] loop_index();   reg [0:7] n; integer i;
                                 begin n = 0; for (i = 0; i < 8; i = i + 2) n[i] = 1'b1; loop_index = n; end endfunction
  function automatic [7:0] ref_formal(ref reg [0:7] n);
                                 begin n[0] = 1'b1; ref_formal = n; end endfunction
  function automatic [7:0] static_keep();
                                 static reg [0:7] n = 0; begin n[0] = 1'b1; static_keep = n; end endfunction
  function [7:0] signed_asc();   reg signed [0:7] n; begin n = 0; n[0] = 1'b1; signed_asc = n; end endfunction

  class holder;
    reg [0:7] p;
    function logic [7:0] method_local(); reg [0:7] n; begin n = 0; n[0] = 1'b1; method_local = n; end endfunction
    task task_local(output logic [7:0] o); reg [0:7] n; begin n = 0; n[0] = 1'b1; o = n; end endtask
    function logic [7:0] property_bit(); begin p = 0; p[0] = 1'b1; property_bit = p; end endfunction
  endclass

  reg [0:7] mv;
  reg [8:1] mo;
  reg [0:7] arg;
  holder h;
  logic [7:0] o;
  initial begin
    h = new;
    check("typedef logic [0:7] local, bit 0",   typedef_asc(), 8'h80);
    check("typedef reg [8:1] local, bit 1",     typedef_off(), 8'h01);
    check("function return variable [0:7]",     ret_var(),     8'h80);
    check("variable index in a loop, [0:7]",    loop_index(),  8'hAA);
    arg = 0;
    check("ref formal [0:7], result",            ref_formal(arg), 8'h80);
    check("ref formal [0:7], actual",            arg,           8'h80);
    check("static local [0:7] with initializer", static_keep(), 8'h80);
    check("signed [0:7] local, bit 0",          signed_asc(),  8'h80);
    check("class method local [0:7]",           h.method_local(), 8'h80);
    h.task_local(o);
    check("class task local [0:7]",             o,             8'h80);
    check("control: class property [0:7]",      h.property_bit(), 8'h80);
    mv = 0; mv[0] = 1'b1;
    check("control: module variable [0:7]",     mv,            8'h80);
    mo = 0; mo[1] = 1'b1;
    check("control: module variable [8:1]",     mo,            8'h01);
    $display("DONE: %0d errors", errors);
    $finish;
  end
endmodule
"##;

const NEIGHBOURS: &str = r##"
module sub #(parameter int W = 8);
  reg [0:7] n;          // module variable named like the function's local
  function [7:0] g();
    reg [7:0] n;        // descending local, shadows the ascending module var
    begin n = 0; n[0] = 1'b1; g = n; end
  endfunction
  function [7:0] h();
    reg [0:W-1] q;      // parameter-sized ascending local
    begin q = 0; q[0] = 1'b1; q[W-2 +: 2] = 2'b10; h = q; end
  endfunction
endmodule

module top;
  int errors = 0;
  task automatic check(string what, logic [127:0] got, logic [127:0] want);
    if (got === want) $display("PASS %s", what);
    else begin errors++; $display("FAIL %s: got %h, expected %h", what, got, want); end
  endtask

  // carried static local: bit 0 toggles at every call
  function [7:0] carry_bit();
    reg [0:7] n;
    begin n[0] = (n[0] === 1'b1) ? 1'b0 : 1'b1; n[7] = 1'b0; n[1:6] = 6'b0; carry_bit = n; end
  endfunction
  // static task with a nonblocking select write into its static local
  reg [0:7] seen;
  task nba_task();
    reg [0:7] t;
    begin t = 0; t[1] <= 1'b1; #1 seen = t; end
  endtask
  // output formal, ascending
  task out_formal(output [0:7] o);
    begin o = 0; o[0] = 1'b1; o[6:7] = 2'b11; end
  endtask
  // inout formal, offset
  task inout_formal(inout [8:1] o);
    begin o[1] = 1'b1; o[8] = 1'b1; end
  endtask
  // return variable ascending, part and indexed
  function [0:7] ret_asc();
    begin ret_asc = 0; ret_asc[0] = 1'b1; ret_asc[5 +: 2] = 2'b11; end
  endfunction
  // queries of the local
  function automatic [31:0] queries();
    reg [0:7] n; reg [11:4] m;
    begin queries = {8'($left(n)), 8'($right(n)), 8'($left(m)), 8'($bits(m))}; end
  endfunction
  // negative and mixed-sign ranges
  function automatic [7:0] neg_range();
    reg [3:-4] n; reg [-8:-1] a;
    begin n = 0; a = 0; n[-4] = 1'b1; n[3] = 1'b1; a[-8] = 1'b1; neg_range = n | a; end
  endfunction
  // out-of-range and x index writes are ignored
  function automatic [7:0] oob();
    reg [0:7] n; reg [8:1] m; logic [3:0] xi;
    begin n = 0; m = 0; xi = 4'bx; n[8] = 1'b1; m[0] = 1'b1; n[xi] = 1'b1; oob = n | m; end
  endfunction
  // concatenation lvalue of selects
  function automatic [7:0] concat_lv();
    reg [0:7] n;
    begin n = 0; {n[0], n[7]} = 2'b10; {n[2:3], n[4 +: 2]} = 4'b0111; concat_lv = n; end
  endfunction
  // compound assignment and increment on a select
  function automatic [7:0] compound();
    reg [0:7] n;
    begin n = 0; n[0] |= 1'b1; n[4:7] += 4'd3; compound = n; end
  endfunction
  // a named block local inside a function
  function automatic [7:0] block_local();
    begin : blk
      reg [0:7] t;
      t = 0; t[0] = 1'b1; t[7] = 1'b1;
      block_local = t;
    end
  endfunction
  // part-select reads of an ascending local (control)
  function automatic [7:0] reads(input [7:0] v);
    reg [0:7] n;
    begin n = v; reads = {n[0:3], n[4 +: 2], n[7 -: 2]}; end
  endfunction
  // class method local and formal
  class C;
    function logic [7:0] m(input logic [0:7] f);
      reg [8:1] q;
      begin q = 0; q[1] = 1'b1; f[0] = 1'b1; m = f | q; end
    endfunction
  endclass

  sub #(.W(8)) u();
  reg [0:7] n;          // top-level variable named like the function locals
  logic [7:0] o, io;
  C c;
  initial begin
    c = new;
    check("carried static local, first call", carry_bit(), 8'h80);
    check("carried static local, second call", carry_bit(), 8'h00);
    nba_task();
    check("nonblocking select of a static task local", seen, 8'h40);
    out_formal(o);
    check("output formal [0:7]", o, 8'h83);
    io = 0; inout_formal(io);
    check("inout formal [8:1]", io, 8'h81);
    check("return variable [0:7]", ret_asc(), 8'h86);
    check("queries of the locals", queries(), 32'h0007_0b08);
    check("negative ranges", neg_range(), 8'h81);
    check("out-of-range and x index writes", oob(), 8'h00);
    check("concatenation lvalue", concat_lv(), 8'h9c);
    check("compound assignment", compound(), 8'h83);
    check("named block local", block_local(), 8'h81);
    check("part-select reads", reads(8'hb6), 8'hb6);
    check("class method local and formal", c.m(8'h00), 8'h81);
    n = 0;
    check("descending local beside an ascending module var", u.g(), 8'h01);
    check("ascending module var untouched", u.n, 8'hxx);
    check("parameter-sized local", u.h(), 8'h82);
    check("top-level n untouched", n, 8'h00);
    $display("DONE: %0d errors", errors);
    $finish;
  end
endmodule
"##;

const METHODS_AND_SCOPES: &str = r##"
module top;
  int errors = 0;
  task automatic check(string what, logic [127:0] got, logic [127:0] want);
    if (got === want) $display("PASS %s", what);
    else begin errors++; $display("FAIL %s: got %h, expected %h", what, got, want); end
  endtask

  class C;
    function logic [7:0] m_bit(input logic [0:7] f); f[0] = 1'b1; return f; endfunction
    function logic [7:0] m_read(input logic [0:7] f); logic [7:0] r; r = f[0]; return r; endfunction
    function logic [7:0] m_off(input logic [8:1] f); f[1] = 1'b1; return f; endfunction
    function logic [7:0] m_part(input logic [0:7] f); f[0:3] = 4'hf; return f; endfunction
    function logic [0:7] m_ret(); m_ret = 0; m_ret[0] = 1'b1; m_ret[6 +: 2] = 2'b01; endfunction
    function logic [7:0] m_desc(input logic [7:0] f8); f8[0] = 1'b1; return f8; endfunction
    task t_out(input logic [0:7] f, output logic [7:0] o); f[0] = 1'b1; o = f; endtask
    static function logic [7:0] s_bit(input logic [0:7] f); f[0] = 1'b1; return f; endfunction
  endclass

  function automatic [7:0] loop_var();
    reg [0:7] n;
    begin n = 0; for (int i = 0; i < 8; i += 2) n[i] = 1'b1; loop_var = n; end
  endfunction
  function automatic [7:0] named_block();
    reg [0:7] n;
    begin n = 0; begin : b reg [3:0] t; t = 4'h1; n[t] = 1'b1; n[4:5] = 2'b11; end named_block = n; end
  endfunction
  function automatic [7:0] read_loop(input [7:0] v);
    reg [0:7] n; reg [7:0] r;
    begin n = v; r = 0; for (int i = 0; i < 8; i++) r[i] = n[i]; read_loop = r; end
  endfunction
  task automatic in_fork(output [7:0] o);
    reg [0:7] n;
    begin n = 0; fork begin n[0] = 1'b1; end join o = n; end
  endtask
  function [7:0] in_foreach();
    reg [0:7] n;
    begin n = 0; foreach (n[k]) if (k % 3 == 0) n[k] = 1'b1; in_foreach = n; end
  endfunction

  reg [0:7] f8;         // design variable named like a method formal
  C c;
  logic [7:0] o;
  initial begin
    c = new;
    f8 = 0;
    check("method formal [0:7], bit write", c.m_bit(8'h00), 8'h80);
    check("method formal [0:7], bit read", c.m_read(8'h80), 8'h01);
    check("method formal [8:1], bit write", c.m_off(8'h00), 8'h01);
    check("method formal [0:7], part write", c.m_part(8'h00), 8'hf0);
    check("method return variable [0:7]", c.m_ret(), 8'h81);
    check("method formal [7:0] beside a [0:7] variable", c.m_desc(8'h00), 8'h01);
    f8[0] = 1'b1;
    check("design variable keeps its range", f8, 8'h80);
    c.t_out(8'h00, o);
    check("method task formal [0:7]", o, 8'h80);
    check("static method formal [0:7]", C::s_bit(8'h00), 8'h80);
    check("loop variable index", loop_var(), 8'haa);
    check("named block", named_block(), 8'h4c);
    check("reads in a loop", read_loop(8'h80), 8'h01);
    in_fork(o);
    check("write in a fork", o, 8'h80);
    check("foreach index", in_foreach(), 8'h92);
    $display("DONE: %0d errors", errors);
    $finish;
  end
endmodule
"##;

/// Writes into locals, formals and task locals declared `[0:7]`, `[8:1]`,
/// `[1:8]` and ascending 33, 65 and 128 bits wide, with controls.
#[test]
fn declared_range_selects_of_locals_formals_and_tasks() {
    all_pass(DECLARED_RANGE, 20);
}

/// A typedef'd local, a return variable, a `ref` formal, a loop index, a
/// signed local, a static local with an initializer and the locals of a
/// class method and task, with controls.
#[test]
fn declared_range_selects_of_other_frame_variables() {
    all_pass(OTHER_FORMS, 13);
}

/// A carried static local, a nonblocking select of a static task local,
/// `output` and `inout` formals, `$left`/`$right`/`$bits`, negative bounds,
/// out-of-range and x writes, a concatenation lvalue, compound assignment,
/// a named block, part-select reads, a class method, and locals beside
/// design variables of their name in the top module and in an instance.
#[test]
fn declared_range_selects_neighbouring_shapes() {
    all_pass(NEIGHBOURS, 18);
}

/// Class method formals, return variable and task formal, a static method,
/// a method formal beside a design variable of its name, and locals written
/// from a `for` variable, a named block, a `fork` and a `foreach`.
#[test]
fn declared_range_selects_of_method_formals_and_nested_scopes() {
    all_pass(METHODS_AND_SCOPES, 14);
}
