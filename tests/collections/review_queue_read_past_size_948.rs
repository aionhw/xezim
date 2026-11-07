//! §7.10.1, §7.8.6, §8.4: a queue or dynamic-array element at or past the
//! current size does not exist, and a read of it gives the element type's
//! default. Follow-ups to #948: a module-scope queue of class handles read x
//! instead of null, and a row of a fixed array of queues (`fq[1][3]`) or of a
//! class member's dynamic array of dynamic arrays still returned the element
//! a shrink left behind. Expected lines are the reference simulator's.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 1_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with('R'))
        .collect()
}

#[test]
fn handles_and_rows_read_past_size() {
    assert_eq!(
        lines(SRC),
        vec!["R1 1 1 1", "R2 1 1", "R3 1", "R4 50 0 0 11 xx", "R5 0 0"]
    );
}

const SRC: &str = r#"
class K; int v; endclass
class H;
  K items[$];
  int rows[][];
  function bit past_null(int i); return items[i] == null; endfunction
endclass
module top;
  K kq[$];
  K aa[int];
  int fq[3][$];
  logic [7:0] lq[2][$];
  integer xi;
  initial begin
    H h = new; K a = new, b = new;
    kq.push_back(a); kq.push_back(b); void'(kq.pop_back());
    h.items.push_back(a); h.items.push_back(b); void'(h.items.pop_back());
    $display("R1 %0b %0b %0b", kq[1] == null, kq[5] == null, kq[0] == a);
    $display("R2 %0b %0b", h.past_null(1), h.items[1] == null);
    $display("R3 %0b", aa[7] == null);
    for (int i = 0; i < 5; i++) fq[1].push_back(40 + i);
    fq[1].delete(); fq[1].push_back(50);
    lq[0].push_back(8'h11); lq[0].push_back(8'h22); void'(lq[0].pop_back());
    $display("R4 %0d %0d %0d %h %h", fq[1][0], fq[1][3], fq[2][0], lq[0][0], lq[0][1]);
    h.rows = new[2]; h.rows[1] = '{1, 2, 3}; h.rows[1] = new[1];
    $display("R5 %0d %0d", h.rows[1][0], h.rows[1][2]);
    $finish;
  end
endmodule
"#;
