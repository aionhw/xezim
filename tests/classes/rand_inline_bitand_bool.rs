//! §11.4.5 / §18.7 — a bitwise `&` between one-bit operands inside an inline
//! constraint. `req.randomize() with { s == A & data == 32'h1234; }` is the
//! shape a UVM sequence item written against a bus driver uses. The whole
//! item was left to the evaluator, which only judges a complete assignment,
//! so the conjunction over a 32-bit member was hit once in 2**32 trials:
//! randomize() returned 0 with the members untouched, the register write the
//! sequence was building never landed, and the testbench waited forever. On
//! one-bit operands `&`/`|` are `&&`/`||`, so they now decompose and propagate
//! like the logical forms.

use xezim::simulate;

const SRC: &str = r#"
typedef enum logic [2:0] {S_INSTR = 3'd1, S_DATA = 3'd2} s_e;
class item;
  rand s_e s;
  rand logic [2:0] ctrl;
  rand logic [31:0] data;
  rand int pause_transfer, cycles_paused, word_division;
  constraint pause_transfer_dist { pause_transfer dist {0:=20, 1:=80}; }
  constraint cycles_paused_range { cycles_paused inside {[1:10]}; }
  constraint word_division_range { word_division inside {[1:100]}; }
endclass
module top;
  item it;
  int ok;
  initial begin
    it = new;
    ok = it.randomize() with { s == S_DATA & data == 32'h1234; };
    $display("two: ok=%0d s=%0d data=%h", ok, it.s, it.data);
    ok = it.randomize() with { s == S_DATA & ctrl == 3'b100 & data == 32'h0e000002; };
    $display("three: ok=%0d s=%0d ctrl=%0d data=%h", ok, it.s, it.ctrl, it.data);
    ok = it.randomize() with { s == S_INSTR | s == S_DATA; };
    $display("or: ok=%0d in=%0d", ok, (it.s == S_INSTR) || (it.s == S_DATA));
    ok = it.randomize() with { s == S_DATA & data == 32'h5; data == 32'h6; };
    $display("conflict: ok=%0d", ok);
    // A multi-bit `&` inside the conjunct is a VALUE, not a boolean: the outer
    // `&` joins two one-bit comparisons and decomposes, the inner one must not.
    ok = it.randomize() with { s == S_DATA & (data & 32'hff) == 32'h3c; };
    $display("wide: ok=%0d s=%0d low=%h", ok, it.s, it.data & 32'hff);
    $display("done");
  end
endmodule
"#;

#[test]
fn one_bit_bitwise_and_in_inline_constraint_solves() {
    let out: Vec<String> = simulate(SRC, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(
        out,
        vec![
            "two: ok=1 s=2 data=00001234",
            "three: ok=1 s=2 ctrl=4 data=0e000002",
            "or: ok=1 in=1",
            "conflict: ok=0",
            "wide: ok=1 s=2 low=0000003c",
            "done",
        ],
        "{out:?}"
    );
}
