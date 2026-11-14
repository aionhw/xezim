//! §8.9, §7.2.1: a member of a packed-struct static class property reads the
//! static cell: through the class name, a handle, a derived class or its
//! handle, inside a method and a static function of the class, below a nested
//! member and under a part-select, beside a module variable of a member's
//! name (#1050). Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn static_packed_struct_member_reads() {
    assert_eq!(
        lines(
            r#"typedef struct packed { logic vld; logic [2:0] tag; } h_t;
typedef struct packed { logic vld; h_t hdr; logic [7:0] addr; } p_t;
class C;
  static p_t sp;
  function void show(); $display("M vld=%b tag=%0d addr=%h", sp.vld, sp.hdr.tag, sp.addr); endfunction
  static function logic [7:0] get_addr(); return sp.addr; endfunction
  static function logic [2:0] get_tag(); return C::sp.hdr.tag; endfunction
endclass
class D extends C; endclass
module t;
  C h; D dh;
  logic [7:0] addr;
  initial begin
    h = new; dh = new; addr = 8'h11;
    C::sp = '{1'b1, '{1'b1, 3'd5}, 8'hA7};
    $display("A vld=%b hdr=%h tag=%0d addr=%h a30=%h", C::sp.vld, C::sp.hdr, C::sp.hdr.tag, C::sp.addr, C::sp.addr[3:0]);
    $display("B h.sp.vld=%b h.sp.hdr.tag=%0d dh.sp.addr=%h D::sp.addr=%h addr=%h", h.sp.vld, h.sp.hdr.tag, dh.sp.addr, D::sp.addr, addr);
    h.show();
    $display("C get=%h tag=%0d", C::get_addr(), C::get_tag());
    $finish;
  end
endmodule
"#
        ),
        [
            r#"A vld=1 hdr=d tag=5 addr=a7 a30=7"#,
            r#"B h.sp.vld=1 h.sp.hdr.tag=5 dh.sp.addr=a7 D::sp.addr=a7 addr=11"#,
            r#"M vld=1 tag=5 addr=a7"#,
            r#"C get=a7 tag=5"#,
        ]
    );
}
