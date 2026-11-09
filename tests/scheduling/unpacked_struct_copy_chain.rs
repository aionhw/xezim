//! IEEE 1800-2023 §10.3, §9.2.2.2, §9.4.2.2 and §7.2: whole unpacked
//! struct reads depend on the member storage, including through continuous
//! copy chains. Sample after propagation (§4.7), and change each leaf alone
//! to check nested, integer and real dependencies independently.

use xezim::simulate;

const TYPES: &str = r#"
typedef struct { real r; int a; } inner_t;
typedef struct { inner_t inner; real r; } s_t;
"#;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

fn chain(depth: usize) -> String {
    let mut src = String::from(
        "int a; real r, outer; s_t p;\n\
         assign p = '{inner: '{a: a, r: r}, r: outer};\n",
    );
    let mut prev = "p".to_string();
    for n in 0..depth {
        src.push_str(&format!("s_t q{n}; assign q{n} = {prev};\n"));
        prev = format!("q{n}");
    }
    src
}

fn display(name: &str) -> String {
    format!("$display(\"R %.2f %0d %.1f\", {name}.inner.r, {name}.inner.a, {name}.r);")
}

fn stimulus(scope: &str, probes: &[String]) -> String {
    let probes = probes.join("\n");
    format!(
        "initial begin
           #1 {scope}a = 3; {scope}r = 0.75; {scope}outer = 4.5;
           #1 {probes}
           {scope}r = 1.25;
           #1 {probes}
           {scope}a = 7;
           #1 {probes}
           {scope}outer = -2.5;
           #1 {probes}
         end"
    )
}

fn expected(copies: usize) -> Vec<String> {
    [
        "R 0.75 3 4.5",
        "R 1.25 3 4.5",
        "R 1.25 7 4.5",
        "R 1.25 7 -2.5",
    ]
    .into_iter()
    .flat_map(|line| std::iter::repeat_n(line.to_string(), copies))
    .collect()
}

#[test]
fn comb_and_star_follow_continuous_copy_chains() {
    for depth in [1, 2, 3] {
        let tail = format!("q{}", depth - 1);
        let src = format!(
            "{TYPES}
             module top;
               {}
               s_t c, s;
               always_comb c = {tail};
               always @* s = {tail};
               {}
             endmodule",
            chain(depth),
            stimulus(
                "",
                &[display("p"), display(&tail), display("c"), display("s")]
            ),
        );
        assert_eq!(lines(&src), expected(4), "copy depth {depth}");
    }
}

#[test]
fn instance_copy_chains_follow_member_storage() {
    for depth in [2, 3] {
        let tail = format!("q{}", depth - 1);
        let src = format!(
            "{TYPES}
             module ch;
               {}
               s_t c, s;
               always_comb c = {tail};
               always @* s = {tail};
               {}
             endmodule
             module top;
               ch u();
             endmodule",
            chain(depth),
            stimulus("", &[display(&tail), display("c"), display("s")]),
        );
        assert_eq!(lines(&src), expected(3), "instance copy depth {depth}");
    }
}

/// §23.3.3: an output connection adds another whole-struct copy between
/// the child's chain and the parent's implicit sensitivity and compiled read.
#[test]
fn output_port_copy_chain_refires_parent_comb() {
    for depth in [2, 3] {
        let tail = format!("q{}", depth - 1);
        let src = format!(
            "{TYPES}
             module ch(output s_t o);
               {}
               assign o = {tail};
             endmodule
             module top;
               s_t q, c, s;
               ch u(.o(q));
               always_comb c = q;
               always @* s = q;
               {}
             endmodule",
            chain(depth),
            stimulus("u.", &[display("q"), display("c"), display("s")]),
        );
        assert_eq!(lines(&src), expected(3), "output copy depth {depth}");
    }
}

/// §9.2.2.4 / §9.4.2 / §10.4.2: clocked reads provide a control independent
/// of implicit sensitivity. Both the NBA copy and an initial copy read the
/// chain's settled value, including after a real-only source change.
#[test]
fn clocked_and_initial_copies_read_continuous_chain() {
    for depth in [2, 3] {
        let tail = format!("q{}", depth - 1);
        let src = format!(
            "{TYPES}
             module top;
               {}
               bit clk;
               s_t f, e, i;
               always_ff @(posedge clk) f <= {tail};
               always @(posedge clk) e <= {tail};
               initial begin
                 #1 a = 3; r = 0.75; outer = 4.5;
                 #1 clk = 1; i = {tail};
                 #1 {} {} {}
                 clk = 0; r = 1.25;
                 #1 clk = 1; i = {tail};
                 #1 {} {} {}
               end
             endmodule",
            chain(depth),
            display("f"),
            display("e"),
            display("i"),
            display("f"),
            display("e"),
            display("i"),
        );
        assert_eq!(lines(&src), expected(3)[..6], "clocked copy depth {depth}");
    }
}
