//! #994: §11.8.2, §11.4.3, §18.5: a signed variable read in an unsigned
//! relation (`row_bytes == line_width * 8` with an `int row_bytes` and a
//! `longint unsigned line_width`) reads as its bits unsigned, and a product
//! of two variables (`cnt * pitch`) is exact; the solver modelled neither, so
//! it waited for a guess of the whole 32- or 64-bit variable and
//! `randomize()` returned 0 on these satisfiable classes. Also a wrapped
//! side (`start_addr - 8'h70`) bounded by an open one, and an `if` whose
//! branch only fails once enforced. Every check is the constraint itself;
//! the reference simulator solves all four classes 20 times out of 20 and
//! draws the mixed-sign `x` from both halves of its range.

use xezim::simulate;

#[test]
fn unsigned_reads_and_products_solve() {
    let src = r#"
class alloc_req;
  rand longint unsigned region_base, region_top, start_addr, line_width;
  rand int row_bytes, row_pad, mode_sel;
  constraint c_shape { line_width > 0; line_width <= 32768; }
  constraint c_region { region_base == 64'h100000; region_top == 64'h17fbf060; }
  constraint c_placement {
    row_bytes == line_width * 8;
    if (mode_sel == 0) {
      start_addr >= region_base;
      start_addr + row_bytes + row_pad <= region_top;
    } else {
      (start_addr - 8'h70) >= region_base + row_pad;
      (start_addr - 8'h70) <= region_top - row_bytes;
    }
  }
  function bit check();
    if (mode_sel == 0)
      return start_addr >= region_base && start_addr + row_bytes + row_pad <= region_top
             && row_bytes == line_width * 8;
    return (start_addr - 8'h70) >= region_base + row_pad && (start_addr - 8'h70) <= region_top - row_bytes
           && row_bytes == line_width * 8;
  endfunction
endclass
class byte_sizes;
  rand longint unsigned a, w;
  rand byte rb;
  rand int sel;
  constraint c { w > 0; w <= 32768; rb == w * 8; sel != 0 -> (a + 8'h70) <= 64'h17fbf060 - rb; }
  function bit check(); return rb == w * 8 && (sel == 0 || (a + 8'h70) <= 64'h17fbf060 - rb); endfunction
endclass
class grid;
  rand longint unsigned start_addr, line_width;
  rand int row_bytes;
  rand bit [31:0] cnt, last, pitch;
  constraint c {
    line_width > 0; line_width <= 32768; pitch == 128;
    last == (line_width / 16) - 1; cnt == 1 + last;
    row_bytes == ((line_width + 15) / 16) * 128;
    if (cnt > 1) {
      start_addr % 128 == 0; start_addr >= 64'h100000;
      start_addr + (cnt * pitch) - 1 <= 64'h17fbf060;
    } else {
      start_addr >= 64'h100000; start_addr + row_bytes <= 64'h17fbf060;
    }
  }
  function bit check();
    if (row_bytes != ((line_width + 15) / 16) * 128 || cnt != last + 1 || last != (line_width / 16) - 1) return 0;
    if (cnt > 1) return start_addr % 128 == 0 && start_addr >= 64'h100000 && start_addr + (cnt * pitch) - 1 <= 64'h17fbf060;
    return start_addr >= 64'h100000 && start_addr + row_bytes <= 64'h17fbf060;
  endfunction
endclass
class mixed;
  rand int x, y;
  rand shortint s;
  rand bit [15:0] u;
  constraint c { x < 32'h8000_0010; x > 32'h7fff_fff0; y + 1 <= 16'd5; y >= 0; s == u; u inside {[16'hfff0:16'hffff]}; }
  function bit check(); return x < 32'h8000_0010 && x > 32'h7fff_fff0 && y + 1 <= 16'd5 && y >= 0 && s == u && u >= 16'hfff0; endfunction
endclass
module top;
  initial begin
    alloc_req a = new; byte_sizes b = new; grid g = new; mixed m = new;
    int ok[4], good[4], neg;
    repeat (20) begin
      ok[0] += a.randomize(); good[0] += a.check();
      ok[1] += b.randomize(); good[1] += b.check();
      ok[2] += g.randomize(); good[2] += g.check();
      ok[3] += m.randomize(); good[3] += m.check(); neg += m.x < 0;
    end
    $display("ok=%0d %0d %0d %0d good=%0d %0d %0d %0d", ok[0], ok[1], ok[2], ok[3], good[0], good[1], good[2], good[3]);
    $display("mixed negative x: %0d", neg > 2 && neg < 18);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        ["ok=20 20 20 20 good=20 20 20 20", "mixed negative x: 1"]
    );
}
