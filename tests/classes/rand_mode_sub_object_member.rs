//! §18.8, §18.9: `p.h.len.rand_mode(0)` and `p.h.v_c.constraint_mode(0)`
//! switch off the random variable `len` and the constraint `v_c` of the
//! object `p.h`: its own randomize() and the randomize() of `p`, which solves
//! it as a rand sub-object, both hold `len` at its value (and size an array
//! from it), and the query forms read the modes back. The mode used to be
//! recorded on `p` under the name `h.len`, which nothing read (#1259).

use xezim::simulate;

#[test]
fn member_modes_through_a_sub_object_handle() {
    let src = r#"
class hdr;
  rand bit [15:0] len;
  rand bit [3:0] v;
  constraint len_c { len inside {[20:576]}; }
  constraint v_c { v == 3; }
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  constraint pad_c { pad.size() == 46 - h.len; }
  function new(); h = new(); endfunction
endclass
class plain;
  rand bit [7:0] x;
endclass
module top;
  initial begin
    pkt p = new();
    hdr d = new();
    int other = 0, ok = 0, bad = 0;
    p.h.len.rand_mode(0);
    p.h.len = 44;
    $display("T|query len=%0d v=%0d", p.h.len.rand_mode(), p.h.v.rand_mode());
    void'(p.h.randomize());
    $display("T|direct len=%0d", p.h.len);
    repeat (5) begin
      if (p.randomize()) begin
        ok++;
        if (p.h.len != 44 || p.pad.size() != 2 || p.h.v != 3) bad++;
      end
    end
    $display("T|nested ok=%0d bad=%0d", ok, bad);
    p.h.len.rand_mode(1);
    p.h.v_c.constraint_mode(0);
    $display("T|cquery v_c=%0d len_c=%0d", p.h.v_c.constraint_mode(), p.h.len_c.constraint_mode());
    repeat (20) begin
      void'(p.randomize());
      if (p.h.v != 3) other++;
    end
    $display("T|v_c off other=%0d len_ok=%0d", other > 0, p.h.len >= 20 && p.h.len <= 46);
    d.len.rand_mode(0); d.len = 7;
    void'(d.randomize());
    $display("T|plain len=%0d", d.len);
    $finish;
  end
endmodule
"#;
    let lines: Vec<String> = simulate(src, 100_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|query len=0 v=1",
            "T|direct len=44",
            "T|nested ok=5 bad=0",
            "T|cquery v_c=0 len_c=1",
            "T|v_c off other=1 len_ok=1",
            "T|plain len=7",
        ]
    );
}
