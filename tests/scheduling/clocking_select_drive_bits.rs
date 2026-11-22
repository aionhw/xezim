//! §14.16.2, §7.4.1: overlapping clocking drives of one output that mature in
//! one time step merge by the physical bits their selects cover. Of an
//! ascending vector (`logic [0:3] w`), `w[0:1]` covers the two most
//! significant bits, and bit 0 of its value is `w[1]`. Merging by the raw
//! index instead wrote the later drive's bit into the wrong bit of the earlier
//! one, so `cb.w[0:1] <= 2'b00; cb.w[1] <= 1'b1;` ended `w=1100`. The bits
//! each output ends with are the reference simulator's. Its undriven bits
//! read x there, while a select drive leaves the other bits of the output
//! as they are here, so those are 0. No output ends where it started, so
//! each wakes its observer once.

#[test]
fn drives_of_ascending_and_offset_vectors_merge_by_physical_bit() {
    let src = r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [0:3] w = 0, v = 0;
  logic [4:1] z = 0;
  logic [3:0] d = 0;
  int nw, nv, nz, nd;
  default clocking cb @(posedge clk);
    output w, v, z, d;
  endclocking
  initial begin
    @(cb);
    cb.w[0:1] <= 2'b00; cb.w[1] <= 1'b1;
    cb.v[1:3] <= 3'b001; cb.v[2] <= 1'b1;
    cb.z[2:1] <= 2'b11; cb.z[1] <= 1'b0;
    cb.d[1:0] <= 2'b00; cb.d[1] <= 1'b1;
    @(cb); @(cb);
    $display("w=%b v=%b z=%b d=%b n=%0d%0d%0d%0d", w, v, z, d, nw, nv, nz, nd);
    $finish;
  end
  always @(w) if ($time > 0) nw++;
  always @(v) if ($time > 0) nv++;
  always @(z) if ($time > 0) nz++;
  always @(d) if ($time > 0) nd++;
endmodule
"#;
    let got: Vec<String> = xezim::simulate(src, 1_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_eq!(got, ["w=0100 v=0011 z=0010 d=0010 n=1111"]);
}
