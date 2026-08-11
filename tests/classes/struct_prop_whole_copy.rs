//! §10.6/§7.2 — whole-value copy of an UNPACKED-STRUCT class property
//! (members: class handle + string) through a FRAME-HELD handle (a
//! function argument), in statement-assign, decl-init, and ?: forms.
//! Reference-validated. Field-wise reads always worked; the whole-struct
//! copy came back x because there are no flat `<obj>.<prop>.<field>`
//! signals to assemble for an arg-held receiver (module-scope receivers
//! had them and passed). This was the root cause of UVM factory
//! set_type_override never matching: the matcher copies `override.orig`
//! into a local through a ternary.

use xezim::simulate;

fn outs(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reference: plain null=0 name=[base_c]; tern sel=1 -> base_c;
/// tern sel=0 -> null=1 name=[deriv_c]; stmt-assign null=0 name=[base_c].
#[test]
fn struct_class_prop_copies_through_arg_handle() {
    let src = r#"
class W;
  string nm;
  function new(string n); nm = n; endfunction
endclass
typedef struct { W m_type; string m_type_name; } pair_t;
class OV;
  pair_t orig;
  pair_t ovrd;
endclass
module top;
  function automatic void probe(OV o, bit sel);
    pair_t p = sel ? o.orig : o.ovrd;
    $display("T|tern%0d null=%0d name=[%s]", sel, p.m_type == null, p.m_type_name);
  endfunction
  function automatic void probe2(OV o);
    pair_t p = o.orig;
    $display("T|init null=%0d name=[%s]", p.m_type == null, p.m_type_name);
  endfunction
  function automatic void probe3(OV o);
    pair_t p;
    p = o.orig;
    $display("T|assign null=%0d name=[%s]", p.m_type == null, p.m_type_name);
  endfunction
  initial begin
    automatic OV o = new;
    automatic W w = new("base_c");
    o.orig.m_type = w;
    o.orig.m_type_name = "base_c";
    o.ovrd.m_type_name = "deriv_c";
    probe2(o);
    probe3(o);
    probe(o, 1);
    probe(o, 0);
  end
endmodule
"#;
    let out = outs(&simulate(src, 10).expect("sim"));
    for want in [
        "T|init null=0 name=[base_c]",
        "T|assign null=0 name=[base_c]",
        "T|tern1 null=0 name=[base_c]",
        "T|tern0 null=1 name=[deriv_c]",
    ] {
        assert!(out.contains(want), "missing `{}`:\n{}", want, out);
    }
}
