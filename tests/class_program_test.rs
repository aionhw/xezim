use xezim::*;

#[test]
fn test_parse_class_program() {
    let source = r#"
        class MyClass;
            int x;
        endclass
        program my_prog;
            initial $display("hi");
        endprogram
    "#;
    let res = parse_str(source).unwrap();
    assert_eq!(res.source.descriptions.len(), 2);
}
