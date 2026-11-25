//! IEEE 1800-2023 §6.16 String data type semantics tests (Milestone 3).
//!
//! Covers both AST interpretation and compiled bytecode execution across:
//! - Empty `""` integral width (8 bits) vs default `string` initialization (len 0)
//! - NUL byte removal on string literals / integrals converted to `string`
//!   (module vars, locals, parameters, properties, formals, concat, repl,
//!   ternary, cast) while preserving `%u`/`%z` NUL bytes inside `string` values
//! - `s[i]` vs `getc`/`putc`/`substr` (X/Z and negative indices vs `int` casting)
//! - `itoa`/`hextoa`/`octtoa`/`bintoa` 32-bit `integer` formal casting
//! - `atoreal` prefix parsing and `_` digit separators
//! - Non-ASCII bytes `0x80..0xFF` lossless round-trips

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 500).unwrap_or_else(|e| panic!("simulation failed: {e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn string_defaults_and_empty_literal_width_ast_and_bytecode() {
    let src = r#"
class Holder;
  string prop;
  function string check_method();
    string loc;
    string r;
    r = {3{loc, "X"}};
    return $sformatf("M:%0d:%0d:%s:%s", prop.len(), loc.len(), (prop == "") ? "T" : "F", r);
  endfunction
endclass

module top;
  string ms;
  string arr1[2];
  string arr2[2][2];

  bit [15:0] b_cat;
  bit [23:0] b_rep;
  int eq_empty_nul;
  int eq_lit_nul_trail;
  int eq_lit_nul_lead;

  initial begin
    automatic string ls;
    automatic Holder h = new();
    string mr;

    b_cat = {"A", ""};
    b_rep = {3{""}};
    eq_empty_nul = ("" == 8'h00);
    eq_lit_nul_trail = ("a\0" == "a");
    eq_lit_nul_lead = ("\0a" == "a");

    $display("AST:%0d:%0d:%0d:%0d:%0d", ms.len(), ls.len(), arr1[1].len(), arr2[1][1].len(), $bits(""));
    $display("CMP:%0d:%0d:%0d:%0d:%s",
             (ms == ""), (arr1[0] == ""), (arr2[0][1] == ""),
             (ls == ""), {3{ms, "X"}});
    $display("LIT:%04h:%06h:%0d:%0d:%0d", b_cat, b_rep, eq_empty_nul, eq_lit_nul_trail, eq_lit_nul_lead);

    for (int k = 0; k < 4; k++) begin
      mr = h.check_method();
    end
    $display("%s", mr);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "AST:0:0:0:0:8",
            "CMP:1:1:1:1:XXX",
            "LIT:4100:000000:1:0:1",
            "M:0:0:T:XXX",
        ]
    );
}

#[test]
fn string_nul_stripping_and_unformatted_nul_preservation() {
    let src = r#"
class StrBox;
  string p = "a\0b";
  string p_int = 24'h410042;
  string p_empty = "";

  function string echo_formal(string f = "x\0y");
    return f;
  endfunction

  function string build_concat_and_tern(string s_in, bit sel);
    string loc_lit;
    string loc_int;
    string loc_repl;
    string loc_tern;
    string loc_cast;
    loc_lit = "hello\0world";
    loc_int = 24'h430044;
    loc_repl = {2{s_in, "\0X"}};
    loc_tern = sel ? s_in : "t\0u";
    loc_cast = string'(24'h450046);
    return $sformatf("%s|%s|%s|%s|%s|%s|%s|%s",
                     p, p_int, p_empty, loc_lit, loc_int, loc_repl, loc_tern, loc_cast);
  endfunction

  function string preserve_u_formal(string u);
    string copy_u;
    copy_u = {u, "!"};
    return copy_u;
  endfunction
endclass

function automatic string fn_formal(string f);
  return f;
endfunction

module top;
  parameter string P_LIT = "p\0q";
  parameter string P_INT = 24'h470048;

  string mod_lit = "hello\0world";
  string mod_zero = 0;
  string mod_int = 24'h410042;
  string mod_nb;
  string mod_u;
  string mod_z;

  initial begin
    automatic StrBox box = new();
    automatic bit cond = 0;
    automatic string empty_s;
    automatic string cat_lit;
    automatic string bc_u;
    automatic string bc_out;
    automatic string u_copy;
    automatic int cmp_ok;

    cat_lit = {empty_s, "m\0n"};
    cmp_ok = (empty_s == "") && ("ab" == box.p) && (box.p == "a\0b") && ("hello\0world" != "helloworld");
    mod_nb <= "nb\0ok";
    #1;

    $display("MOD:%s:%0d:%s:%s:%s:%s:%s:%0d",
             mod_lit, mod_zero.len(), mod_int, P_LIT, P_INT, cat_lit, mod_nb, cmp_ok);
    $display("FN:%s:%s:%s:%s",
             fn_formal("a\0b"), fn_formal(24'h49004a),
             box.echo_formal(), box.echo_formal("c\0d"));

    for (int i = 0; i < 4; i++) begin
      bc_out = box.build_concat_and_tern(empty_s, cond);
    end
    $display("BC:%s", bc_out);

    // %u / %z produce embedded NULs inside `string` expressions that must survive.
    mod_u = $sformatf("%u", 32'h41004243);
    mod_z = $sformatf("%z", 32'h44004546);
    u_copy = mod_u;
    for (int i = 0; i < 4; i++) begin
      bc_u = box.preserve_u_formal(mod_u);
    end
    $display("UZ:%0d:%02x:%02x:%0d:%0d:%0d",
             mod_u.len(), mod_u.getc(2), u_copy.getc(2),
             mod_z.len(), bc_u.len(), fn_formal(mod_u).len());
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "MOD:helloworld:0:AB:pq:GH:mn:nbok:1",
            "FN:ab:IJ:xy:cd",
            "BC:ab|AB||helloworld|CD|XX|tu|EF",
            "UZ:4:00:00:4:5:4",
        ]
    );
}

#[test]
fn string_indexing_getc_putc_substr_edges_ast_and_bytecode() {
    let src = r#"
class Ops;
  function string run_bc();
    string s = "ABC";
    string s_hi = "\xffZ";
    string r1;
    string r2;
    byte b_neg;
    byte b_oob;
    byte b_x;
    byte b_z;
    byte b_signed;
    int g_neg;
    int g_oob;
    int g_x;
    byte c;

    b_neg = s[-1];
    b_oob = s[3];
    b_x = s[4'bx];
    b_z = s[4'bz];
    b_signed = s_hi[0];
    c = s[1];

    s[-1] = "Q";
    s[3] = "Q";
    s[4'bx] = "Q";
    s[0] = 8'h00;

    g_neg = s.getc(-1);
    g_oob = s.getc(3);
    g_x = s.getc(4'bx);

    s.putc(-1, "W");
    s.putc(3, "W");
    s.putc(0, 0);
    s.putc(4'bx, "a");

    r1 = s.substr(-1, 1);
    r2 = s.substr(0, 0);
    return $sformatf("%0d:%0d:%0d:%0d:%0d:%0d|%0d:%0d:%0d|%s:%s:%s:%0d",
                     b_neg, b_oob, b_x, b_z, b_signed, c,
                     g_neg, g_oob, g_x, s, r1, r2, s.substr(2, 1).len());
  endfunction
endclass

module top;
  string q[$];

  initial begin
    automatic Ops o = new();
    automatic string s = "ABC";
    automatic string s_hi = "\xffZ";
    automatic byte b_neg, b_oob, b_x, b_z, b_signed;
    automatic int g_neg, g_oob, g_x;
    automatic string bc;

    b_neg = s[-1];
    b_oob = s[3];
    b_x = s[4'bx];
    b_z = s[4'bz];
    b_signed = s_hi[0];

    s[-1] = "Q";
    s[4'bx] = "Q";
    s[0] = 8'h00;

    g_neg = s.getc(-1);
    g_oob = s.getc(3);
    g_x = s.getc(4'bx);

    s.putc(-1, "W");
    s.putc(3, "W");
    s.putc(0, 0);
    s.putc(4'bx, "a");

    q.push_back("Hello");
    $display("AST:%0d:%0d:%0d:%0d:%0d|%0d:%0d:%0d|%s:%0d:%0d:%0d:%s",
             b_neg, b_oob, b_x, b_z, b_signed,
             g_neg, g_oob, g_x, s,
             s.substr(-1, 1).len(), s.substr(0, 3).len(), s.substr(2, 1).len(),
             q[0].substr(1, 3));

    for (int i = 0; i < 4; i++) begin
      bc = o.run_bc();
    end
    $display("BC:%s", bc);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "AST:0:0:0:0:-1|0:0:65|aBC:0:0:0:ell",
            "BC:0:0:0:0:-1:66|0:0:65|aBC::a:0",
        ]
    );
}

#[test]
fn string_toa_32bit_casting_atoreal_and_non_ascii_roundtrip() {
    let src = r#"
class Fmt;
  function string check_bc();
    string s1;
    string s2;
    string s3;
    string s4;
    string s5;
    string hi1;
    string hi2;
    byte sb = -5;
    byte unsigned ub = 251;
    longint wide = 64'h1_0000_0005;
    logic [3:0] x4 = 4'bx;
    real r1;
    real r2;
    real r3;
    real r4;

    s1.itoa(sb);
    s2.hextoa(sb);
    s3.hextoa(ub);
    s4.itoa(wide);
    s5.itoa(x4);

    hi1 = "\x80\xff";
    hi2 = hi1;
    hi2.putc(0, "\xfe");

    r1 = "12.5abc".atoreal();
    r2 = "-3_2.5e+1xyz".atoreal();
    r3 = "1_000.5".atoreal();
    r4 = "_1.5".atoreal();

    return $sformatf("%s:%s:%s:%s:%s|%02x:%02x:%0d|%.2f:%.1f:%.1f:%.1f",
                     s1, s2, s3, s4, s5,
                     hi2.getc(0) & 8'hff, hi2.getc(1) & 8'hff,
                     (hi1.compare(hi2) < 0),
                     r1, r2, r3, r4);
  endfunction
endclass

module top;
  initial begin
    automatic Fmt f = new();
    automatic string s1, s2, s3, s4, s5, s6, s7;
    automatic string out;
    automatic byte sb = -5;
    automatic byte unsigned ub = 251;
    automatic longint wide = 64'h1_0000_0005;
    automatic logic [3:0] x4 = 4'bx;
    automatic string hi = "\x80\xff";
    automatic string sub;

    s1.itoa(sb);
    s2.hextoa(sb);
    s3.hextoa(ub);
    s4.octtoa(sb);
    s5.bintoa(ub);
    s6.itoa(wide);
    s7.itoa(x4);
    $display("AST:%s:%s:%s:%s:%s:%s:%s|%.2f:%.1f:%.1f:%.1f:%.1f",
             s1, s2, s3, s4, s5, s6, s7,
             "  +12.5abc".atoreal(),
             "-3_2.5e+1xyz".atoreal(),
             "1_000.5".atoreal(),
             "_1.5".atoreal(),
             "abc".atoreal());

    sub = {hi.substr(0, 0), "\xaa"};
    $display("RAW:%02x:%02x:%0d:%0d",
             sub.getc(0) & 8'hff, sub.getc(1) & 8'hff,
             ("\xff".compare("\x01") > 0),
             ("aBc".icompare("AbC") == 0));

    for (int i = 0; i < 4; i++) begin
      out = f.check_bc();
    end
    $display("BC:%s", out);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "AST:-5:fffffffb:fb:37777777773:11111011:5:0|12.50:-325.0:1000.5:0.0:0.0",
            "RAW:80:aa:1:1",
            "BC:-5:fffffffb:fb:5:0|fe:ff:1|12.50:-325.0:1000.5:0.0",
        ]
    );
}
