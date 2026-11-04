//! IEEE 1800-2017 §33.4: a configuration declaration is read and set aside
//! (xezim takes its tops from `-s` and maps no libraries), with a warning.
//! It was a stream of "unexpected token" errors (#603, reported by
//! amal-khailtash).
use xezim::simulate;

#[test]
fn configuration_is_set_aside() {
    for src in [
        "module top;\n initial begin $display(\"RAN\"); $finish; end\nendmodule : top\n\
         config cfg;\n design work.top;\n default liblist work;\nendconfig\n",
        "config cfg;\n design work.top;\n instance top.u use work.m;\nendconfig : cfg\n\
         module top; initial begin $display(\"RAN\"); $finish; end endmodule\n",
    ] {
        let out: Vec<String> = simulate(src, 10)
            .expect("simulate")
            .output
            .iter()
            .map(|l| l.message.clone())
            .filter(|m| m == "RAN")
            .collect();
        assert_eq!(out, ["RAN"], "{src}");
    }
}
