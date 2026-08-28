//! Verilog-AMS 2.4.0 grammar conformance — the declaration forms taken
//! verbatim from the LRM's own syntax boxes and examples.
//!
//! Written after auditing the implementation against the standard. Every case
//! here was BROKEN when it was written, and each failed in a different way:
//!
//!   * `wreal electrical wd;` and `wreal [3:0] wv;` (§3.7) parsed without
//!     error but lost `real` — the discipline was taken as the data type and
//!     the range as packed dimensions, so a real driven onto the net
//!     truncated to its LSB with nothing said.
//!   * `nature n : electrical.potential;` (§3.6.1) silently recorded the
//!     DISCIPLINE as the parent, dropping `.potential`.
//!   * `potential.abstol = 1u;` (§3.6.2) was a hard parse error on legal
//!     source.
//!   * `ground gnd;` (§3.6.4) was a hard parse error, because the keyword was
//!     reserved without a rule to consume it.
//!
//! Two of those four were silent, which is why these assert on the AST rather
//! than on "no errors".

use crate::ams_mode::with_ams;
use sv_parser::ast::Description;
use sv_parser::ast::decl::{ModuleItem, ParentNature, PotentialOrFlow};
use sv_parser::ast::types::{DataType, NetType};

fn parse_ams(src: &str) -> sv_parser::ast::SourceText {
    with_ams(|| {
        let res = sv_parser::parse(src);
        assert!(
            res.errors.is_empty(),
            "LRM-legal source must parse: {:?}",
            res.errors.iter().map(|d| d.to_string()).collect::<Vec<_>>()
        );
        res.source
    })
}

/// §3.7: `wreal [ discipline_identifier ] [ range ] list_of_net_identifiers ;`
///
/// All three shapes must keep the real data type. The discipline and the range
/// are parsed and dropped (the net stays scalar and undisciplined), but losing
/// `real` is the failure that matters — it is silent and it corrupts values.
#[test]
fn every_wreal_declaration_form_keeps_the_real_type() {
    let ast = parse_ams(
        r#"
module m;
  wreal electrical wd;
  wreal [3:0] wv;
  wreal plain;
endmodule
"#,
    );
    let Description::Module(m) = &ast.descriptions[0] else { panic!("module") };
    let nets: Vec<_> = m
        .items
        .iter()
        .filter_map(|i| match i {
            ModuleItem::NetDeclaration(nd) => Some(nd),
            _ => None,
        })
        .collect();
    assert_eq!(nets.len(), 3, "three wreal declarations");
    for (nd, want) in nets.iter().zip(["wd", "wv", "plain"]) {
        assert!(
            matches!(nd.net_type, NetType::Wreal(_)),
            "{want}: net type must be wreal, got {:?}",
            nd.net_type
        );
        assert!(
            matches!(nd.data_type, DataType::Real { .. }),
            "{want}: a wreal must carry `real`, got {:?} — a real driven onto \
             this net would truncate",
            nd.data_type
        );
        assert_eq!(nd.declarators[0].name.name, want);
    }
}

/// §3.6.1: `parent_nature ::= nature_identifier
///                          | discipline_identifier . potential_or_flow`
///
/// The dotted form names whichever nature the discipline bound to that access,
/// so it cannot be flattened to the discipline's own name.
#[test]
fn a_parent_nature_may_name_a_discipline_access() {
    let ast = parse_ams(
        r#"
nature plain_parent : Voltage;
endnature
nature via_discipline : electrical.potential;
  abstol = 1n;
endnature
nature via_flow : electrical.flow;
endnature
"#,
    );
    let nature = |want: &str| {
        ast.descriptions
            .iter()
            .find_map(|d| match d {
                Description::Nature(n) if n.name.name == want => Some(n),
                _ => None,
            })
            .unwrap_or_else(|| panic!("nature {want}"))
    };
    match nature("plain_parent").parent.as_ref().expect("parent") {
        ParentNature::Nature(id) => assert_eq!(id.name, "Voltage"),
        other => panic!("expected a plain nature parent, got {other:?}"),
    }
    match nature("via_discipline").parent.as_ref().expect("parent") {
        ParentNature::DisciplineAccess { discipline, which } => {
            assert_eq!(discipline.name, "electrical");
            assert_eq!(*which, PotentialOrFlow::Potential);
        }
        other => panic!("expected a discipline access parent, got {other:?}"),
    }
    match nature("via_flow").parent.as_ref().expect("parent") {
        ParentNature::DisciplineAccess { which, .. } => {
            assert_eq!(*which, PotentialOrFlow::Flow)
        }
        other => panic!("expected a discipline access parent, got {other:?}"),
    }
}

/// §3.6.2: `nature_attribute_override ::= potential_or_flow . nature_attribute`
///
/// A discipline narrowing a tolerance it inherited from the bound nature —
/// most of the reason a design declares its own discipline at all. It shares
/// its leading keyword with `nature_binding`, and the dot is what tells them
/// apart.
#[test]
fn a_discipline_may_override_a_bound_natures_attribute() {
    let ast = parse_ams(
        r#"
discipline electrical;
  potential Voltage;
  flow Current;
  potential.abstol = 1u;
  flow.abstol = 1p;
enddiscipline
"#,
    );
    let Description::Discipline(d) = &ast.descriptions[0] else { panic!("discipline") };
    assert_eq!(d.potential.as_ref().map(|i| i.name.as_str()), Some("Voltage"));
    assert_eq!(d.flow.as_ref().map(|i| i.name.as_str()), Some("Current"));
    assert_eq!(d.overrides.len(), 2, "both overrides retained");
    assert_eq!(d.overrides[0].0, PotentialOrFlow::Potential);
    assert_eq!(d.overrides[0].1.name, "abstol");
    assert_eq!(d.overrides[1].0, PotentialOrFlow::Flow);
    assert_eq!(d.overrides[1].1.name, "abstol");
}

/// §3.6.2: a signal-flow discipline binds only ONE of potential/flow. The
/// binding arm must not require both.
#[test]
fn a_signal_flow_discipline_binds_one_nature() {
    let ast = parse_ams(
        r#"
discipline voltage;
  potential Voltage;
enddiscipline
discipline current;
  flow Current;
enddiscipline
"#,
    );
    let d = |want: &str| {
        ast.descriptions
            .iter()
            .find_map(|x| match x {
                Description::Discipline(dd) if dd.name.name == want => Some(dd),
                _ => None,
            })
            .unwrap_or_else(|| panic!("discipline {want}"))
    };
    assert!(d("voltage").potential.is_some() && d("voltage").flow.is_none());
    assert!(d("current").flow.is_some() && d("current").potential.is_none());
}

/// §3.6.4: `ground [ discipline_identifier ] [ range ] list_of_net_identifiers ;`
///
/// Parse-accepted and dropped — the reference node means nothing without the
/// analog solver. It must still be CONSUMED: reserving the keyword with no
/// rule behind it turned the LRM's own example into a parse error.
#[test]
fn a_ground_declaration_parses() {
    parse_ams(
        r#"
module loadedsrc(in, out);
  input in;
  output out;
  wreal in, out;
  wreal gnd;
  ground gnd;
endmodule
"#,
    );
}

/// §3.6.1/§3.6.2: the LRM's own `nature`/`discipline` examples, verbatim,
/// including the `idt_nature` attribute and the `1u`/`1m` real literals.
#[test]
fn the_lrm_nature_and_discipline_examples_parse() {
    let ast = parse_ams(
        r#"
nature current;
  units = "A";
  access = I;
  idt_nature = charge;
  abstol = 1u;
endnature
nature voltage;
  units = "V";
  access = V;
  abstol = 1u;
endnature
nature new_curr : current;
  abstol = 1m;
  maxval = 12.3;
endnature
discipline electrical;
  potential voltage;
  flow current;
enddiscipline
"#,
    );
    let n = ast
        .descriptions
        .iter()
        .find_map(|d| match d {
            Description::Nature(x) if x.name.name == "current" => Some(x),
            _ => None,
        })
        .expect("nature current");
    assert_eq!(n.access().map(|i| i.name.as_str()), Some("I"));
    assert_eq!(n.attributes.len(), 4, "units, access, idt_nature, abstol");

    let d = ast
        .descriptions
        .iter()
        .find_map(|x| match x {
            Description::Discipline(dd) => Some(dd),
            _ => None,
        })
        .expect("discipline");
    assert!(d.potential.is_some() && d.flow.is_some(), "conservative discipline");
}

/// §3.7's SECOND production:
/// `wreal [ discipline ] [ range ] list_of_net_decl_assignments ;`
///
/// The declaration-with-initializer form, which is a continuous assignment on
/// the net (§10.3.1). Checked through the simulator rather than the AST — the
/// point is that the initializer drives the net and the value stays real.
#[test]
fn a_wreal_net_declaration_assignment_drives_the_net() {
    use crate::util::r;
    let sim = with_ams(|| {
        xezim::simulate(
            r#"
module tb;
  wreal w = 1.5;
  wreal electrical wd = 2.5;
  initial #1;
endmodule
"#,
            10,
        )
        .expect("simulate")
    });
    assert_eq!(r(&sim, "w"), 1.5);
    assert_eq!(r(&sim, "wd"), 2.5, "the discipline form must drive too");
}
