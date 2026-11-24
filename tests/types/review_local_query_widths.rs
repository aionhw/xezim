//! Behavioral regressions for reviewed PR follow-ups (IEEE 1800 §6.21, §11.6, §16.5, §18.12, §20.7).
fn output(src: &str) -> String {
    xezim::simulate(src, 1_000_000)
        .expect("simulation failed")
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn task_local_array_shadows_instance_array() {
    let got = output(
        r###"module child;
 int mem[7];
 task automatic run();
  int mem[16];
  logic [$clog2($size(mem))-1:0] v;
  v=15;
  $display("T|local size %0d width %0d value %0d",$size(mem),$bits(v),v);
 endtask
 initial begin run(); $finish; end
endmodule
module top; child u(); endmodule
"###,
    );
    assert!(got.contains("T|local size 16 width 4 value 15"), "{got}");
}

#[test]
fn generated_array_query_scope() {
    let got = output(
        r###"module child;
 for(genvar g=0;g<2;g++) begin: bank
  int mem[7+g];
  initial begin
   logic [$clog2($size(mem))-1:0] v;
   v=6;
   #1; $display("T|g %0d size %0d width %0d value %0d",g,$size(mem),$bits(v),v);
  end
 end
endmodule
module top; child u(); initial #2 $finish; endmodule
"###,
    );
    assert!(got.contains("T|g 0 size 7 width 3 value 6"), "{got}");
    assert!(got.contains("T|g 1 size 8 width 3 value 6"), "{got}");
}

#[test]
fn static_task_local_array_shadows_instance_array() {
    let got = output(
        r###"module child;
 int mem[7];
 task run();
  int mem[16];
  logic [$clog2($size(mem))-1:0] v;
  v=15;
  $display("T|local size %0d width %0d value %0d",$size(mem),$bits(v),v);
 endtask
 initial begin run(); $finish; end
endmodule
module top; child u(); endmodule
"###,
    );
    assert!(got.contains("T|local size 16 width 4 value 15"), "{got}");
}

#[test]
fn function_local_array_shadows_instance_array() {
    let got = output(
        r###"module child;
 int mem[7];
 function automatic void run();
  int mem[16];
  logic [$clog2($size(mem))-1:0] v;
  v=15;
  $display("T|local size %0d width %0d value %0d",$size(mem),$bits(v),v);
 endfunction
 initial begin run(); $finish; end
endmodule
module top; child u(); endmodule
"###,
    );
    assert!(got.contains("T|local size 16 width 4 value 15"), "{got}");
}

#[test]
fn static_query_ranges_follow_local_renaming() {
    let template = r#"module child;
 int mem[7];
 task run();
  int mem[16];
  begin
   logic [$clog2($size(mem))-1:0] v;
   int copies[$size(mem)];
   v=15;
   $display("T|nested width %0d value %0d copies %0d",$bits(v),v,$size(copies));
  end
 endtask
 initial begin run(); $finish; end
endmodule
module top; child u(); endmodule"#;
    for source in [
        template.to_string(),
        template.replace("task run();", "task static run();"),
        template
            .replace("task run();", "function void run();")
            .replace("endtask", "endfunction"),
    ] {
        let got = output(&source);
        assert!(got.contains("T|nested width 4 value 15 copies 16"), "{got}");
    }
}

/// §20.6.2: a declaration may use the fixed width of an earlier local.
#[test]
fn bits_of_a_local_sizes_a_later_declaration() {
    let got = output(
        r#"
module child #(parameter int W = 8);
  localparam int a = 0;
  function automatic int fill(input logic [4:0] v);
    logic [$bits(v)-1:0] b;
    b = '1;
    return int'(b);
  endfunction
  function automatic int fill_from_local();
    logic [5:0] src;
    logic [$bits(src)-1:0] val;
    val = '1;
    return int'(val);
  endfunction
  initial begin
    logic [W-1:0] a;
    logic [$bits(a)-1:0] b;
    b = '1;
    $display("T|width %0d value %0d formal %0d local %0d", $bits(b), b, fill('0), fill_from_local());
  end
endmodule
module top; child #(.W(11)) u(); endmodule
"#,
    );
    assert!(got.contains("T|width 11 value 2047 formal 31 local 63"), "{got}");
}
