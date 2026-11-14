//! Issue #325 and its neighbours: a class-scope reference `Scope::member`
//! whose prefix is a type parameter bound to a specialization, a typedef of
//! a specialization, a package-qualified or direct specialization, or the
//! class chain itself. IEEE 1800-2023 §13.5.1 (a call to a function with no
//! arguments may omit the parentheses), §8.23 (class scope resolution) and
//! §8.25 / §8.25.1 (parameterized classes, class-scope typedefs).
//!
//! Each group runs on every method-execution path (default, compiled on
//! first call, compiled after one call, interpreter only). The expected
//! `T|` lines are the reference simulator's output.

use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

const PATHS: &[(&str, &str)] = &[
    ("CLASS_SCOPE_SPEC_PATH", "default"),
    ("XEZIM_METHOD_TIER", "0"),
    ("XEZIM_METHOD_TIER", "1"),
    ("XEZIM_COMPILE_METHODS", "0"),
];

fn run_lines(file: &str, env: (&str, &str)) -> Vec<String> {
    let path = format!("{}/tests/classes/{}", env!("CARGO_MANIFEST_DIR"), file);
    let out = Command::new(xezim())
        .args(["--simulate", "--no-cache", "-s", "top", &path])
        .env(env.0, env.1)
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|") || l.starts_with("PASS") || l.starts_with("FAIL"))
        .map(str::to_string)
        .collect()
}

fn check(file: &str, want: &[&str]) {
    for &env in PATHS {
        let got = run_lines(file, env);
        assert_eq!(got, want, "{file} with {}={}", env.0, env.1);
    }
}

/// #325 (reported by Dragon-git): `Tregistry::type_name` with `Tregistry`
/// bound to `holder #(int, "base_test")` returned four spaces.
#[test]
fn issue325_bare_static_fn_through_type_param() {
    check(
        "class_scope_spec_issue325.sv",
        &[
            "PASS bare, param bound to parameterized class: 'base_test'",
            "PASS parenthesised, same binding: 'base_test'",
            "PASS bare, param bound to plain class: 'plain'",
        ],
    );
}

/// Type parameter, typedef, package and direct specialization prefixes.
#[test]
fn spec_scope_reached() {
    check(
        "class_scope_spec_reached.sv",
        &[
            "T|A01 param->spec virtual:base_test",
            "T|A02 param->spec static:st",
            "T|A03 param->default spec #():<unknown>",
            "T|A04 param->typedef spec:td",
            "T|A05 param->nested spec:out/in",
            "T|A06 param->plain:plain",
            "T|A07 param->pkg spec:pkq",
            "T|A08 param->value spec:70",
            "T|A09 param->value default:30",
            "T|A10 class-scope typedef:cs",
            "T|A14 pkg::spec direct:pkd",
            "T|A15 direct spec:x",
            "T|A16 direct typedef:td",
            "T|A17 direct default spec:<unknown>",
            "T|A18 derived::type_name bare:ext",
        ],
    );
}

/// Statics inherited from a specialization.
#[test]
fn spec_scope_inherited() {
    check(
        "class_scope_spec_inherited.sv",
        &[
            "T|A12p extends inherited paren (method):ext",
            "T|A18 derived::type_name bare:ext",
            "T|A18p derived::type_name():ext",
        ],
    );
}

/// Member kinds through a type parameter bound to a specialization.
#[test]
fn spec_scope_members() {
    check(
        "class_scope_spec_members.sv",
        &[
            "T|B01 bare string:mk",
            "T|B02 paren string:mk",
            "T|B03 bare int:106",
            "T|B04 bare enum:BLUE",
            "T|B05 bare enum cmp:1",
            "T|B06 bare handle:mk",
            "T|B07 chained get().name:mk",
            "T|B08 static prop:11",
            "T|B09 bare+1:107",
            "T|B10 concat:mk!",
            "T|B11 compare:1",
            "T|B12 sformatf arg:<mk>",
            "T|B13 user fn arg:[mk]",
            "T|B14 bare task stmt:12",
            "T|B15 paren task stmt:13",
            "T|B16 param P:6",
            "T|B17 localparam LP:12",
            "T|B18 typedef elem_t bits:8",
            "T|B20 cond bare:mk",
        ],
    );
}

/// Callers: virtual override, module, package function and class, constant.
#[test]
fn spec_scope_callers() {
    check(
        "class_scope_spec_callers.sv",
        &[
            "T|C01 virtual override:c:virt2:virt2",
            "T|C02 module initial typedef bare:mod",
            "T|C03 pkg function:pkfn",
            "T|C04 pkg class static:pkcls",
            "T|C05 const expr param:9",
        ],
    );
}

/// The same members reached directly and through a module typedef.
#[test]
fn spec_scope_direct() {
    check(
        "class_scope_spec_direct.sv",
        &[
            "T|D01 spec bare string:dd",
            "T|D02 spec bare int:106",
            "T|D03 spec static prop:11",
            "T|D04 spec param P:6",
            "T|D05 spec localparam LP:12",
            "T|D06 td bare string:dt",
            "T|D07 td bare int:106",
            "T|D08 td static prop:11",
            "T|D09 td param P:6",
            "T|D10 td localparam LP:12",
            "T|D11 td bare task:12",
            "T|D12 td paren task:13",
            "T|D13 td typedef elem_t bits:8",
            "T|D14 td paren string:dt",
        ],
    );
}

/// Package-qualified, defaulted and UVM-2017-registry shapes.
#[test]
fn spec_scope_neighbours() {
    check(
        "class_scope_spec_neighbours.sv",
        &[
            "T|E01 pkg spec P:9",
            "T|E02 pkg spec static prop:21",
            "T|E03 pkg typedef bare fn:pktd",
            "T|E04 pkg typedef P:8",
            "T|E05 type-param default spec:dflt",
            "T|E06 direct spec bare in method:inm",
            "T|E07 direct spec P in method:17",
            "T|E08 pkg spec bare in static method:inpk",
            "T|E09 uvm09 registry via common:uvm09",
            "T|E10 uvm09 registry via base handle:uvm09",
            "T|E11 pkg default spec bare:<pk>",
        ],
    );
}

/// Static members inherited from a specialization, called inside the derived class.
#[test]
fn spec_scope_extends() {
    check(
        "class_scope_spec_extends.sv",
        &[
            "T|F01 inherited paren in method:ext",
            "T|F02 this.f() in method:ext",
            "T|F03 derived::f() in method:ext",
            "T|F04 inherited paren in static method:ext",
            "T|F05 inherited int fn in method:9",
            "T|F06 inherited param P in method:9",
            "T|F07 inherited param P in static:9",
            "T|F08 super.f() in method:ext",
            "T|F09 two-level inherited:ext",
            "T|F10 handle.f():ext",
        ],
    );
}

/// Class-scope typedefs reached through a specialization.
#[test]
fn spec_scope_typedefs() {
    check(
        "class_scope_spec_typedefs.sv",
        &[
            "T|H01 module td::elem_t:8",
            "T|H02 module spec::elem_t:16",
            "T|H03 module td::vec_t:6",
            "T|H04 module spec::vec_t:11",
            "T|H05 class typedef R::elem_t (td):8",
            "T|H06 class typedef R::vec_t (td):6",
            "T|H07 class typedef R::elem_t (spec):64",
            "T|H08 class typedef R::vec_t (spec):3",
            "T|H09 method local td::elem_t:8",
            "T|H10 method local spec::elem_t:16",
            "T|H11 method local td::vec_t:6",
            "T|H12 class typedef R::elem_t signed (td):-1",
            "T|H13 class typedef R::elem_t signed (spec):-1",
        ],
    );
}

/// Localparams of a specialization read by static methods.
#[test]
fn spec_scope_localparams() {
    check(
        "class_scope_spec_localparams.sv",
        &["T|I01 9", "T|I02 9", "T|I03 9", "T|I04 10"],
    );
}

/// Bare no-argument static functions named inside the class chain.
#[test]
fn spec_scope_in_class() {
    check(
        "class_scope_spec_in_class.sv",
        &[
            "T|J01 plain bare static in method:42",
            "T|J02 plain bare static in static:43",
            "T|J03 spec bare static in method:7",
            "T|J04 spec bare static in static:7",
            "T|J05 inherited plain bare static in method:41",
        ],
    );
}
