//! §6.20.2 / §10.9.1 / §10.9.2: a `localparam` or `parameter` typed as a
//! PACKED array of packed structs and initialised with an assignment
//! pattern, in an instantiated module. Each pattern item is one element,
//! the first item the left (most significant) element; `$bits` is the whole
//! array. The instantiated-body parameter path packed the pattern as ONE
//! struct, so the nested form read 0, the positional form folded both items
//! into one element (16'h4422) and `$bits` was 16 (#817). A header-parameter
//! override with the same pattern shapes packed the same way.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

#[test]
fn body_param_array_of_structs_pattern_in_instance() {
    let msgs = messages(
        "module leaf;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } s_t;
  localparam s_t [1:0] PAT     = '{'{a: 8'h33, b: 8'h44}, '{a: 8'h11, b: 8'h22}};
  localparam s_t [1:0] PAT_POS = '{16'h3344, 16'h1122};
  parameter  s_t [1:0] PRM     = '{16'h3344, 16'h1122};
  localparam s_t [1:0] CAT     = {16'h3344, 16'h1122};
  initial $display(\"PAT=%h POS=%h PRM=%h CAT=%h bits=%0d %0d %0d PAT1a=%h PAT0b=%h\",
                   PAT, PAT_POS, PRM, CAT, $bits(PAT), $bits(PAT_POS), $bits(PRM), PAT[1].a, PAT[0].b);
endmodule
module top; leaf u(); endmodule",
    );
    assert_line(
        &msgs,
        "PAT=33441122 POS=33441122 PRM=33441122 CAT=33441122 bits=32 32 32 PAT1a=33 PAT0b=22",
    );
}

#[test]
fn header_param_array_of_structs_pattern_override() {
    let msgs = messages(
        "typedef struct packed { logic [7:0] a; logic [7:0] b; } s_t;
module leaf #(parameter s_t [1:0] POS = '{16'h0001, 16'h0002},
              parameter s_t [1:0] NAMED = '{'{a: 8'h01, b: 8'h02}, '{a: 8'h03, b: 8'h04}});
  initial $display(\"%m POS=%h NAMED=%h bits=%0d %0d NAMED0b=%h\", POS, NAMED, $bits(POS), $bits(NAMED), NAMED[0].b);
endmodule
module top;
  leaf #(.POS('{16'h3344, 16'h1122}), .NAMED('{'{a: 8'h55, b: 8'h66}, '{a: 8'h77, b: 8'h88}})) u ();
  leaf #('{16'h3344, 16'h1122}, '{'{a: 8'h55, b: 8'h66}, '{a: 8'h77, b: 8'h88}}) o ();
  leaf d ();
endmodule",
    );
    assert_line(
        &msgs,
        "top.u POS=33441122 NAMED=55667788 bits=32 32 NAMED0b=88",
    );
    assert_line(
        &msgs,
        "top.o POS=33441122 NAMED=55667788 bits=32 32 NAMED0b=88",
    );
    assert_line(
        &msgs,
        "top.d POS=00010002 NAMED=01020304 bits=32 32 NAMED0b=04",
    );
}
