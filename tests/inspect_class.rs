use xezim::ast::*;
use xezim::*;

#[test]
fn test_class_items() {
    let res = parse_str("class C; int x; endclass").unwrap();
    if let Description::Class(c) = &res.source.descriptions[0] {
        assert_eq!(c.name.name, "C");
    }
}
