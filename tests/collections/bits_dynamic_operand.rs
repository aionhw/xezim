//! §20.6.2: `$bits` of a dynamically sized operand is its CURRENT size —
//! element bits times the element count for a queue or dynamic array, eight
//! bits a character for a string, 0 when empty. An empty string read 1024,
//! a class-member queue sized from a method of its class read one element's
//! width, and `$bits(h.q)` sized a module queue that shares the leaf name.
//! Expected values are the reference simulator's (one reference simulator
//! rejects `$bits` of a queue reached through a handle; the other sizes it
//! like the bare member).

use xezim::simulate;

const SRC: &str = r#"
class holder;
  int q[$];
  function int qbits(); return $bits(q); endfunction
endclass
module top;
  int q[$];
  byte bq[$];
  int da[];
  string s;
  holder h = new();
  initial begin
    $display("T|empty q=%0d da=%0d s=%0d", $bits(q), $bits(da), $bits(s));
    q.push_back(1); q.push_back(2); q.push_back(3);
    bq.push_back(8'h1); bq.push_back(8'h2);
    da = new[5];
    s = "hello";
    $display("T|filled q=%0d bq=%0d da=%0d s=%0d", $bits(q), $bits(bq), $bits(da), $bits(s));
    h.q.push_back(7); h.q.push_back(8);
    $display("T|member method=%0d dotted=%0d", h.qbits(), $bits(h.q));
    q.delete();
    $display("T|deleted q=%0d", $bits(q));
  end
endmodule
"#;

#[test]
fn bits_of_dynamic_operands_is_their_current_size() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let sim = simulate(SRC, 1_000).expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        [
            "T|empty q=0 da=0 s=0",
            "T|filled q=96 bq=16 da=160 s=40",
            "T|member method=64 dotted=64",
            "T|deleted q=0",
        ]
    );
}
