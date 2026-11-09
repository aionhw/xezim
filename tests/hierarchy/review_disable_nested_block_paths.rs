//! §9.6.2: a `disable` whose target is a block of another process stops that
//! block and lets the disabling process carry on, also from inside a nested
//! `begin ... end` of its own; a path (`lab.sub`, `l[1].lab.sub`) names the
//! block's process by its head. Every expected line is the reference
//! simulator's.
use xezim::simulate;

/// The `T|` lines, sorted.
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

#[test]
fn a_disable_inside_a_nested_block_keeps_the_rest_of_that_block() {
    let src = r#"
module top;
  initial begin : lab
    begin : sub
      #30 $display("T|sub done");
    end
    $display("T|%0t lab continues", $time);
  end
  initial #5 begin disable lab.sub; $display("T|%0t disabled lab.sub", $time); end
  initial begin #6; begin disable lab.sub; $display("T|%0t inactive lab.sub", $time); end end
  if (1) begin : g
    initial begin : lab
      begin : sub
        #30 $display("T|g sub done");
      end
      $display("T|%0t g lab continues", $time);
    end
    initial #6 begin disable lab.sub; $display("T|%0t g disabled lab.sub", $time); end
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|5 disabled lab.sub",
            "T|5 lab continues",
            "T|6 g disabled lab.sub",
            "T|6 g lab continues",
            "T|6 inactive lab.sub",
        ]
    );
}

#[test]
fn a_path_through_a_loop_iteration_disables_only_that_iterations_block() {
    let src = r#"
module top;
  for (genvar i = 0; i < 3; i++) begin : l
    initial begin : lab
      begin : sub
        #(10 + i) $display("T|%0t l[%0d] sub done", $time, i);
      end
      $display("T|%0t l[%0d] lab continues", $time, i);
    end
  end
  if (1) begin : g
    initial #3 begin
      disable l[1].lab.sub;
      $display("T|%0t disabled l[1].lab.sub", $time);
    end
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|10 l[0] lab continues",
            "T|10 l[0] sub done",
            "T|12 l[2] lab continues",
            "T|12 l[2] sub done",
            "T|3 disabled l[1].lab.sub",
            "T|3 l[1] lab continues",
        ]
    );
}

#[test]
fn a_disable_of_an_own_enclosing_block_still_unwinds_it() {
    let src = r#"
module top;
  initial begin : own
    begin : inner
      #1;
      begin
        disable own.inner;
        $display("T|not disabled");
      end
      $display("T|after nested");
    end
    $display("T|%0t own continues", $time);
  end
  task automatic tk(); begin : tb #4 $display("T|tb done"); end endtask
  initial begin : caller
    begin : cb tk(); $display("T|after tk"); end
    $display("T|%0t caller continues", $time);
  end
  initial #2 begin disable caller.cb; $display("T|%0t disabled caller.cb", $time); end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|1 own continues",
            "T|2 caller continues",
            "T|2 disabled caller.cb",
        ]
    );
}
