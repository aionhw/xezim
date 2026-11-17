//! IEEE 1800-2023 §§11.4.13 and 11.4.5: nonintegral membership uses `==`,
//! converting an integral operand to real. §6.12 covers shortreal, realtime,
//! signed zero and IEEE 754 NaN; §20.5 supplies their bit conversions.
//! Collections contribute their elements (§§7.4, 7.5, 7.8, 7.10, 11.4.13).
//! Parameters and object members follow §§6.20, 8.5 and 8.6.
//! Case-inside uses the same rule (§12.5.4); timed arms exercise its interpreter.

use xezim::simulate;

const FORMS: &str = r#"
// IEEE 1800-2023 §§6.12, 11.4.5, 11.4.13, 20.5.
package values;
  parameter real P = 2.5;
  parameter int IP = 1;
  real item;
endpackage
class holder;
  localparam real ITEMS[2] = '{0.0, 1.0};
  real item;
  function bit contains(int i);
    return i inside {item};
  endfunction
endclass
module top;
  real r, other, nan_value, inf_value, minus_zero;
  int i;
  int unsigned u;
  longint unsigned encoding;
  shortreal sr;
  realtime rt;
  real fixed_items[2], dynamic_items[], queue_items[$], assoc_items[int];
  holder h;
  logic [63:0] wildcard_item;
  logic [3:0] partial_x, partial_z;
  logic continuous_variable, continuous_constant;
  parameter real P = 2.5;
  localparam real LP = 2.5;
  parameter int IP = 1;
  localparam int ILP = 1;
  assign continuous_variable = r inside {i};
  assign continuous_constant = r inside {1};
  initial begin
    r=1.0; other=1.0; i=1; u=32'hffffffff;
    sr=1.0; rt=1.0;
    nan_value=$bitstoreal(64'h7ff8000000000000);
    inf_value=$bitstoreal(64'h7ff0000000000000);
    minus_zero=$bitstoreal(64'h8000000000000000);
    wildcard_item='x; partial_x=4'bx001; partial_z=4'bz001;
    #1;
    $display("scalar %b %b %b %b %b", r inside {i}, i inside {r}, r inside {other}, sr inside {i}, rt inside {i});
    $display("continuous %b %b", continuous_variable, continuous_constant);
    $display("special %b %b %b %b %b", nan_value inside {nan_value}, nan_value==nan_value, inf_value inside {inf_value}, 0.0 inside {minus_zero}, minus_zero inside {0.0});
    $display("ranges %b %b %b %b %b", r inside {[other:other]}, r inside {[$:other]}, r inside {[other:$]}, r inside {[2.0:1.0]}, nan_value inside {[other:other]});
    $display("wildcard %b %b %b", 4'b1011 inside {4'b10xx}, 4'b1011 inside {4'b10zz}, i inside {wildcard_item});
    $display("mixed_xz %b %b %b %b", r inside {partial_x}, r inside {partial_z}, 0.0 inside {partial_x}, 0.0 inside {partial_z});
    i=-1; r=-1.0;
    $display("negative %b %b", r inside {i}, i inside {r});
    r=1.0; encoding=64'h3ff0000000000000;
    $display("encoding %b %b", r inside {encoding}, encoding inside {r});
    r=4294967295.0;
    $display("unsigned %b %b %b", r inside {u}, u inside {r}, r inside {[u:u]});
    r=2.5; other=2.5; fixed_items[0]=0.0; fixed_items[1]=2.5;
    dynamic_items=new[2]; dynamic_items[0]=0.0; dynamic_items[1]=2.5;
    queue_items.push_back(0.0); queue_items.push_back(2.5);
    assoc_items[7]=2.5;
    $display("collections %b %b %b %b", r inside {fixed_items}, r inside {dynamic_items}, r inside {queue_items}, r inside {assoc_items});
    i=1; fixed_items[0]=1.0; dynamic_items[0]=1.0; queue_items[0]=1.0; assoc_items[0]=1.0;
    $display("mixed_collections %b %b %b %b", i inside {fixed_items}, i inside {dynamic_items}, i inside {queue_items}, i inside {assoc_items});
    $display("class_constant %b", i inside {holder::ITEMS});
    $display("elements %b %b %b %b %b", r inside {fixed_items[1]}, r inside {dynamic_items[1]}, r inside {queue_items[1]}, r inside {assoc_items[7]}, 0.0 inside {assoc_items[8]});
    fixed_items[1]=nan_value; dynamic_items[1]=nan_value; queue_items[1]=nan_value; assoc_items[7]=nan_value;
    $display("nan_collections %b %b %b %b", nan_value inside {fixed_items}, nan_value inside {dynamic_items}, nan_value inside {queue_items}, nan_value inside {assoc_items});
    values::item=2.5;
    h=new; h.item=2.5;
    $display("names %b %b %b %b %b", r inside {P}, r inside {LP}, r inside {values::P}, r inside {values::item}, r inside {h.item});
    h.item=1.0;
    $display("class_method %b", h.contains(i));
    r=1.0;
    $display("int_names %b %b %b", r inside {IP}, r inside {ILP}, r inside {values::IP});
    $display("casts %b %b %b %b", r inside {$signed(1)}, r inside {$unsigned(1)}, $signed(1) inside {r}, $unsigned(1) inside {r});
    $display("literal %b %b %b", r inside {1}, r inside {1.0}, 1 inside {r});
    $finish;
  end
endmodule
"#;

const INTERPRETED_CASE: &str = r#"
// IEEE 1800-2023 §§11.4.13 and 12.5.4.
module top;
  real r, n, items[1];
  int i, scalar_hit, array_hit, nan_hit, range_hit;
  initial begin
    r=1.0; i=1; items[0]=1.0; n=$bitstoreal(64'h7ff8000000000000);
    case(r) inside i: begin #1; scalar_hit=1; end default: scalar_hit=0; endcase
    case(i) inside items: begin #1; array_hit=1; end default: array_hit=0; endcase
    case(n) inside items: begin #1; nan_hit=1; end n: nan_hit=2; default: nan_hit=0; endcase
    case(r) inside [r:r]: begin #1; range_hit=1; end default: range_hit=0; endcase
    $display("case %0d %0d %0d %0d", scalar_hit, array_hit, nan_hit, range_hit);
    $finish;
  end
endmodule
"#;

#[test]
fn real_membership_uses_numeric_equality() {
    let sim = simulate(FORMS, 10_000).expect("simulate real membership");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert_eq!(
        lines,
        vec![
            "scalar 1 1 1 1 1",
            "continuous 1 1",
            "special 0 0 1 1 1",
            "ranges 1 1 1 0 0",
            "wildcard 1 1 1",
            "mixed_xz 1 1 0 0",
            "negative 1 1",
            "encoding 0 0",
            "unsigned 1 1 1",
            "collections 1 1 1 1",
            "mixed_collections 1 1 1 1",
            "class_constant 1",
            "elements 1 1 1 1 1",
            "nan_collections 0 0 0 0",
            "names 1 1 1 1 1",
            "class_method 1",
            "int_names 1 1 1",
            "casts 1 1 1 1",
            "literal 1 1 1",
        ]
    );
}

#[test]
fn interpreted_case_inside_uses_numeric_equality() {
    let sim = simulate(INTERPRETED_CASE, 10_000).expect("simulate interpreted case-inside");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert_eq!(lines, ["case 1 1 0 1"]);
}
