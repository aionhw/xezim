use xezim::simulate;

fn check(source: &str) {
    let result = simulate(source, 100).unwrap();
    let output = result
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.contains("PASS") && !output.contains("Fatal"),
        "{output}"
    );
}

fn policies() -> bool {
    super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0"), ("1", "1000")])
}

#[test]
fn compiled_enum_case_keeps_lexical_class_constants() {
    if !policies() {
        return;
    }
    check(include_str!("../fixtures/compiled_lexical_enum.sv"));
}

#[test]
fn formal_shadows_compiled_class_enum_constant() {
    if !policies() {
        return;
    }
    check(&include_str!("../fixtures/compiled_lexical_enum.sv")
        .replace("extern function kind_t channel();", "extern function kind_t channel(); function int shadow(int RD_PLAIN); return RD_PLAIN; endfunction")
        .replace("$display(\"PASS\");", "repeat(2005) if(t.shadow(21)!=21) $fatal(1,\"enum shadow\"); $display(\"PASS\");"));
}

#[test]
fn compiled_other_object_string_store_preserves_all_bytes() {
    if !policies() {
        return;
    }
    check(include_str!("../fixtures/compiled_member_string.sv"));
}

#[test]
fn compiled_other_object_wide_store_preserves_all_bits() {
    if !policies() {
        return;
    }
    check(
        &include_str!("../fixtures/compiled_member_string.sv")
            .replace("string", "bit[127:0]")
            .replace(
                "\"read_data_21399\"",
                "128'h123456789abcdef0_fedcba9876543210",
            )
            .replace("\"\"", "'0"),
    );
}

#[test]
fn class_localparam_shadows_inherited_enum_constant() {
    if !policies() {
        return;
    }
    check(
        r#"
class base;
 typedef enum {TOKEN=1} base_kind;
endclass
class child extends base;
 localparam int TOKEN=7;
 function int value(); return TOKEN; endfunction
endclass
class unrelated;
 typedef enum {TOKEN=14} other_kind;
endclass
module top;
 initial begin child c=new();
   repeat(2005) if(c.value()!=7) $fatal(1,"class parameter shadow");
   $display("PASS");$finish;
 end
endmodule
"#,
    );
}

#[test]
fn compiled_typedef_vif_copy_keeps_binding() {
    if !policies() {
        return;
    }
    check(include_str!("../fixtures/compiled_vif_copy.sv"));
}

#[test]
fn compiled_type_parameter_vif_copy_keeps_binding() {
    if !policies() {
        return;
    }
    check(
        &include_str!("../fixtures/compiled_vif_copy.sv")
            .replace(
                "class cfg;\n typedef virtual bus VIF;",
                "class cfg #(type VIF=int);",
            )
            .replace("cfg rhs", "cfg#(VIF) rhs")
            .replace(
                "cfg c=new();cfg d=new();",
                "cfg#(virtual bus) c=new();cfg#(virtual bus) d=new();",
            ),
    );
}

#[test]
fn compiled_vif_copy_rebinds_and_clears() {
    if !policies() {
        return;
    }
    check(&include_str!("../fixtures/compiled_vif_copy.sv")
        .replace("bus b();", "bus b();bus second();")
        .replace("$display(\"PASS\");", "c.set(second);d.copy(c);d.vif.value=33;if(second.value!=33 || b.value!=17)$fatal(1,\"rebind failed\");c.set(null);d.copy(c);if(d.vif!=null)$fatal(1,\"clear failed\");$display(\"PASS\");"));
}

/// IEEE 1800 §8.24 / §26.3: a forward-declared package class and its
/// imported/qualified aliases must all retain out-of-class method bodies.
#[test]
fn package_alias_keeps_extern_constructor_body() {
    if !policies() {
        return;
    }
    check(include_str!(
        "../fixtures/extern_constructor_package_alias.sv"
    ));
}
