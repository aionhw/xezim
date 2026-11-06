//! §23.9 / §6.21: a subroutine local is a variable of ITS declaration's type;
//! another subroutine's same-named local has nothing to do with it. The
//! interpreter registers a local's packed geometry (`packed_full_dims`,
//! `packed_signal_elem_widths`, `packed_struct_fields`) under the bare name
//! and put it back only when the name had metadata before the call: a FRESH
//! name's registration outlived the call. A function whose plain
//! `logic [63:0] v` ran COMPILED (its declaration never reaches the
//! interpreter) then handed `v[col*8 +: 8] = …` / `v[col] = …` to the
//! interpreter with the stale `u8_t [0:7] v` geometry, and the write landed
//! as a one-byte element select (`00000000000000c1` for `c7c6c5c4c3c2c1c0`,
//! #916, reported by Ganesh-TNL). Two fixes: a fresh local's geometry is
//! removed when its activation ends, and a fallback statement that carries a
//! register-bound local sets the bare name's geometry aside for its duration
//! (which also covers a trigger that is still LIVE while the victim runs).
//! Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("design must run");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn expect(src: &str, max_time: u64, want: &[&str]) {
    let got = lines(src, max_time);
    let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want, "\n got: {got:#?}\nwant: {want:#?}");
}

/// The issue's shape: victims compiled by two pre-poison calls, one call to
/// each trigger (function, task, class method, three packed dimensions), the
/// victims again.
#[test]
fn packed_of_packed_local_does_not_reshape_other_subroutines_same_named_local() {
    let src = r#"
module tb;
  typedef logic [7:0] u8_t;
  class Pat;
    function u8_t [0:7] mk_pat(input logic [7:0] base);
      u8_t [0:7] v;
      int i;
      for (i = 0; i < 8; i++) v[i] = base + i[7:0];
      return v;
    endfunction
  endclass
  class Util;
    function logic [63:0] mk_row(input logic [7:0] tag, input int row);
      logic [63:0] v;
      int col;
      for (col = 0; col < 8; col++) v[col*8 +: 8] = tag + row * 8 + col;
      return v;
    endfunction
  endclass
  function automatic u8_t [0:7] trig_pat(input logic [7:0] base);
    u8_t [0:7] v;
    int i;
    for (i = 0; i < 8; i++) v[i] = base + i[7:0];
    return v;
  endfunction
  u8_t [0:7] p_task;
  task automatic trig_task(input logic [7:0] base);
    u8_t [0:7] v;
    int i;
    for (i = 0; i < 8; i++) v[i] = base + i[7:0];
    p_task = v;
  endtask
  function automatic int trig_fill2d(input logic [7:0] base);
    u8_t [0:3][0:3] v;
    int i, j;
    for (i = 0; i < 4; i++)
      for (j = 0; j < 4; j++) v[i][j] = base + i[3:0] + j[3:0];
    return int'(v[3][3]);
  endfunction
  function automatic logic [63:0] victim_row(input logic [7:0] tag, input int row);
    logic [63:0] v;
    int col;
    for (col = 0; col < 8; col++) v[col*8 +: 8] = tag + row * 8 + col;
    return v;
  endfunction
  function automatic logic [63:0] victim_pair(input logic [7:0] tag, input int row);
    logic [63:0] v;
    int col;
    for (col = 0; col < 4; col++) begin
      v[col*8     +: 8] = tag + row * 8 + col;
      v[(col+4)*8 +: 8] = tag + row * 8 + col + 4;
    end
    return v;
  endfunction
  function automatic logic [63:0] victim_bits(input logic [7:0] tag);
    logic [63:0] v;
    int col;
    for (col = 0; col < 64; col++) v[col] = (col + tag) % 2;
    return v;
  endfunction
  Pat  pt = new();
  Util ut = new();
  u8_t [0:7] pp;
  int warm;
  initial begin
    for (warm = 0; warm < 2; warm++)
      $display("T|A%0d %h %h %h %h", warm, victim_row(8'hC0, 0), victim_pair(8'hC0, 0),
               victim_bits(8'h00), ut.mk_row(8'hC0, 0));
    pp = trig_pat(8'h10);
    trig_task(8'h10);
    $display("T|B %h %h %h %0d", pp, p_task, pt.mk_pat(8'h10), trig_fill2d(8'h10));
    $display("T|C %h %h %h %h %h", victim_row(8'hC0, 0), victim_row(8'hC0, 1),
             victim_pair(8'hC0, 0), victim_bits(8'h00), ut.mk_row(8'hC0, 0));
    $finish;
  end
endmodule
"#;
    expect(
        src,
        10,
        &[
            "T|A0 c7c6c5c4c3c2c1c0 c7c6c5c4c3c2c1c0 aaaaaaaaaaaaaaaa c7c6c5c4c3c2c1c0",
            "T|A1 c7c6c5c4c3c2c1c0 c7c6c5c4c3c2c1c0 aaaaaaaaaaaaaaaa c7c6c5c4c3c2c1c0",
            "T|B 1011121314151617 1011121314151617 1011121314151617 22",
            "T|C c7c6c5c4c3c2c1c0 cfcecdcccbcac9c8 c7c6c5c4c3c2c1c0 aaaaaaaaaaaaaaaa c7c6c5c4c3c2c1c0",
        ],
    );
}

/// Neighbours: an UNPACKED-array local, a packed-struct local and a queue
/// local named `v`; a task victim; a victim whose inner block declares a
/// second `v`; a victim first called only AFTER the poison; and a trigger
/// still LIVE while it calls the victim (its own `v` geometry must not reach
/// the callee).
#[test]
fn same_named_locals_of_other_shapes_and_a_live_caller() {
    let src = r#"
module tb;
  typedef logic [7:0] u8_t;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } nib_t;
  logic [63:0] r;
  function automatic u8_t [0:7] trig_pat(input logic [7:0] base);
    u8_t [0:7] v; int i;
    for (i = 0; i < 8; i++) v[i] = base + i[7:0];
    return v;
  endfunction
  task automatic trig_unp(input logic [7:0] base);
    logic [7:0] v [0:7]; int i;
    for (i = 0; i < 8; i++) v[i] = base + i[7:0];
    r = {v[7], v[6], v[5], v[4], v[3], v[2], v[1], v[0]};
  endtask
  function automatic int trig_struct(input logic [7:0] base);
    nib_t v; v = base; return v.hi;
  endfunction
  function automatic int trig_queue(input int n);
    int v[$]; for (int i = 0; i < n; i++) v.push_back(i); return v.size();
  endfunction
  function automatic logic [63:0] victim_row(input logic [7:0] tag, input int row);
    logic [63:0] v; int col;
    for (col = 0; col < 8; col++) v[col*8 +: 8] = tag + row * 8 + col;
    return v;
  endfunction
  task automatic victim_task(input logic [7:0] tag);
    logic [63:0] v; int col;
    for (col = 0; col < 8; col++) v[col*8 +: 8] = tag + col;
    r = v;
  endtask
  function automatic logic [63:0] victim_blocks(input logic [7:0] tag);
    logic [63:0] v; int col;
    begin : inner
      u8_t [0:7] v;
      for (col = 0; col < 8; col++) v[col] = 8'hEE;
    end
    for (col = 0; col < 8; col++) v[col*8 +: 8] = tag + col;
    return v;
  endfunction
  function automatic logic [63:0] victim_nested(input logic [7:0] tag);
    u8_t [0:7] v; int i;
    for (i = 0; i < 8; i++) v[i] = tag + i[7:0];
    return victim_row(tag, 1) ^ {v[7], v[6], v[5], v[4], v[3], v[2], v[1], v[0]};
  endfunction
  function automatic logic [63:0] victim_first(input logic [7:0] tag);
    logic [63:0] v; int col;
    for (col = 0; col < 8; col++) v[col*8 +: 8] = tag + col;
    return v;
  endfunction
  u8_t [0:7] pp;
  int k;
  initial begin
    for (k = 0; k < 2; k++) begin
      r = victim_row(8'hC0, 0);
      victim_task(8'hA0);
      $display("T|A%0d row=%h task=%h blocks=%h", k, victim_row(8'hC0, 0), r,
               victim_blocks(8'h10));
    end
    pp = trig_pat(8'h10);
    trig_unp(8'h20);
    $display("T|B pat=%h unp=%h st=%0d q=%0d", pp, r, trig_struct(8'hA5), trig_queue(3));
    r = victim_row(8'hC0, 0);
    victim_task(8'hA0);
    $display("T|C row=%h task=%h first=%h blocks=%h nested=%h", victim_row(8'hC0, 0), r,
             victim_first(8'h30), victim_blocks(8'h10), victim_nested(8'h40));
    for (k = 0; k < 2; k++) begin
      victim_task(8'hA0);
      $display("T|D%0d row=%h task=%h first=%h nested=%h", k, victim_row(8'hC0, 1), r,
               victim_first(8'h30), victim_nested(8'h40));
    end
    $finish;
  end
endmodule
"#;
    expect(
        src,
        10,
        &[
            "T|A0 row=c7c6c5c4c3c2c1c0 task=a7a6a5a4a3a2a1a0 blocks=1716151413121110",
            "T|A1 row=c7c6c5c4c3c2c1c0 task=a7a6a5a4a3a2a1a0 blocks=1716151413121110",
            "T|B pat=1011121314151617 unp=2726252423222120 st=10 q=3",
            "T|C row=c7c6c5c4c3c2c1c0 task=a7a6a5a4a3a2a1a0 first=3736353433323130 \
blocks=1716151413121110 nested=0808080808080808",
            "T|D0 row=cfcecdcccbcac9c8 task=a7a6a5a4a3a2a1a0 first=3736353433323130 \
nested=0808080808080808",
            "T|D1 row=cfcecdcccbcac9c8 task=a7a6a5a4a3a2a1a0 first=3736353433323130 \
nested=0808080808080808",
        ],
    );
}
