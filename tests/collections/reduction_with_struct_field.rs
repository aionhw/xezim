//! §7.12.3, §7.12.1, §7.2.1: a reduction or locator `with (item.field)`
//! over packed-struct elements reads the field. The reductions bound only
//! `item`, so `item.code` read 0 for every element and the result had 32
//! bits; the locators did not find the layout of a class member's elements.
//! The result has the width of the `with` expression, so the 4-bit sum
//! wraps. Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
typedef struct packed { bit valid; bit [7:0] mask; bit [3:0] code; } beat_t;
class C;
  beat_t pool[4];
  beat_t dq[$];
  function void fill();
    foreach (pool[i]) pool[i] = '{valid: i[0], mask: 8'h10 + i, code: 4'(5 * i + 1)};
    dq = '{'{1'b1, 8'h01, 4'd9}, '{1'b0, 8'h02, 4'd9}, '{1'b1, 8'h03, 4'd2}};
  endfunction
endclass
module tb;
  beat_t mpool[4];
  initial begin
    C c = new;
    int idx[$];
    mpool[0].code = 15; mpool[1].code = 15; mpool[2].code = 1; mpool[3].code = 0;
    $display("T|module sum=%0d bits=%0d", mpool.sum() with (item.code), $bits(mpool.sum() with (item.code)));
    c.fill();
    idx = c.pool.find_index with (item.code > 5);
    $display("T|class sum=%0d or=%0d and=%0d idx=%p", c.pool.sum() with (item.code), c.pool.or() with (item.code), c.pool.and() with (item.mask), idx);
    idx = c.dq.find_index with (item.code == 9 && item.valid);
    $display("T|queue sum=%0d xor=%0d idx=%p", c.dq.sum() with (item.code), c.dq.xor() with (item.mask), idx);
  end
endmodule
"#;

#[test]
fn with_clause_reads_struct_fields() {
    let lines: Vec<String> = simulate(SRC, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|module sum=15 bits=4",
            "T|class sum=2 or=15 and=16 idx='{1, 2}",
            "T|queue sum=4 xor=0 idx='{0}",
        ]
    );
}
