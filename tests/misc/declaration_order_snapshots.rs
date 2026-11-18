//! IEEE 1800-2023 §6.5, §6.21, §13.5: declaration-order snapshots
//! preserve defaults, procedural shadows, blocks and generated scopes.

fn passes(source: &str, expected: &str) {
    let run = xezim::simulate(source, 20).expect("legal lexical scopes must run");
    assert!(
        run.output.iter().any(|line| line.message == expected),
        "{:?}",
        run.output
    );
}

fn rejects(source: &str) {
    let error = match xezim::simulate(source, 20) {
        Ok(run) => panic!("accepted a premature module-data use: {:?}", run.output),
        Err(error) => error,
    };
    assert!(
        error.contains("'later'") && error.contains("before its declaration"),
        "{error}"
    );
}

#[test]
fn defaults_and_local_shadows_keep_the_declaring_scope() {
    passes(
        r#"
        module top;
          int earlier = 7;
          function automatic int f(input int later = 3);
            int shadow;
            shadow = earlier + later;
            return shadow;
          endfunction
          function automatic int from_default(input int value = earlier);
            return value;
          endfunction
          int later = 100;
          int shadow = 200;
          initial begin
            if (f() != 10 || f(5) != 12 || from_default() != 7 ||
                earlier != 7 || later != 100 || shadow != 200)
              $fatal(1, "default or procedural shadow resolved incorrectly");
            $display("PASS declaration snapshots");
          end
        endmodule
    "#,
        "PASS declaration snapshots",
    );
}

#[test]
fn nested_blocks_and_generate_scopes_keep_separate_values() {
    passes(
        r#"
        module top;
          int earlier = 7;
          function automatic int f(input bit enabled = 1);
            int result;
            result = 0;
            begin : inner
              int later;
              later = 3;
              if (enabled && earlier) result = later;
            end
            return result + earlier;
          endfunction
          if (1) begin : generated
            int later = 4;
            initial if (later != 4 || earlier != 7)
              $fatal(1, "generated data resolved outside its scope");
          end
          int later = 100;
          initial begin
            if (f() != 10 || f(0) != 7 || later != 100)
              $fatal(1, "block or module data resolved incorrectly");
            $display("PASS block snapshots");
          end
        endmodule
    "#,
        "PASS block snapshots",
    );
}

#[test]
fn function_body_and_default_keep_their_source_order_snapshot() {
    for declaration in [
        "function automatic int f(); return later; endfunction",
        "function automatic int f(input int value = later); return value; endfunction",
    ] {
        rejects(&format!(
            "module top; {declaration} int later = 4; initial $display(f()); endmodule"
        ));
    }
}

#[test]
fn a_nested_block_shadow_does_not_hide_a_later_free_use() {
    rejects(
        r#"
        module top;
          function automatic int f();
            begin : inner
              int later;
              later = 1;
            end
            return later;
          endfunction
          int later = 4;
          initial $display(f());
        endmodule
    "#,
    );
}

#[test]
fn a_generate_declaration_does_not_satisfy_a_premature_module_use() {
    rejects(
        r#"
        module top;
          int earlier = 7;
          if (1) begin : generated
            int later;
            initial later = earlier;
          end
          initial later = 1;
          int later;
        endmodule
    "#,
    );
}
