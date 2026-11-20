//! IEEE 1800-2023 §11.4.12.1, §10.9.1: the operands of a replication in an
//! assignment pattern are evaluated exactly once, however many elements the
//! copies fill: `fb = '{2{f()}}` calls `f` once and stores its value in
//! both elements. The blocking store called `f` once per element.

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn replicated_call_runs_once_for_every_target_shape() {
    let out = lines(
        r#"
module top;
  typedef struct { int a; int b; } S;
  int calls;
  function int f(); calls++; return calls * 10; endfunction
  int fb [2], fq [$], fd [];
  int m [2][2];
  S st [2];
  class C;
    int a [3];
    function void run(); a = '{3{f()}}; endfunction
  endclass
  C c;
  initial begin
    calls = 0; fb = '{2{f()}}; $display("T|fb calls=%0d fb=%p", calls, fb);
    calls = 0; fq = '{3{f()}}; $display("T|fq calls=%0d fq=%p", calls, fq);
    calls = 0; fd = '{2{f(), 1}}; $display("T|fd calls=%0d fd=%p", calls, fd);
    calls = 0; m = '{2{'{f(), 1}}}; $display("T|m calls=%0d m=%p", calls, m);
    calls = 0; m = '{'{2{f()}}, '{2{f()}}}; $display("T|m2 calls=%0d m=%p", calls, m);
    calls = 0; st = '{2{'{f(), 2}}}; $display("T|st calls=%0d st=%p", calls, st);
    c = new; calls = 0; c.run(); $display("T|cls calls=%0d a=%p", calls, c.a);
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|fb calls=1 fb='{10, 10}",
            "T|fq calls=1 fq='{10, 10, 10}",
            "T|fd calls=1 fd='{10, 1, 10, 1}",
            "T|m calls=1 m='{'{10, 1}, '{10, 1}}",
            "T|m2 calls=2 m='{'{10, 10}, '{20, 20}}",
            "T|st calls=1 st='{'{a:10, b:2}, '{a:10, b:2}}",
            "T|cls calls=1 a='{10, 10, 10}",
        ]
    );
}
