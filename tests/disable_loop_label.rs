//! §9.6.2 / §12.7: `disable <label>` on a labelled loop/case/if must terminate
//! that statement and resume execution AFTER it. The parser discarded the
//! label ("no AST node hosts it"), so `disable L` found no target and unwound
//! the whole process — silently killing everything after the loop. Fixed by
//! wrapping a labelled non-block statement in a named block.

use xezim::simulate;

fn out(src: &str, tag: &str) -> String {
    let sim = simulate(src, 100).expect("sim");
    sim.output.iter().find(|o| o.message.starts_with(tag))
        .map(|o| o.message.clone()).unwrap_or_default()
}

#[test]
fn disable_labelled_loop_resumes_after() {
    let src = "module t; int s=0; initial begin\n\
        L: for (int i=0;i<5;i++) begin if (i==3) disable L; s+=i; end\n\
        $display(\"A %0d\", s);\n\
        $finish; end endmodule";
    assert_eq!(out(src, "A "), "A 3", "disable L exits loop, continues after (0+1+2)");
}

#[test]
fn disable_outer_from_nested_loop() {
    let src = "module t; int s=0; initial begin\n\
        outer: for (int i=0;i<3;i++) for (int j=0;j<3;j++) begin\n\
          if (j==2) continue; if (i==2) disable outer; s+=1; end\n\
        $display(\"B %0d\", s);\n\
        $finish; end endmodule";
    assert_eq!(out(src, "B "), "B 4", "disable outer breaks nested loops, continues after");
}
