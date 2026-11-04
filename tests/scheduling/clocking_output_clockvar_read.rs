//! IEEE 1800-2017 §14.3: reading an OUTPUT clockvar (`$display(mif.cb.addr)`
//! after `mif.cb.addr <= a`) is not allowed by the standard, but
//! Verilator answers with the driven net's value. xezim read x, so a
//! testbench printing the address it had just written showed `Addr x`.
use xezim::simulate;

#[test]
fn output_clockvar_reads_the_driven_value() {
    let src = r#"`timescale 1ns/1ns
interface mem_if(input bit clk);
  logic [4:0] addr;
  logic [7:0] data_in, data_out;
  logic write;
  clocking cb @(posedge clk);
    default input #1 output #2;
    output write, addr, data_in;
    input  data_out;
  endclocking
  modport tb (clocking cb);
endinterface
program tb_mem(mem_if.tb mif);
  initial begin
    @(mif.cb);
    mif.cb.write <= 1; mif.cb.addr <= 3; mif.cb.data_in <= 3;
    @(mif.cb);
    $display("T|Addr %0d Data_in %0d write %0d", mif.cb.addr, mif.cb.data_in, mif.cb.write);
    $finish;
  end
endprogram
module top;
  bit clk;
  always #50 clk = ~clk;
  mem_if mif(clk);
  tb_mem tb(mif);
endmodule
"#;
    let out: Vec<String> = simulate(src, 10_000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        vec!["T|Addr 3 Data_in 3 write 1".to_string()],
        "{out:?}"
    );
}
