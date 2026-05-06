//! SV LRM compliance tests (unit tests for compiler components).
use xezim::ast::decl::*;
use xezim::ast::expr::*;
use xezim::ast::module::*;
use xezim::ast::stmt::*;
use xezim::ast::*;
use xezim::*;

fn parse_ok(source: &str) -> ParseResult {
    let result = xezim::parse_str(source);
    match result {
        Ok(res) => {
            if !res.errors.is_empty() {
                for d in &res.errors {
                    eprintln!("{}", d);
                }
                panic!("Expected no parse errors");
            }
            res
        }
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", d);
            }
            panic!("Parse failed");
        }
    }
}

fn first_module(result: &ParseResult) -> &ModuleDeclaration {
    match &result.source.descriptions[0] {
        Description::Module(m) => m,
        _ => panic!("Expected module"),
    }
}

#[test]
fn test_lrm_6_2_data_types() {
    let res = parse_ok("module top; int a; logic b; bit c; endmodule");
    let m = first_module(&res);
    assert_eq!(m.items.len(), 3);
}

#[test]
fn test_lrm_11_4_concatenation() {
    let res =
        parse_ok("module top; logic [3:0] a, b; logic [7:0] c; initial c = {a, b}; endmodule");
    let m = first_module(&res);
    // 0: logic [3:0] a, b; 1: logic [7:0] c; 2: initial ...
    if let ModuleItem::InitialConstruct(ic) = &m.items[2] {
        if let StatementKind::BlockingAssign { rvalue, .. } = &ic.stmt.kind {
            assert!(matches!(rvalue.kind, ExprKind::Concatenation(_)));
        }
    }
}
