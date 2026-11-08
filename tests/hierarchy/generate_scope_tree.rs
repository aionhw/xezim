//! §27.5: a conditional generate that is the only item of a branch without
//! `begin` is directly nested. That branch is not a scope, and the inner
//! blocks take the outer construct's place and number. §27.6: an implicit
//! `genblk<n>` gets leading zeros until no explicit name of the scope conflicts.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| !m.is_empty() && !m.contains("$finish") && !m.contains("[xezim]"))
        .collect()
}

/// The `%m` of a process written in place of `@`, in module `top`.
fn m_of(body: &str) -> String {
    let body = body.replace('@', "initial $display(\"%m\");");
    let got = lines(&format!("module top;\n{body}\nendmodule\n"));
    assert_eq!(got.len(), 1, "{body}: {got:?}");
    got[0].clone()
}

#[test]
fn a_directly_nested_conditional_adds_no_scope() {
    let got = lines(
        r#"module top;
  if (1) if (1) begin : d
    task automatic t(); $display("%m"); endtask
    initial t();
  end
  if (1) begin : e
    task automatic t(); $display("%m"); endtask
    initial #1 t();
  end
endmodule
"#,
    );
    assert_eq!(got, ["top.d.t", "top.e.t"]);
}

#[test]
fn a_function_in_a_directly_nested_block_is_called_through_its_label() {
    let got = lines(
        r#"module top;
  if (1) if (1) begin : d
    function automatic int f(); return 1; endfunction
    initial $display("%m");
  end
  if (1) begin : e
    function automatic int f(); return 2; endfunction
  end
  initial #1 $display("%0d %0d", d.f(), e.f());
endmodule
"#,
    );
    assert_eq!(got, ["top.d", "1 2"]);
}

#[test]
fn every_arm_of_a_direct_if_and_case_chain_answers_to_one_label() {
    let got = lines(
        r#"module m #(parameter P=0); int r; initial r=x.f();
  if(P==1) begin:x function automatic int f(); return 1; endfunction end
  else if(P==2) begin:x function automatic int f(); return 2; endfunction end
  else case(P) 3: begin:x function automatic int f(); return 3; endfunction end
               default: begin:x function automatic int f(); return 4; endfunction end endcase endmodule
module top; m #(1) a(); m #(2) b(); m #(3) c(); m #(4) d();
  initial begin #1; $display("%0d %0d %0d %0d",a.r,b.r,c.r,d.r); end
endmodule
"#,
    );
    assert_eq!(got, ["1 2 3 4"]);
}

/// The shape of §27.5 Example 1, with data in place of the gates.
#[test]
fn the_blocks_of_the_lrm_direct_nesting_example_share_one_name() {
    let got = lines(
        r#"module test #(parameter p=0, q=0);
  if (p == 1)
    if (q == 0) begin : u1 int w = 1; end
    else if (q == 2) begin : u1 int w = 2; end
    else ;
  else if (p == 2)
    case (q)
      0, 1, 2: begin : u1 int w = 3; end
      default: begin : u1 int w = 4; end
    endcase
endmodule
module top;
  test #(1, 0) a(); test #(1, 2) b(); test #(2, 1) c(); test #(2, 9) d();
  initial #1 $display("%0d %0d %0d %0d", a.u1.w, b.u1.w, c.u1.w, d.u1.w);
endmodule
"#,
    );
    assert_eq!(got, ["1 2 3 4"]);
}

#[test]
fn an_instance_in_a_directly_nested_block_is_named_through_the_label() {
    let got = lines(
        r#"module leaf; initial $display("%m"); endmodule
module test;
  if (1) if (1) begin : u1 leaf g1(); end
endmodule
module top; test t(); endmodule
"#,
    );
    assert_eq!(got, ["top.t.u1.g1"]);
}

#[test]
fn an_unnamed_direct_nest_takes_the_number_of_the_outer_construct() {
    assert_eq!(m_of("if (1) if (1) @"), "top.genblk1");
    assert_eq!(m_of("if (1) ; if (1) if (1) @"), "top.genblk2");
    assert_eq!(m_of("if (1) if (0) ; else @"), "top.genblk1");
    assert_eq!(m_of("if (0) ; else case (1) 1: @ endcase"), "top.genblk1");
    assert_eq!(m_of("case (1) 1: if (1) @ endcase"), "top.genblk1");
    assert_eq!(
        m_of("case (1) default: case (1) 1: @ endcase endcase"),
        "top.genblk1"
    );
    assert_eq!(m_of("if (1) if (1) if (1) if (1) @"), "top.genblk1");
}

#[test]
fn a_construct_after_a_direct_nest_keeps_its_own_number() {
    let got = lines(
        r#"module top;
  if (1) if (1) initial $display("%m");
  if (1) initial $display("%m");
endmodule
"#,
    );
    assert_eq!(got, ["top.genblk1", "top.genblk2"]);
}

#[test]
fn a_labelled_block_under_a_case_chain_is_named_by_its_label() {
    assert_eq!(
        m_of("case (1) 1: case (1) default: begin : c @ end endcase endcase"),
        "top.c"
    );
    assert_eq!(
        m_of("if (0) ; else case (1) 1: begin : c @ end endcase"),
        "top.c"
    );
}

#[test]
fn a_begin_wrapped_nest_is_a_scope() {
    assert_eq!(
        m_of("if (1) begin if (1) begin : x @ end end"),
        "top.genblk1.x"
    );
    assert_eq!(m_of("if (1) begin if (1) @ end"), "top.genblk1.genblk1");
    assert_eq!(
        m_of("if (0) ; else begin case (1) 1: @ endcase end"),
        "top.genblk1.genblk1"
    );
    assert_eq!(
        m_of("case (1) 1: begin : a if (1) @ end endcase"),
        "top.a.genblk1"
    );
}

#[test]
fn a_block_with_two_items_is_a_scope() {
    assert_eq!(
        m_of("if (1) begin if (1) @ int w; end"),
        "top.genblk1.genblk1"
    );
}

#[test]
fn a_loop_is_never_directly_nested() {
    assert_eq!(
        m_of("if (1) for (genvar i = 0; i < 1; i++) @"),
        "top.genblk1.genblk1[0]"
    );
    assert_eq!(
        m_of("for (genvar i = 0; i < 1; i++) if (1) @"),
        "top.genblk1[0].genblk1"
    );
    assert_eq!(
        m_of("for (genvar i = 0; i < 1; i++) if (1) if (1) @"),
        "top.genblk1[0].genblk1"
    );
}

#[test]
fn a_direct_nest_inside_a_block_adds_nothing_to_that_block() {
    assert_eq!(
        m_of("if (1) begin : a if (1) if (1) begin : b @ end end"),
        "top.a.b"
    );
    assert_eq!(
        m_of("if (1) begin : a if (1) if (1) @ end"),
        "top.a.genblk1"
    );
    assert_eq!(
        m_of("for (genvar i = 0; i < 1; i++) begin : l if (1) if (1) begin : d @ end end"),
        "top.l[0].d"
    );
}

#[test]
fn an_instance_and_the_root_name_a_direct_nest_alike() {
    let got = lines(
        r#"module leaf; initial $display("%m"); endmodule
module child;
  if (1) if (1) begin : d initial $display("%m"); leaf v(); end
  if (1) if (1) begin initial $display("%m"); leaf v(); end
endmodule
module top; child u(); endmodule
"#,
    );
    assert_eq!(got.len(), 4, "{got:?}");
    for want in ["top.u.d", "top.u.d.v", "top.u.genblk2", "top.u.genblk2.v"] {
        assert!(got.iter().any(|l| l == want), "{want}: {got:?}");
    }
}

/// §27.6: an unnamed block has no name usable in a hierarchical name, so
/// `genblk1.f` in source is a tool extension. Pinned to keep the name in step.
#[test]
fn an_unnamed_direct_nest_names_its_subroutines_by_the_outer_number() {
    for (open, close) in [
        ("module top;", "endmodule"),
        ("module m;", "endmodule module top; m u(); endmodule"),
    ] {
        let got = lines(&format!(
            r#"{open}
  if (1) if (1) begin
    function automatic int f(); return 5; endfunction
    initial $display("%0d", f());
    initial #1 $display("%0d", genblk1.f());
  end
{close}
"#
        ));
        assert_eq!(got, ["5", "5"], "{open}");
    }
}

#[test]
fn subroutines_of_two_direct_nests_stay_apart() {
    let got = lines(
        r#"module top;
  if (1) if (1) begin
    function automatic int f(); return 1; endfunction
    initial $display("%0d", f());
  end
  if (1) if (1) begin
    function automatic int f(); return 2; endfunction
    initial #1 $display("%0d", f());
  end
endmodule
"#,
    );
    assert_eq!(got, ["1", "2"]);
}

#[test]
fn data_of_a_labelled_direct_nest_is_reached_through_the_label() {
    let got = lines(
        r#"module m; if (1) if (1) begin : d int x = 3; end endmodule
module top;
  if (1) if (1) begin : d int x = 4; end
  m u();
  initial #1 $display("%0d %0d", d.x, u.d.x);
endmodule
"#,
    );
    assert_eq!(got, ["4 3"]);
}

#[test]
fn unlabelled_data_of_a_direct_nest_is_named_by_the_outer_constructs_implicit_name() {
    let got = lines(
        r#"module top;
  if (1) if (1) begin int x = 3; end
  initial #1 $display("%0d", genblk1.x);
endmodule
"#,
    );
    assert_eq!(got, ["3"]);
}

#[test]
fn unlabelled_data_of_a_direct_nest_is_not_visible_from_the_module() {
    // §23.9: the block is a scope, so a bare `x` outside it is undeclared.
    let src = r#"module top;
  if (1) if (1) begin int x = 3; end
  initial #1 $display("%0d", x);
endmodule
"#;
    let err = simulate(src, 1000)
        .err()
        .expect("a bare read of block data is undeclared");
    assert!(err.contains("Undeclared identifier 'x'"), "{err}");
}

#[test]
fn an_implicit_name_is_zero_padded_beside_an_explicit_genblk() {
    // The example of §27.6, with a process per block in place of its data.
    let got = lines(
        r#"module top;
  parameter genblk2 = 0;
  genvar i;
  if (genblk2) initial $display("no"); else initial $display("%m");
  if (genblk2) initial $display("no"); else initial #1 $display("%m");
  for (i = 0; i < 1; i = i + 1) begin : g1
    if (1) initial #2 $display("%m");
  end
  for (i = 0; i < 1; i = i + 1)
    if (1) initial #3 $display("%m");
  if (1) initial #4 $display("%m");
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "top.genblk1",
            "top.genblk02",
            "top.g1[0].genblk1",
            "top.genblk4[0].genblk1",
            "top.genblk5",
        ]
    );
}

#[test]
fn a_block_label_pads_the_implicit_name_of_a_later_construct() {
    let got = lines(
        r#"module top;
  if (1) begin : genblk2
    function automatic int f(); return 1; endfunction
    initial $display("lab %0d", f());
  end
  if (1) begin
    function automatic int f(); return 2; endfunction
    initial $display("unl %0d", f());
  end
endmodule
"#,
    );
    assert_eq!(got, ["lab 1", "unl 2"]);
}

#[test]
fn a_block_label_pads_the_same_way_in_an_instance() {
    let got = lines(
        r#"module c;
  if (1) begin : genblk2
    function automatic int f(); return 1; endfunction
    initial $display("%m %0d", f());
  end
  if (1) begin
    function automatic int f(); return 2; endfunction
    initial $display("%m %0d", f());
  end
endmodule
module top; c u(); endmodule
"#,
    );
    assert_eq!(got, ["top.u.genblk2 1", "top.u.genblk02 2"]);
}

#[test]
fn a_label_in_a_dead_arm_still_conflicts() {
    assert_eq!(m_of("if (0) begin : genblk2 end if (1) @"), "top.genblk02");
}

#[test]
fn a_label_in_a_directly_nested_dead_arm_still_conflicts() {
    assert_eq!(
        m_of("if (1) if (0) begin : genblk2 end if (1) @"),
        "top.genblk02"
    );
}

#[test]
fn zeros_are_added_until_the_name_is_free() {
    assert_eq!(
        m_of("parameter genblk1 = 0, genblk01 = 0, genblk001 = 0; if (1) @"),
        "top.genblk0001"
    );
}

#[test]
fn a_name_that_differs_by_zeros_only_does_not_conflict() {
    assert_eq!(m_of("parameter genblk01 = 0; if (1) @"), "top.genblk1");
}

#[test]
fn a_name_declared_after_the_construct_still_conflicts() {
    assert_eq!(m_of("if (1) @ parameter genblk1 = 0;"), "top.genblk01");
}

#[test]
fn a_directly_nested_block_takes_the_padded_name() {
    assert_eq!(
        m_of("parameter genblk1 = 0; if (1) if (1) @"),
        "top.genblk01"
    );
    assert_eq!(
        m_of("parameter genblk1 = 0; if (0) ; else case (1) 1: @ endcase"),
        "top.genblk01"
    );
}

#[test]
fn an_unnamed_loop_is_padded_with_its_index_after_the_zeros() {
    assert_eq!(
        m_of("parameter genblk1 = 0; for (genvar i = 0; i < 1; i++) @"),
        "top.genblk01[0]"
    );
}

#[test]
fn every_kind_of_declaration_conflicts() {
    for decl in [
        "wire genblk1;",
        "int genblk1;",
        "localparam genblk1 = 0;",
        "parameter type genblk1 = int;",
        "genvar genblk1;",
        "typedef int genblk1;",
        "typedef enum {genblk1} e_t;",
        "enum {genblk1} e;",
        "function void genblk1(); endfunction",
        "task genblk1(); endtask",
        "class genblk1; endclass",
        "wire a, b; and genblk1(a, a, b);",
        "initial begin : genblk1 end",
        "let genblk1 = 1;",
        "for (genvar i = 0; i < 0; i++) begin : genblk1 end",
    ] {
        // `@` is the next construct after any construct in `decl`.
        let want = if decl.starts_with("for") {
            "top.genblk2"
        } else {
            "top.genblk01"
        };
        assert_eq!(m_of(&format!("{decl} if (1) @")), want, "{decl}");
    }
}

#[test]
fn an_instance_name_and_a_port_conflict() {
    let got = lines(
        r#"module leaf; endmodule
module p(input int genblk1); if (1) initial $display("%m"); endmodule
module q; leaf genblk1(); if (1) initial $display("%m"); endmodule
module r #(parameter genblk1 = 0); if (1) initial $display("%m"); endmodule
module top; int a = 0; p up(a); q uq(); r ur(); endmodule
"#,
    );
    assert_eq!(got.len(), 3, "{got:?}");
    for want in ["top.up.genblk01", "top.uq.genblk01", "top.ur.genblk01"] {
        assert!(got.iter().any(|l| l == want), "{want}: {got:?}");
    }
}

#[test]
fn only_the_names_of_the_same_scope_conflict() {
    assert_eq!(
        m_of("parameter genblk1 = 0; if (1) begin : a if (1) @ end"),
        "top.a.genblk1"
    );
    assert_eq!(
        m_of("if (1) begin : a int genblk2; end if (1) @"),
        "top.genblk2"
    );
}

/// §27.6: an unnamed block has no name usable in a hierarchical name, so
/// `genblk01.f` in source is a tool extension. Pinned to keep the name in step.
#[test]
fn a_padded_block_is_named_from_inside_by_its_padded_name() {
    let got = lines(
        r#"module top;
  parameter genblk1 = 0;
  if (1) begin
    function automatic int f(); return 7; endfunction
    initial #1 $display("%0d", genblk01.f());
  end
endmodule
"#,
    );
    assert_eq!(got, ["7"]);
}

/// Audit forms that held on the base and must keep holding.
#[test]
fn unnamed_blocks_without_a_nest_keep_their_numbers() {
    let got = lines(
        r#"module top; int r[2];
  for(genvar i=0;i<2;i++) begin function automatic int f(); return 10+i; endfunction initial r[i]=f(); end
  initial begin #1; $display("%0d %0d",r[0],r[1]); end
endmodule
"#,
    );
    assert_eq!(got, ["10 11"]);
    let got = lines(
        r#"module m; int r1,r2;
 if(1) begin function automatic int f(); return 1; endfunction initial r1=f(); end
 if(1) begin function automatic int f(); return 2; endfunction initial r2=f(); end endmodule
module top; m u();
  initial begin #1; $display("%0d %0d",u.r1,u.r2); end
endmodule
"#,
    );
    assert_eq!(got, ["1 2"]);
    let got = lines(
        r#"module top;
  if(1) begin function automatic void f(); $display("%m"); endfunction initial f(); end
  if(1) begin function automatic void f(); $display("%m"); endfunction initial #1 f(); end
endmodule
"#,
    );
    assert_eq!(got, ["top.genblk1.f", "top.genblk2.f"]);
}

#[test]
fn a_direct_nest_names_its_block_after_the_inner_label() {
    let got = lines(
        r#"module top; int a; if(1) if(1) begin:x function automatic int f(); return 5; endfunction initial a=f(); end
initial begin #1; $display("%0d %0d",a,x.f()); end endmodule
"#,
    );
    assert_eq!(got, ["5 5"]);
}

/// §27.5: `d` and `e` are scopes of `top`. `hit` ends at 2 whether or not the
/// `disable` reaches `d`'s task, so this does not pin the `disable`.
#[test]
fn a_directly_nested_block_and_its_sibling_are_both_named_from_the_module() {
    let got = lines(
        r#"module top;
  int hit = 0;
  if (1) if (1) begin : d
    task automatic t(); #10 hit = 1; endtask
    function automatic int f(); return 1; endfunction
    initial $display("%m");
    initial begin t(); end
  end
  if (1) begin : e
    task automatic t(); #10 hit = 2; endtask
    function automatic int f(); return 2; endfunction
    initial begin t(); end
  end
  initial begin #1 $display("%0d %0d", d.f(), e.f()); disable d.t; #20 $display("hit=%0d", hit); end
endmodule
"#,
    );
    assert_eq!(got, ["top.d", "1 2", "hit=2"]);
}

/// §21.2.1.5: `%m` of a task is instance, block path, task, across the forms
/// of one module.
#[test]
fn a_task_names_its_full_path_in_each_form_of_block() {
    let got = lines(
        r#"module dut;
  if (1) begin : a task t; $display("%m"); endtask initial #1 t; end
  if (1) if (1) begin : d task t; $display("%m"); endtask initial #2 t; end
  if (1) begin task t; $display("%m"); endtask initial #3 t; end
  if (1) begin if (1) begin : n task t; $display("%m"); endtask initial #4 t; end end
  for (genvar i = 0; i < 1; i++) begin : g
     for (genvar j = 0; j < 1; j++) begin task t; $display("%m"); endtask initial #5 t; end
  end
  case (1) 1: begin : cc task t; $display("%m"); endtask initial #6 t; end default: begin : dd end endcase
endmodule
module top; dut u(); endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "top.u.a.t",
            "top.u.d.t",
            "top.u.genblk3.t",
            "top.u.genblk4.n.t",
            "top.u.g[0].genblk1[0].t",
            "top.u.cc.t",
        ]
    );
}

/// §17.2, §27.6: a checker's blocks are scopes in each of its instances.
#[test]
fn a_top_level_checker_names_its_blocks_in_each_instance() {
    let got = lines(
        r#"checker chk(input logic c);
  if (1) begin function int f(); return 11; endfunction initial $display("%m %0d", f()); end
  if (1) begin task t(); $display("%m t"); endtask initial t(); end
  if (0) ; else begin logic [7:0] v = 8'd42; initial $display("%m v=%0d", v); end
endchecker
module top;
  logic clk = 0;
  chk k(clk);
  chk k2(clk);
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "top.k.genblk1 11",
            "top.k.genblk2.t t",
            "top.k.genblk3 v=42",
            "top.k2.genblk1 11",
            "top.k2.genblk2.t t",
            "top.k2.genblk3 v=42",
        ]
    );
}

/// §27.5, §27.6: two unnamed blocks of a checker declare two functions, and a
/// direct nest adds no scope.
#[test]
fn a_top_level_checkers_blocks_keep_their_subroutines_apart() {
    let got = lines(
        r#"checker chk(input logic c);
  if (1) begin function int f(); return 1; endfunction initial $display("a %0d", f()); end
  if (1) begin function int f(); return 2; endfunction initial $display("b %0d", f()); end
  if (1) if (1) begin : d function int f(); return 3; endfunction initial $display("%m %0d", f()); end
endchecker
module top;
  logic clk = 0;
  chk k(clk);
endmodule
"#,
    );
    assert_eq!(got, ["a 1", "b 2", "top.k.d 3"]);
}

/// Stdout lines of the xezim binary run in a fresh directory holding `files`.
fn cli(tag: &str, files: &[(&str, &str)], runs: &[&[&str]]) -> Vec<String> {
    let d = std::env::temp_dir().join(format!("xezim_scope_tree_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).unwrap();
    for (name, text) in files {
        std::fs::write(d.join(name), text).unwrap();
    }
    let mut got = Vec::new();
    for args in runs {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .arg("--no-cache")
            .args(*args)
            .current_dir(&d)
            .output()
            .expect("run xezim");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{args:?}: {text}");
        got.extend(
            text.lines()
                .filter(|l| l.starts_with("top."))
                .map(str::to_string),
        );
    }
    let _ = std::fs::remove_dir_all(&d);
    got
}

#[test]
fn a_library_module_names_its_blocks_like_any_other() {
    let got = cli(
        "lib",
        &[
            (
                "lib/lc.sv",
                r#"module lc;
  parameter genblk2 = 0;
  if (1) if (1) begin : d initial $display("%m"); end
  if (1) initial $display("%m");
endmodule
"#,
            ),
            ("t.sv", "module top; lc u(); endmodule\n"),
        ],
        &[&["-y", "lib", "+libext+.sv", "t.sv"]],
    );
    assert_eq!(got, ["top.u.d", "top.u.genblk02"]);
}

#[test]
fn a_compiled_artifact_keeps_the_names() {
    let got = cli(
        "artifact",
        &[(
            "t.sv",
            r#"module c; parameter genblk1 = 0; if (1) if (1) initial $display("%m"); endmodule
module top; if (1) if (1) begin : d initial $display("%m"); end c u(); endmodule
"#,
        )],
        &[&["--compile", "-o", "a.xez", "t.sv"], &["a.xez"]],
    );
    assert_eq!(got.len(), 2, "{got:?}");
    for want in ["top.d", "top.u.genblk01"] {
        assert!(got.iter().any(|l| l == want), "{want}: {got:?}");
    }
}
