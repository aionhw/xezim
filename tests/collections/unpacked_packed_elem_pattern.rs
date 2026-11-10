//! IEEE 1800-2023 §10.9.1, §7.4, §6.8: an assignment pattern written to a
//! whole multi-dimensional unpacked array whose element is a
//! multi-dimensional packed type (`logic [1:0][39:0] n [2][2]`) assigns
//! every element. The array was taken for a packed vector and the pattern
//! stored as one value under the array's name, which has no storage, so
//! every element read x. Positional, keyed, `default:`, nested and
//! replicated patterns; declaration initializers, blocking and nonblocking
//! stores and `always_comb`. Expected values are from the reference
//! simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m.trim() == want),
        "missing {want:?} in {msgs:#?}"
    );
}

#[test]
fn declaration_initializers_fill_every_element() {
    let msgs = messages(
        "module t;
  logic [1:0][39:0] m  [2][2] = '{default: 80'h0};
  logic [1:0][7:0]  di [2][2] = '{default: 16'hbeef};
  logic [1:0][7:0]  pi [2][2] = '{'{16'h0102, 16'h0304}, '{16'h0506, 16'h0708}};
  initial #1 $display(\"m=%h di=%h,%h pi=%h,%h,%h,%h\", m[1][1], di[0][0], di[1][1],
                      pi[0][0], pi[0][1], pi[1][0], pi[1][1]);
endmodule",
    );
    assert_line(
        &msgs,
        "m=00000000000000000000 di=beef,beef pi=0102,0304,0506,0708",
    );
}

#[test]
fn blocking_default_positional_keyed_nested_and_replicated_patterns() {
    let msgs = messages(
        "module t;
  logic [1:0][39:0] n  [2][2];
  logic [1:0][7:0]  a2 [2][2], a6 [2][2], a7 [2][2], a8 [2][2], a9 [2][2], rp [2][2];
  initial begin
    n  = '{default: 80'h0};
    a2 = '{default: 16'h1234};
    a6 = '{0: '{default: 16'h1111}, default: 16'h2222};
    a7 = '{default: '{default: 16'h7777}};
    a8 = '{'{16'h0a0b, 16'h0c0d}, '{16'h0e0f, 16'h1011}};
    a9 = '{'{1: 16'h9999, 0: 16'h8888}, '{default: 16'h6666}};
    rp = '{2{'{2{16'habcd}}}};
    #1 $display(\"n=%h a2=%h,%h a6=%h,%h,%h a7=%h,%h\", n[1][1], a2[0][0], a2[1][1],
                a6[0][0], a6[0][1], a6[1][1], a7[0][0], a7[1][1]);
    $display(\"a8=%h,%h,%h,%h a9=%h,%h,%h rp=%h,%h\", a8[0][0], a8[0][1], a8[1][0],
             a8[1][1], a9[0][0], a9[0][1], a9[1][1], rp[0][0], rp[1][1]);
    $display(\"sel=%h\", a8[1][0][1]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "n=00000000000000000000 a2=1234,1234 a6=1111,1111,2222 a7=7777,7777",
    );
    assert_line(
        &msgs,
        "a8=0a0b,0c0d,0e0f,1011 a9=8888,9999,6666 rp=abcd,abcd",
    );
    assert_line(&msgs, "sel=0e");
}

#[test]
fn deeper_packed_and_unpacked_shapes_and_ascending_ranges() {
    let msgs = messages(
        "module t;
  logic [1:0][3:0][7:0] a4 [2][2];
  logic [1:0][7:0]  a5 [2][2][2];
  logic [0:1][7:0]  asc [0:1][0:1];
  initial begin
    a4 = '{default: 64'h0102030405060708};
    a5 = '{default: 16'h4321};
    asc = '{'{16'h0102, 16'h0304}, '{16'h0506, 16'h0708}};
    #1 $display(\"a4=%h,%h a5=%h,%h sel=%h\", a4[0][0], a4[1][1], a5[0][0][0],
                a5[1][1][1], a4[1][1][0]);
    $display(\"asc=%h,%h,%h,%h\", asc[0][0], asc[0][1], asc[1][0], asc[1][1]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "a4=0102030405060708,0102030405060708 a5=4321,4321 sel=05060708",
    );
    assert_line(&msgs, "asc=0102,0304,0506,0708");
}

#[test]
fn typedef_and_packed_struct_array_elements() {
    let msgs = messages(
        "module t;
  typedef logic [1:0][7:0] w16_t;
  typedef struct packed { logic [7:0] hi; logic [7:0] lo; } ps_t;
  w16_t      td [2][2];
  ps_t [1:0] sp [2][2];
  initial begin
    td = '{default: 16'hd00d};
    sp = '{default: 32'h01020304};
    #1 $display(\"td=%h,%h sp=%h,%h\", td[0][0], td[1][1], sp[0][0], sp[1][1]);
  end
endmodule",
    );
    assert_line(&msgs, "td=d00d,d00d sp=01020304,01020304");
}

#[test]
fn nonblocking_always_comb_and_variable_default() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [1:0][7:0] v = 16'h1357;
  logic [1:0][7:0] nb [2][2], ac [2][2], vr [2][2];
  always_comb ac = '{'{v, 16'h0001}, '{16'h0002, v}};
  always @(posedge clk) nb <= '{default: 16'hf00d};
  initial begin
    vr = '{default: v};
    #1 clk = 1;
    #1 $display(\"nb=%h,%h ac=%h,%h,%h vr=%h\", nb[0][0], nb[1][1], ac[0][0], ac[0][1],
                ac[1][1], vr[1][0]);
  end
endmodule",
    );
    assert_line(&msgs, "nb=f00d,f00d ac=1357,0001,1357 vr=1357");
}

// Guards: shapes that already stored every element.
#[test]
fn one_dimensional_packed_one_dimensional_unpacked_and_continuous_forms() {
    let msgs = messages(
        "module t;
  typedef struct packed { logic [7:0] hi; logic [7:0] lo; } ps_t;
  logic [79:0]      c1 [2][2];
  logic [1:0][39:0] c2 [2];
  ps_t              st [2][2];
  logic [1:0][7:0]  ca [2][2];
  logic [1:0][7:0]  pk;
  assign ca = '{default: 16'hcafe};
  initial begin
    c1 = '{default: 80'h5};
    c2 = '{default: 80'h6};
    st = '{default: 16'h5a5a};
    pk = '{8'h12, 8'h34};
    #1 $display(\"c1=%h c2=%h st=%h,%h ca=%h,%h pk=%h\", c1[1][1], c2[1], st[0][0], st[1][1],
                ca[0][0], ca[1][1], pk);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "c1=00000000000000000005 c2=00000000000000000006 st=5a5a,5a5a ca=cafe,cafe pk=1234",
    );
}
