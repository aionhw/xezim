//! §7.2.1, §7.4.6, §10.4.2, §11.5.1: selecting through a packed array or
//! unpacked array of packed structs with an unknown (x/z) or out-of-range
//! index must discard writes and read unknown (4-state) / zero (2-state) at
//! the member's declared width.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

#[test]
fn packed_member_xz_and_oob_index_reads_and_writes() {
    let src = r#"
module t;
  parameter logic [1:0] P_X = 2'bx1;
  parameter int P_OOB = 5;
  typedef struct packed { logic [7:0] value; logic [3:0] nib; } dw_t;
  typedef struct packed { dw_t [1:0] dw; dw_t single; } e_t;
  typedef struct packed { e_t [3:0] e; } st_t;
  typedef struct packed { bit [7:0] value; } dw2_t;
  typedef struct packed { dw2_t [1:0] dw; } e2_t;
  typedef struct packed { e2_t [3:0] e; } st2_t;
  typedef struct packed { logic [63:0] other; e_t [3:0] e; } big_st_t;

  st_t st, st_nba;
  st2_t st2;
  e_t ua [4];
  big_st_t bst;
  logic [3:0][1:0][7:0] p2;
  logic [1:0] k = 2'bx1;
  logic [1:0] xk = 2'bx1;
  logic [3:0] dyn_k;
  logic [2:0] oob = 3'd5;
  logic [7:0] out_v;
  logic [23:0] out_e;

  always_comb begin
    out_v = bst.e[dyn_k].dw[0].value;
    out_e = bst.e[dyn_k];
  end

  initial begin
    #1;
    st = '0; st.e[k] = 36'h123456789;         $display("T|e_k=%h", st);
    st = '0; st.e[k].single.value = 8'h3c;    $display("T|single=%h", st);
    st = '0; st.e[k].dw[0].value = 8'h3c;     $display("T|dw_val=%h", st);
    st = '0; st.e[P_X].dw[0].value = 8'h3c;   $display("T|param_x=%h", st);
    st = '0; st.e[oob].dw[0].value = 8'h3c;   $display("T|oob=%h", st);
    st = '0; st.e[P_OOB].dw[0].value = 8'h3c; $display("T|p_oob=%h", st);
    st = '0; st.e[k].dw[0].value[3:0] = 4'ha; $display("T|range_xk=%h", st);
    st = '0; st.e[k].dw[0].value[2] = 1'b1;   $display("T|bit_xk=%h", st);
    st = '0; st.e[1].dw[k].value = 8'h3c;     $display("T|inner_xk=%h", st);
    p2 = '0; p2[k][0] = 8'h3c;                $display("T|p2=%h", p2);

    st = {4{36'h123_456_789}};
    $display("T|reads=%h,%h,%h,%h,%h,%h",
      st.e[k].single.value,
      st.e[oob].dw[0].value,
      st.e[P_X].dw[0].value,
      st.e[k].dw[0].value[3:0],
      st.e[1].dw[k].value,
      st.e[k]);

    st2 = 64'h0011_2233_4455_6677;
    $display("T|2s_read=%h,%h", st2.e[k].dw[0].value, st2.e[oob].dw[0].value);
    st2 = '0; st2.e[k].dw[0].value = 8'h3c;
    $display("T|2s_write=%h", st2);

    for (int i = 0; i < 4; i++) ua[i] = 36'h112_233_445;
    ua[k].single.value = 8'hff;
    ua[k].dw[0].value = 8'hee;
    $display("T|ua=%h,%h,%h,%h", ua[1].single.value, ua[1].dw[0].value, ua[k].single.value, ua[k].dw[0].value);

    bst.other = 64'haaaa_bbbb_cccc_dddd;
    bst.e = {4{36'h112_233_445}};
    dyn_k = 4'd5; #1;
    $display("T|comb_oob=%h,%h", out_v, out_e);
    dyn_k = 4'b0x01; #1;
    $display("T|comb_xk=%h,%h", out_v, out_e);

    // §10.4.2 + §11.5.1: NBA schedule evaluates x index before mutation.
    st_nba = '0;
    st_nba.e[xk].dw[0].value <= 8'h3c;
    xk = 2'b00;
    #1;
    $display("T|nba_xk=%h", st_nba);
  end
endmodule
"#;
    let out: Vec<String> = lines(src)
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        [
            "T|e_k=000000000000000000000000000000000000",
            "T|single=000000000000000000000000000000000000",
            "T|dw_val=000000000000000000000000000000000000",
            "T|param_x=000000000000000000000000000000000000",
            "T|oob=000000000000000000000000000000000000",
            "T|p_oob=000000000000000000000000000000000000",
            "T|range_xk=000000000000000000000000000000000000",
            "T|bit_xk=000000000000000000000000000000000000",
            "T|inner_xk=000000000000000000000000000000000000",
            "T|p2=0000000000000000",
            "T|reads=xx,xx,xx,x,xx,xxxxxxxxx",
            "T|2s_read=00,00",
            "T|2s_write=0000000000000000",
            "T|ua=44,23,xx,xx",
            "T|comb_oob=xx,xxxxxx",
            "T|comb_xk=xx,xxxxxx",
            "T|nba_xk=000000000000000000000000000000000000",
        ]
    );
}
