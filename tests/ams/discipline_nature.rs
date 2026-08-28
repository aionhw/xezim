//! Stage 3 of Verilog-AMS support: the nature (§3.4) and discipline (§3.5)
//! declarations.
//!
//! Parse-only, deliberately. A nature carries the tolerances and the ACCESS
//! FUNCTION that names a quantity in a contribution (`V(a,b)`), and a
//! discipline binds the potential/flow natures a net carries — that is the
//! type system the analog kernel is built on, so it has to exist in the AST
//! before any of it can be elaborated or solved. Nothing here simulates.
//!
//! Assertions are on the AST rather than on simulator output: there is no
//! observable behavior yet, and "it did not fail to parse" would pass just as
//! well if the declaration were skipped to `endnature` and thrown away — which
//! is exactly the failure this stage has to rule out.

use crate::ams_mode::{with_ams, without_ams};
use sv_parser::ast::Description;
use sv_parser::ast::decl::DisciplineDomain;

fn parse_ams(src: &str) -> sv_parser::ast::SourceText {
    with_ams(|| {
        let res = sv_parser::parse(src);
        assert!(
            res.errors.is_empty(),
            "unexpected parse errors: {:?}",
            res.errors.iter().map(|d| d.to_string()).collect::<Vec<_>>()
        );
        res.source
    })
}

const STD_DISCIPLINES: &str = r#"
nature Voltage;
  units      = "V";
  access     = V;
  abstol     = 1e-6;
endnature

nature Current;
  units      = "A";
  access     = I;
  abstol     = 1e-12;
endnature

discipline electrical;
  potential Voltage;
  flow      Current;
enddiscipline
"#;

/// AMS §3.6.1: a nature's attributes are RETAINED, not skipped to `endnature`.
/// `abstol` is the solver's convergence tolerance and `access` names the
/// quantity function — dropping either makes the declaration decorative.
#[test]
fn a_nature_retains_its_attributes() {
    let ast = parse_ams(STD_DISCIPLINES);
    let voltage = ast
        .descriptions
        .iter()
        .find_map(|d| match d {
            Description::Nature(n) if n.name.name == "Voltage" => Some(n),
            _ => None,
        })
        .expect("nature Voltage");

    assert_eq!(voltage.attributes.len(), 3, "units, access, abstol");
    let names: Vec<&str> = voltage
        .attributes
        .iter()
        .map(|(k, _)| k.name.as_str())
        .collect();
    assert_eq!(names, vec!["units", "access", "abstol"], "source order kept");
    assert_eq!(
        voltage.access().map(|i| i.name.as_str()),
        Some("V"),
        "the access function is what names V(a,b) in a contribution"
    );
    assert!(voltage.parent.is_none());
}

/// AMS §3.6.1 derived nature: `nature Hi : Voltage;` refines a base. The parent
/// link is what an analog stage needs to inherit the unresolved attributes.
#[test]
fn a_derived_nature_records_its_parent() {
    let ast = parse_ams(
        r#"
nature Voltage;
  units  = "V";
  access = V;
  abstol = 1e-6;
endnature
nature Voltage_hi : Voltage;
  abstol = 1e-9;
endnature
"#,
    );
    let hi = ast
        .descriptions
        .iter()
        .find_map(|d| match d {
            Description::Nature(n) if n.name.name == "Voltage_hi" => Some(n),
            _ => None,
        })
        .expect("nature Voltage_hi");
    match hi.parent.as_ref().expect("parent") {
        sv_parser::ast::decl::ParentNature::Nature(id) => assert_eq!(id.name, "Voltage"),
        other => panic!("expected a plain nature parent, got {other:?}"),
    }
    assert_eq!(hi.attributes.len(), 1, "only the override is declared here");
    assert_eq!(hi.attributes[0].0.name, "abstol");
}

/// AMS §3.6.2: a discipline binds the potential and flow natures its nets carry.
/// `electrical` is the one every analog testbench starts from.
#[test]
fn a_discipline_binds_its_potential_and_flow_natures() {
    let ast = parse_ams(STD_DISCIPLINES);
    let elec = ast
        .descriptions
        .iter()
        .find_map(|d| match d {
            Description::Discipline(dd) if dd.name.name == "electrical" => Some(dd),
            _ => None,
        })
        .expect("discipline electrical");
    assert_eq!(elec.potential.as_ref().map(|i| i.name.as_str()), Some("Voltage"));
    assert_eq!(elec.flow.as_ref().map(|i| i.name.as_str()), Some("Current"));
    assert_eq!(elec.domain, None, "no explicit domain in this declaration");
}

/// AMS §3.6.2 `domain discrete` — how AMS types a plain digital net. Both domain
/// spellings must round-trip.
#[test]
fn a_discipline_records_an_explicit_domain() {
    let ast = parse_ams(
        r#"
discipline ddiscrete;
  domain discrete;
enddiscipline
discipline dcontinuous;
  domain continuous;
enddiscipline
"#,
    );
    let of = |want: &str| {
        ast.descriptions
            .iter()
            .find_map(|d| match d {
                Description::Discipline(dd) if dd.name.name == want => Some(dd.domain),
                _ => None,
            })
            .unwrap_or_else(|| panic!("discipline {}", want))
    };
    assert_eq!(of("ddiscrete"), Some(DisciplineDomain::Discrete));
    assert_eq!(of("dcontinuous"), Some(DisciplineDomain::Continuous));
}

/// Natures and disciplines coexist with ordinary module source in one file —
/// the layout of every real `.vams` design.
#[test]
fn declarations_coexist_with_module_source() {
    let mut src = STD_DISCIPLINES.to_string();
    src.push_str(
        r#"
module tb;
  wrealsum n;
  real a;
  assign n = a;
  initial begin a = 4.5; #1; end
endmodule
"#,
    );
    let ast = parse_ams(&src);
    let modules = ast
        .descriptions
        .iter()
        .filter(|d| matches!(d, Description::Module(_)))
        .count();
    let natures = ast
        .descriptions
        .iter()
        .filter(|d| matches!(d, Description::Nature(_)))
        .count();
    let disciplines = ast
        .descriptions
        .iter()
        .filter(|d| matches!(d, Description::Discipline(_)))
        .count();
    assert_eq!((modules, natures, disciplines), (1, 2, 1));
}

/// The gate. `nature`, `discipline`, `potential`, `flow`, `domain`, `ground`
/// are not IEEE 1800 keywords — with AMS off they must stay usable as
/// ordinary identifiers.
#[test]
fn gate_is_off_by_default() {
    without_ams(|| {
        let res = sv_parser::parse(
            r#"
module tb;
  integer nature, discipline, potential, flow, domain, ground, discrete;
  initial begin
    nature = 1; discipline = 2; potential = 3; flow = 4;
    domain = 5; ground = 6; discrete = 7;
  end
endmodule
"#,
        );
        assert!(
            res.errors.is_empty(),
            "AMS words must be plain identifiers with the gate off: {:?}",
            res.errors.iter().map(|d| d.to_string()).collect::<Vec<_>>()
        );
    });
}
