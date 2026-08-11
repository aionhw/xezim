//! GitHub #86: `mem[i] = v` on a module-scope 1-D fixed unpacked array takes
//! an id-math fast path (`array_first_id`) with no per-element name
//! formatting. These pin the semantics the fast path must preserve: values
//! land, resize applies, writes interleave with reads, instance-scoped
//! arrays stay distinct, and collections (queue/assoc) are NOT intercepted.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn fill_then_read_back_matches() {
    let src = r#"
module top;
  reg [7:0] mem [0:255];
  reg [31:0] i, sum;
  initial begin
    for (i = 0; i < 256; i = i + 1) mem[i] = i[7:0];
    sum = 0;
    for (i = 0; i < 256; i = i + 1) sum = sum + mem[i];
    $display("T|sum=%0d m0=%h m255=%h", sum, mem[0], mem[255]);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|sum=32640 m0=00 m255=ff"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn resize_and_instance_scoping_hold() {
    let src = r#"
module sub;
  reg [7:0] mem [0:3];
  initial begin mem[1] = 8'hAA; #1 $display("T|sub=%h", mem[1]); end
endmodule
module top;
  reg [7:0] mem [0:3];
  sub u();
  initial begin
    mem[1] = 16'h1234;   // truncates to 8 bits
    mem[2] = 8'h56;
    #2 $display("T|top=%h %h", mem[1], mem[2]);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let got = msgs(&sim);
    assert!(got.iter().any(|m| m == "T|sub=aa"), "got {got:?}");
    assert!(got.iter().any(|m| m == "T|top=34 56"), "got {got:?}");
}

#[test]
fn queues_and_assoc_are_not_intercepted() {
    // These names sit in `module.arrays` with a fake backing range; the fast
    // path must decline so collection semantics (size, keys) stay intact.
    let src = r#"
module top;
  int q [$];
  int aa [int];
  initial begin
    q.push_back(7); q.push_back(9);
    q[0] = 5;
    aa[42] = 1; aa[42] = 2;
    $display("T|q=%0d,%0d size=%0d aa=%0d num=%0d", q[0], q[1], q.size(), aa[42], aa.num());
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|q=5,9 size=2 aa=2 num=1"),
        "got {:?}",
        msgs(&sim)
    );
}
