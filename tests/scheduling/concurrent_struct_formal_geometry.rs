//! Different suspended interface tasks may use the same formal name. Its
//! packed member geometry belongs to the active frame, not the last task
//! that happened to register metadata for that name. Reference-verified.
use xezim::simulate;

const SOURCE: &str = r#"
`timescale 1ns/1ps
package types;
 typedef struct {bit[3:0] rid;bit[3:0][63:0] rdata;} read_t;
 typedef struct {bit[3:0] wid;bit[3:0][63:0] wdata;} write_t;
endpackage
interface sampler;
 import types::*;
 task read_sample(output read_t req);
  #5;
  req.rid=3;
  req.rdata[0]=64'h0123456789abcdef;
  $display("READ bits=%0d data=%h",$bits(req.rdata[0]),req.rdata[0]);
 endtask
 task write_sample(output write_t req);
  #2;
  req.wid=7;
  req.wdata[0]=64'hfedcba9876543210;
  #10;
 endtask
endinterface
module top;
 import types::*;
 sampler bus();read_t r;write_t w;
 initial begin
  fork
   bus.read_sample(r);
   begin #1;bus.write_sample(w);end
  join
  $display("RESULT r=%h w=%h rid=%h wid=%h",r.rdata[0],w.wdata[0],r.rid,w.wid);
  $finish;
 end
endmodule
"#;

#[test]
fn suspended_interface_struct_formals_keep_their_packed_geometry() {
    let sim = simulate(SOURCE, 100_000).expect("simulation");
    let messages: Vec<_> = sim.output.iter().map(|m| m.message.as_str()).collect();
    assert!(
        messages.contains(&"READ bits=64 data=0123456789abcdef"),
        "{messages:?}"
    );
    assert!(
        messages.contains(&"RESULT r=0123456789abcdef w=fedcba9876543210 rid=3 wid=7"),
        "{messages:?}"
    );
}

const NESTED_SOURCE: &str = r#"
`timescale 1ns/1ps
package types;
 typedef struct {logic[0:1][8:1] data;} narrow_t;
 typedef struct {narrow_t inner;} read_t;
 typedef struct {bit[3:0][31:0] data;} write_t;
endpackage
interface sampler;
 import types::*;
 task read_sample(output read_t req);
  #5;
  req.inner.data[0]=8'ha5;
  req.inner.data[1][8:5]=4'hc;
  req.inner.data[1][4:1]=4'bxz10;
  req.inner.data[2]=8'hff;
  $display("READ bits=%0d data=%b oob=%b",$bits(req.inner.data[0]),req.inner.data,req.inner.data[2]);
 endtask
 task write_sample(output write_t req);
  #2;req.data[0]=32'hcafebabe;#10;
 endtask
endinterface
module top;
 import types::*;
 sampler bus();
 class client;
  virtual sampler vif;read_t r;write_t w;
  task run();
   fork
    vif.read_sample(r);
    begin #1;vif.write_sample(w);end
   join
   $display("RESULT r=%b w=%h",r.inner.data,w.data[0]);
  endtask
 endclass
 initial begin
  client c=new;c.vif=bus;c.run();$finish;
 end
endmodule
"#;

fn assert_nested_geometry(source: &str) {
    let sim = simulate(source, 100_000).expect("simulation");
    let messages: Vec<_> = sim.output.iter().map(|m| m.message.as_str()).collect();
    assert!(
        messages.contains(&"READ bits=8 data=101001011100xz10 oob=xxxxxxxx"),
        "{messages:?}"
    );
    assert!(
        messages.contains(&"RESULT r=101001011100xz10 w=cafebabe"),
        "{messages:?}"
    );
}

#[test]
fn nested_struct_vif_outputs_keep_ranges_part_selects_and_four_state_bits() {
    assert_nested_geometry(NESTED_SOURCE);
}

#[test]
fn same_named_members_with_different_shapes_use_the_active_formal_type() {
    let source = NESTED_SOURCE
        .replace(
            "typedef struct {narrow_t inner;} read_t;",
            "typedef narrow_t read_t;",
        )
        .replace(".inner.data", ".data");
    assert_nested_geometry(&source);
}
