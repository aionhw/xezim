//! §6.19: enum member values with a base type WIDER than 64 bits.
//!
//! The member pipeline is u64 end to end (`expand_enum_member`,
//! `enum_members`, the per-scope registration sites), so every member
//! constant was silently truncated to its low 64 bits — and the §6.19
//! auto-increment chain ran down there too, so even an unspecified member
//! carried only the low half of its predecessor. Comparisons and case arms
//! still "worked" because both sides truncated identically, which is how a
//! 96-bit enum shipped broken for so long.
//!
//! Fixed with a Value-domain twin of the expansion (`wide_enum_value_map`)
//! consulted by every registration site (module/package/anonymous/instance)
//! and by the class and qualified-package lookups. The u64 tables keep their
//! shape for iteration order and rand pools.
//!
//! Every expected value is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn wide_enum_members_keep_all_bits_in_every_scope() {
    let o = out(r#"
package pk;
  typedef enum logic [95:0] { PV = 96'h112233445566778899aabbcc } p_t;
endpackage
module sub ();
  import pk::*;
  typedef enum logic [95:0] {
    SV96 = 96'haaaabbbbccccddddeeeeffff,
    SN                                   // auto-increment past a wide init
  } s_t;
  s_t s; p_t pp; logic [95:0] q;
  initial begin
    s = SV96; pp = PV; q = pk::PV;       // wildcard AND qualified
    $display("SUB=%h SN=%h PKG=%h QP=%h", s, SN, pp, q);
  end
endmodule
module tb;
  typedef enum logic [127:0] { W128 = 128'hdeadbeef_cafef00d_12345678_9abcdef0 } f_t;
  f_t w;
  sub u ();
  class c;
    typedef enum logic [95:0] { CV = 96'h0123456789abcdef01234567 } c_t;
    c_t x;
    function void go(); x = CV; $display("CLS=%h", x); endfunction
  endclass
  initial begin
    c o = new();
    w = W128;
    #1 o.go();
    $display("W=%h", w);
  end
endmodule
"#);
    for expect in [
        "SUB=aaaabbbbccccddddeeeeffff SN=aaaabbbbccccddddeeef0000 \
          PKG=112233445566778899aabbcc QP=112233445566778899aabbcc",
        "CLS=0123456789abcdef01234567",
        "W=deadbeefcafef00d123456789abcdef0",
    ] {
        let want: String = expect.split_whitespace().collect::<Vec<_>>().join(" ");
        let got: String = o.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(got.contains(&want), "expected `{want}` in:\n{o}");
    }
}
