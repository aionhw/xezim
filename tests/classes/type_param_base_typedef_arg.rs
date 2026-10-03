//! §6.18 / §8.25: a typedef names the type it is declared as, so a class
//! extending `wrap #(my_s)`, with `typedef sbase #(my_cfg) my_s;` and
//! `class wrap #(type BASE) extends BASE;`, has the ancestor `sbase
//! #(my_cfg)` exactly as one extending `wrap #(sbase #(my_cfg))` does. The
//! entry linked for the derived class read `sbase`'s arguments back from
//! the binding of `BASE`, which the bare typedef name did not spell, so
//! `sbase` ran with its defaults: a `CFG` object built in its constructor
//! was a `cfg_base`. This is the shape of tue's sequencer behind
//! `tvip_axi_sequencer_base #(.BASE(<typedef of tue_sequencer>))`.

use xezim::simulate;

const SRC: &str = r#"
class cfg_base; virtual function string name(); return "cfg_base"; endfunction endclass
class my_cfg extends cfg_base; virtual function string name(); return "my_cfg"; endfunction endclass
class st_base; virtual function string name(); return "st_base"; endfunction endclass
class my_st extends st_base; virtual function string name(); return "my_st"; endfunction endclass
class proxy_root; endclass
class proxy_base #(type CFG = cfg_base) extends proxy_root; endclass
class proxy #(type CFG = cfg_base) extends proxy_base #(CFG); endclass
class sbase #(type CFG = cfg_base, type ST = st_base, type PCFG = CFG);
  cfg_base c;
  st_base s;
  proxy_root px;
  function new();
    CFG t = new();
    ST u = new();
    proxy #(PCFG) p = new();
    c = t;
    s = u;
    px = p;
  endfunction
endclass
typedef sbase #(my_cfg) my_s;
typedef sbase #(.ST(my_st), .CFG(my_cfg)) my_named_s;
class wrap #(type BASE = int) extends BASE; endclass
class wrap2 #(type BASE = int) extends wrap #(BASE); endclass
class via_typedef extends wrap #(my_s); endclass
class via_spec    extends wrap #(sbase #(my_cfg)); endclass
class via_named   extends wrap #(.BASE(my_named_s)); endclass
class via_two     extends wrap2 #(my_s); endclass
class via_deeper  extends via_typedef; endclass
module top;
  initial begin
    via_typedef  a = new();
    via_spec     b = new();
    via_named    n = new();
    via_two      w = new();
    via_deeper   d = new();
    wrap #(my_s) c = new();
    proxy_base #(my_cfg) pb;
    $display("typedef=%s spec=%s named=%s,%s two=%s deeper=%s itself=%s proxy_cast=%0d",
             a.c.name(), b.c.name(), n.c.name(), n.s.name(), w.c.name(), d.c.name(), c.c.name(),
             $cast(pb, a.px));
  end
endmodule
"#;

#[test]
fn typedef_specialization_through_type_param_base_keeps_its_arguments() {
    let out: Vec<String> = simulate(SRC, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(
        out,
        [
            "typedef=my_cfg spec=my_cfg named=my_cfg,my_st two=my_cfg deeper=my_cfg itself=my_cfg proxy_cast=1"
        ],
        "{out:?}"
    );
}
