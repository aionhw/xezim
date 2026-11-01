//! §10.9.1, §10.9.2: an array pattern on a packed array fills it element by
//! element, also when the array is a member of a packed struct: a nested
//! `'{default: v}` member pattern, a direct `cfg.dom = '{...}` with
//! positional items, `default:` or `index: value` items, and a packed or
//! unpacked array of packed structs whose `default:` is a value or a struct
//! pattern (#771). Each kept only the low element, wrote nothing, or laid the
//! items out at their own widths. Index keys name their element on any
//! packed array. A struct-level `default:` fills each innermost element of a
//! multi-dimensional packed-array member, which is no simple bit vector type
//! (§6.11.1): the `unpacked elems`, `struct default` and `comb struct
//! default` lines follow §10.9.2 there, where the reference simulator sizes
//! the whole member; every other expected T| line is the reference's.

use xezim::simulate;

const SRC: &str = r#"
module top;
  typedef struct packed {
    bit [3:0] id;
    bit [4:0][3:0] dom;
    bit [12:0] w;
  } cfg_t;
  typedef struct packed { bit [3:0] a; bit [7:0] b; } s_t;
  cfg_t cfg, cfg2, cfg3, cfg4, cfg5;
  cfg_t ua [1:0];
  s_t [1:0] pa, pb, pc, pd;
  s_t e1, e2;
  bit [2:0][3:0] v, w;
  bit [0:2][3:0] u;
  always_comb pd = '{default: 4'h6};
  always_comb cfg5 = '{id: 4'd1, default: 8'hAA};
  initial begin
    cfg = '{id: 4'd9, dom: '{default: 4'hA}, w: 13'd170};
    $display("T|nested %h %h %0d", cfg.id, cfg.dom, cfg.w);
    cfg.dom = '{default: 4'h5};
    $display("T|member default %h", cfg.dom);
    cfg2.dom = '{4'h1, 4'h2, 4'h3, 4'h4, 4'h5};
    $display("T|member ordered %h", cfg2.dom);
    cfg3.dom = '{0: 4'h7, default: 4'h1};
    $display("T|member keyed %h", cfg3.dom);
    ua = '{default: '{id: 4'd9, default: 4'h3}};
    $display("T|unpacked elems %h %h", ua[0], ua[1]);
    pa = '{default: 4'h3};
    e1 = '{a: 1, b: 2}; e2 = '{a: 3, b: 4};
    pb = '{e1, e2};
    pc = '{'{a: 5, b: 6}, '{a: 7, b: 8}};
    $display("T|packed elems %h %h %h", pa, pb, pc);
    pa = '{default: '{a: 9, default: 1}};
    $display("T|packed pattern default %h", pa);
    v = '{0: 4'h7, default: 4'h1};
    w = '{2: 4'h9, 0: 4'h3, 1: 4'h5};
    u = '{0: 4'h7, default: 4'h1};
    $display("T|keyed vectors %h %h %h", v, w, u);
    cfg4 = '{default: 8'hAA};
    $display("T|struct default %h %h %h", cfg4.id, cfg4.dom, cfg4.w);
    pc <= '{default: '{a: 2, default: 4}};
    #1 $display("T|nonblocking %h comb %h", pc, pd);
    $display("T|comb struct default %h %h %h", cfg5.id, cfg5.dom, cfg5.w);
  end
endmodule
"#;

#[test]
fn array_patterns_fill_packed_members_and_struct_arrays_per_element() {
    let lines: Vec<String> = simulate(SRC, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|nested 9 aaaaa 170",
            "T|member default 55555",
            "T|member ordered 12345",
            "T|member keyed 11117",
            "T|unpacked elems 1266666003 1266666003",
            "T|packed elems 303303 102304 506708",
            "T|packed pattern default 901901",
            "T|keyed vectors 117 953 711",
            "T|struct default a aaaaa 00aa",
            "T|nonblocking 204204 comb 606606",
            "T|comb struct default 1 aaaaa 00aa",
        ]
    );
}

/// §10.9.2: the same per-element `default:` reaches a packed-array member
/// of a packed struct held by a class property, written in a method or
/// through a handle, of an element of an unpacked array of structs, and of
/// an unpacked struct. Expected values follow §10.9.2 and §6.11.1.
const STORAGE: &str = r#"
module top;
  typedef struct packed { bit [3:0] id; bit [2:0][3:0] dom; } cfg_t;
  typedef struct { bit [3:0] id; bit [2:0][3:0] dom; } ucfg_t;
  class C; cfg_t c; function void f(); c = '{default: 8'hAA}; endfunction endclass
  cfg_t arr [2];
  ucfg_t u;
  C h;
  initial begin
    h = new; h.f();
    $display("L|class %h", h.c);
    h.c = '{default: 8'h55};
    $display("L|handle %h", h.c);
    arr[1] = '{default: 8'hAA};
    $display("L|elem %h", arr[1]);
    u = '{default: 8'hAA};
    $display("L|unpacked %h %h", u.id, u.dom);
  end
endmodule
"#;

#[test]
fn a_struct_default_fills_packed_array_members_in_every_storage_kind() {
    let lines: Vec<String> = simulate(STORAGE, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("L|"))
        .collect();
    assert_eq!(
        lines,
        [
            "L|class aaaa",
            "L|handle 5555",
            "L|elem aaaa",
            "L|unpacked a aaa"
        ]
    );
}
