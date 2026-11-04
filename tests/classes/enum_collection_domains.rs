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

#[test]
fn dynamic_enum_draws_and_repairs_stay_in_domain() {
    check(include_str!("../fixtures/enum_collection_domain.sv"));
}

#[test]
fn queue_enum_draws_stay_in_domain_without_constraints() {
    check(
        &include_str!("../fixtures/enum_collection_domain.sv")
            .replace("resp[]", "resp[$]")
            .replace("resp.size()==8;", "")
            .replace(
                "txn t=new();",
                "txn t=new(); t.resp='{0,0,0,0,0,0,0,0}; t.c.constraint_mode(0);",
            )
            .replace("{0,2,3}", "{0,1,2,3}"),
    );
}

#[test]
fn associative_enum_draws_stay_in_domain() {
    check(
        &include_str!("../fixtures/enum_collection_domain.sv")
            .replace("resp[]", "resp[int]")
            .replace("resp.size()==8;", "")
            .replace(
                "txn t=new();",
                "txn t=new(); t.resp[4]=txn::OKAY; t.resp[12]=txn::OKAY;",
            ),
    );
}

#[test]
fn illegal_dynamic_enum_constraint_fails_and_preserves_values() {
    check(
        r#"class txn;
      typedef enum bit[3:0] {A=0,B=2,C=3} response;
      rand response r[];
      constraint c {r.size()==1; r[0]==9;}
    endclass
    module top;
     initial begin
      txn t=new(); t.r='{txn::B};
      if(t.randomize()) $fatal(1,"illegal enum accepted");
      if(t.r.size()!=1 || t.r[0]!=2) $fatal(1,"failed solve changed array");
      $display("PASS");$finish;
     end
    endmodule"#,
    );
}
