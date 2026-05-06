use xezim::ast::*;
use xezim::*;

#[test]
fn test_inspect_types() {
    let res = parse_str("module top; int a; logic [7:0] b; endmodule").unwrap();
    assert_eq!(res.source.descriptions.len(), 1);
    for d in res.source.descriptions {
        if let Description::Module(m) = d {
            assert_eq!(m.name.name, "top");
        }
    }
}
