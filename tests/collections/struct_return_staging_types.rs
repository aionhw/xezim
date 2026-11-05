//! §13.4.1 / §7.5: a function returning an unpacked struct with a dynamic
//! member hands the member over through one staging collection
//! (`__xz_ret_struct__.<member>`) shared by every such function. It kept
//! the element type of its first use, so after a function returning a
//! struct of `byte unsigned d[]` had run, a struct of `bit [31:0] d[]`
//! returned (here into an element of a local dynamic array of structs)
//! arrived with each element cut to 8 bits. The design is reduced from the
//! conversion of a VHDL array-of-arrays input port (`vh_fci0`) the VHDL
//! lowering generates. Expected line from the reference simulator.

use xezim::simulate;

#[test]
fn struct_returns_with_different_member_element_types() {
    let src = r#"
module top;
  logic [47:0] d; logic [47:0] q; logic [7:0] e0, e5;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } vh_a_u8;
  function automatic vh_a_u8 vh_a_u8_mk(longint l, longint r, bit dn, byte unsigned t[]);
  endfunction
  typedef struct { longint l; longint r; bit dn; bit [31:0] d[]; } vh_a_pk4_8;
  function automatic vh_a_pk4_8 vh_a_pk4_8_mk(longint l, longint r, bit dn, bit [31:0] t[]);
    vh_a_pk4_8 a;
    a.d = t;
    return a;
  endfunction
  function automatic vh_a_pk4_8 vh_a_pk4_8_fill(longint l, longint r, bit dn, bit [31:0] e);
  endfunction
  typedef struct { longint l; longint r; bit dn; vh_a_pk4_8 d[]; } vh_a_vh_a_pk4_8;
  function automatic vh_a_vh_a_pk4_8 vh_a_vh_a_pk4_8_mk(longint l, longint r, bit dn, vh_a_pk4_8 t[]);
    vh_a_vh_a_pk4_8 a;
    a.d = t;
    return a;
  endfunction
  function automatic vh_a_vh_a_pk4_8 vh_a_vh_a_pk4_8_fill(longint l, longint r, bit dn, vh_a_pk4_8 e);
  endfunction
  function automatic bit [31:0] vh_pkset4_8(bit [31:0] vh_v, longint vh_off, byte unsigned vh_e);
    bit [31:0] vh_ve, vh_m;
    int vh_sh;
    vh_sh = 4 * (7 - vh_off);
    vh_ve = vh_e;
    vh_m = ~vh_m;
    vh_m = vh_m >> 28;
    return (vh_v & ~(vh_m << vh_sh)) | ((vh_ve & vh_m) << vh_sh);
  endfunction
  function automatic byte unsigned vh_l2u(logic b);
    case (b)
      1'b0: return 2;
      1'b1: return 3;
    endcase
  endfunction
  function automatic vh_a_vh_a_pk4_8 vh_fci0(logic [47:0] vh_v);
    int vh_bit;
    vh_a_pk4_8 vh_u41[];
    bit [31:0] vh_u43[];
    bit [31:0] vh_u45;
    vh_bit = 47;
    vh_u41 = new[3];
    for (int vh_p42 = 0; vh_p42 < 3; vh_p42++) begin
    vh_u43 = new[2];
    for (int vh_p44 = 0; vh_p44 < 2; vh_p44++) begin
    for (int vh_p46 = 0; vh_p46 < 8; vh_p46++) begin
      vh_u45 = vh_pkset4_8(vh_u45, vh_p46, vh_l2u(vh_v[vh_bit])); vh_bit = vh_bit - 1; end
      vh_u43[vh_p44] = vh_u45; end
      vh_u41[vh_p42] = vh_a_pk4_8_mk(0, 1, 0, vh_u43); end
    return vh_a_vh_a_pk4_8_mk(0, 2, 0, vh_u41);
  endfunction
  function automatic vh_a_pk4_8 vh_dflt39();
  endfunction
  function automatic vh_a_vh_a_pk4_8 vh_dflt41();
  endfunction
  function automatic void vh_elab();
    byte unsigned vh_agg3_t[];
    longint vh_agg3_l;
    longint vh_agg3_r;
    bit vh_agg3_dn;
    vh_a_u8 vh_agg3;
    vh_agg3 = vh_a_u8_mk(vh_agg3_l, vh_agg3_r, vh_agg3_dn, vh_agg3_t);
  endfunction
  initial begin : vh_kernel
    vh_elab();
  end
  initial begin vh_a_vh_a_pk4_8 x; #1; x = vh_fci0(48'h010203040506); $display("T|%h %h %h", x.d[0].d[0], x.d[0].d[1], x.d[2].d[1]); end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(got, vec!["T|22222223 22222232 22222332"]);
}
