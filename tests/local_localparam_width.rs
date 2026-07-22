//! §6.20.2 — a task/block-local `localparam`/`parameter` with no explicit type
//! is SELF-DETERMINED from its initializer (32-bit for the usual integer
//! constant), NOT the 1-bit that an implicit type resolves to. Previously such
//! a local localparam truncated to 1 bit and read 0/-1, so `x / QUANTUM` (with
//! `localparam QUANTUM = 24` reading 0) divided by zero and produced x —
//! surfacing to a customer as x-bits in an `8'(...)`-packed register field.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn task_local_localparam_is_full_width() {
    let src = r#"
module t;
  task automatic run();
    localparam Q = 24;          // untyped -> 32-bit, not 1-bit
    localparam SEVEN = 7;
    int bw; logic [31:0] hdr;
    bw = 1200;
    hdr = 32'd0;
    hdr[31:24] = 8'((bw / Q) - 1);   // 8'(49) = 0x31
    $display("Q=%0d S=%0d DIV=%0d HDR=%h",
             Q, SEVEN, bw / Q, hdr);
  endtask
  initial begin run(); $finish; end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "Q=24 S=7 DIV=50 HDR=31000000"),
        "task-local localparam width wrong; got {:?}",
        out
    );
}

#[test]
fn block_local_localparam_is_full_width() {
    let src = r#"
module t;
  initial begin
    begin
      localparam K = 100;
      int r;
      r = 500 / K;              // 5, not x (K must be 100 not 0)
      $display("K=%0d R=%0d", K, r);
    end
    $finish;
  end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "K=100 R=5"),
        "block-local localparam width wrong; got {:?}",
        out
    );
}

#[test]
fn local_localparam_type_inference_siblings() {
    // Sibling coverage: the untyped-localparam self-determined-width fix must
    // (a) cover function-local + `parameter` + multi-declarator + wide value,
    // (b) NOT wrongly widen a `var v` (1-bit logic per §6.8), and
    // (c) preserve real/string untyped localparams (not force them to int).
    let src = r#"
module t;
  function automatic int f();
    localparam FQ = 24;
    return 1200 / FQ;
  endfunction
  task automatic run();
    localparam A = 5, B = 300;        // multi-declarator
    localparam BIG = 32'hDEADBEEF;    // wide
    parameter  P = 99;                // `parameter` (not localparam)
    localparam RR = 2.5;              // untyped real -> stays real
    localparam SS = "yo";             // untyped string -> stays string
    var v = 24;                       // §6.8 var -> 1-bit logic -> 0
    $display("F=%0d A=%0d B=%0d BIG=%h P=%0d RR=%0.1f SS=%s V=%0d",
             f(), A, B, BIG, P, RR, SS, v);
  endtask
  initial begin run(); $finish; end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter()
            .any(|m| m == "F=50 A=5 B=300 BIG=deadbeef P=99 RR=2.5 SS=yo V=0"),
        "localparam type-inference siblings wrong; got {:?}",
        out
    );
}
