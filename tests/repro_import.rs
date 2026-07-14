use xezim::simulate;

#[test]
fn test_repro_import_wildcard() {
    let src = r#"
        package p;
            parameter A = 1;
            parameter B = 2;
        endpackage
        module top;
            import p::*;
            initial begin
                $display("A=%0d B=%0d", A, B);
                $finish;
            end
        endmodule
    "#;
    let sim = simulate(src, 1000).unwrap();
    assert!(sim.output[0].message.contains("A=1 B=2"));
}

#[test]
fn test_repro_import_too_much() {
    let src = r#"
        package p;
            parameter A = 1;
            parameter B = 2;
        endpackage
        module top;
            import p::A;
            initial begin
                $display("B=%0d", B); // Should fail to elaborate because B is not imported
                $finish;
            end
        endmodule
    "#;
    let result = simulate(src, 1000);
    assert!(result.is_err(), "Should fail because B is not imported");
}

#[test]
fn test_repro_import_specific_missing() {
    let src = r#"
        package p;
            parameter A = 1;
        endpackage
        module top;
            import p::C; // C does not exist in p
            initial begin
                $finish;
            end
        endmodule
    "#;
    let result = simulate(src, 1000);
    match result {
        Err(e) => assert!(e.contains("Symbol 'C' not found in package 'p'")),
        Ok(_) => panic!("Should have failed because C is not in package p"),
    }
}
