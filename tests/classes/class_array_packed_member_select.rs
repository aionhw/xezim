//! §7.2.1, §7.4.1, §8.4, §11.5.1: bit-select, constant part-select, and
//! indexed part-select reads and writes on packed members of class-property
//! arrays (fixed, dynamic, queue, associative, and 2D).

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn class_array_packed_member_reads_and_writes() {
    let src = r#"
module t;
  typedef struct packed { logic [3:0] tag; logic [1:0][3:0] f; } e_t;
  typedef struct packed { logic [31:0] vl; logic [2:0] sew; logic [1:0][3:0] arr; } st_t;
  typedef struct packed { logic [7:0] op; st_t arch; } cmd_t;
  typedef struct packed { logic [0:3] asc; logic [4:1] nz; } odd_t;

  class C;
    e_t ea [2];
    cmd_t inst [2];
    cmd_t da [];
    cmd_t q [$];
    cmd_t aa [int];
    cmd_t m2 [2][2];
    odd_t oa [2];

    function void run_tests();
      logic [1:0] k = 0;
      logic [1:0] xk = 2'bx1;

      ea[0] = '0; ea[0].f[0] = 4'h5;
      ea[0].f[0][3:1] = 3'b011;
      $display("T|ea_range=%h,%b", ea[0].f[0], ea[0].f[0][3:1]);

      ea[0] = '0; ea[0].f[0] = 4'h5;
      ea[0].f[0][1] = 1'b1;
      $display("T|ea_bit=%h,%b", ea[0].f[0], ea[0].f[0][1]);

      ea[0] = '0; ea[0].f[0] = 4'h5;
      ea[0].f[0][k+:2] = 2'b10;
      $display("T|ea_idx=%h,%b", ea[0].f[0], ea[0].f[0][k+:2]);

      inst[0] = '0; inst[1] = '0;
      inst[0].arch.vl[7:4] = 4'ha;
      inst[0].arch.vl[1] = 1'b1;
      inst[0].arch.sew[2:1] = 2'b11;
      inst[0].arch.arr[1] = 4'h9;
      inst[0].arch.arr[0][3:1] = 3'b101;
      inst[0].arch.arr[0][0] = 1'b1;
      $display("T|nested=%h,%b,%h,%h,%h,%b,%b",
        inst[0].arch.vl, inst[0].arch.sew, inst[0].arch.arr[1], inst[0].arch.arr[0],
        inst[0].arch.vl[7:4], inst[0].arch.arr[0][3:1], inst[0].arch.arr[0][0]);

      da = new[2]; da[0] = '0; da[1] = '0;
      da[1].arch.vl = 32'd42;
      da[1].op[1] = 1'b1;
      da[1].arch.vl[7:4] = 4'hb;
      $display("T|da=%h,%b", da[1].arch.vl, da[1].op);

      q = {'0, '0};
      q[1].arch.vl = 32'd77;
      q[1].op[2] = 1'b1;
      q[1].arch.vl[7:4] = 4'hc;
      $display("T|q=%h,%b", q[1].arch.vl, q[1].op);

      aa[5] = '0;
      aa[5].arch.vl = 32'd88;
      aa[5].op[3] = 1'b1;
      aa[5].arch.vl[7:4] = 4'hd;
      $display("T|aa=%h,%b", aa[5].arch.vl, aa[5].op);

      m2[1][0] = '0;
      m2[1][0].arch.vl = 32'd55;
      m2[1][0].op[1] = 1'b1;
      $display("T|m2=%0d,%b", m2[1][0].arch.vl, m2[1][0].op);

      oa[0] = '0;
      oa[0].asc[0] = 1'b1;
      oa[0].asc[2:3] = 2'b10;
      oa[0].nz[4] = 1'b1;
      oa[0].nz[3:2] = 2'b10;
      $display("T|odd=%b,%b,%b,%b", oa[0].asc, oa[0].nz, oa[0].asc[2:3], oa[0].nz[3:2]);

      inst[1] = '0;
      inst[xk].arch.vl = 32'hdeadbeef;
      inst[xk].op[1] = 1'b1;
      $display("T|xk=%h,%b,%h,%b", inst[1].arch.vl, inst[1].op, inst[xk].arch.vl, inst[xk].op[1]);

      begin
        e_t ea [2];
        ea[0] = '0; ea[0].f[0] = 4'h9;
        ea[0].f[0][3:1] = 3'b001;
        $display("T|shadow=%h", ea[0].f[0]);
      end
      $display("T|after_shadow=%h", this.ea[0].f[0]);
    endfunction
  endclass

  initial begin
    C c = new;
    c.run_tests();
    c.ea[1] = '0; c.ea[1].f[0] = 4'h5;
    c.ea[1].f[0][3:1] = 3'b011;
    c.inst[1] = '0;
    c.inst[1].arch.vl = 32'h1234;
    c.inst[1].arch.vl[3:0] = 4'h9;
    $display("T|handle=%h,%b,%h", c.ea[1].f[0], c.ea[1].f[0][3:1], c.inst[1].arch.vl);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|ea_range=7,011",
            "T|ea_bit=7,1",
            "T|ea_idx=6,10",
            "T|nested=000000a2,110,9,b,a,101,1",
            "T|da=000000ba,00000010",
            "T|q=000000cd,00000100",
            "T|aa=000000d8,00001000",
            "T|m2=55,00000010",
            "T|odd=1010,1100,10,10",
            "T|xk=00000000,00000000,xxxxxxxx,x",
            "T|shadow=3",
            "T|after_shadow=6",
            "T|handle=7,011,00001239",
        ]
    );
}
