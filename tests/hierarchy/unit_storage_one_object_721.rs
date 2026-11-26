//! #721, IEEE 1800-2023 §3.12.1: a compilation-unit variable or net is one
//! object for the whole compilation unit. A write from any module, interface,
//! program, unit subroutine or unit class is seen by every other reader, a
//! module's own declaration of the name shadows it and `$unit::name` still
//! names it. Expected lines are the reference simulator's transcripts; lines
//! of one time step are compared as a set where processes race.
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "xezim-unit-one-object-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        for (name, source) in files {
            std::fs::write(path.join(name), source).unwrap();
        }
        Self(path)
    }
    fn output(&self, args: &[&str], env: &[(&str, &str)]) -> (bool, String, Vec<String>) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command.args(args).arg("--error-exit").current_dir(&self.0);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("XEZIM_") {
                command.env_remove(name);
            }
        }
        command.envs(env.iter().copied());
        let out = command.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let lines = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| line.starts_with("T|"))
            .map(str::to_owned)
            .collect();
        (out.status.success(), text, lines)
    }
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> Vec<String> {
        let (ok, text, lines) = self.output(args, env);
        assert!(ok, "{args:?} {env:?}\n{text}");
        // No reference to unit storage may fall back to an implicit net.
        assert!(!text.contains("implicit 1-bit net"), "{args:?}\n{text}");
        lines
    }
    /// Uncached, interpreted methods, cached twice, and a compiled artifact.
    fn check_all_paths(&self, design: &[&str], expected: &str, ordered: bool) {
        let norm = |mut lines: Vec<String>| {
            if !ordered {
                lines.sort();
            }
            lines
        };
        let expected = norm(expected.lines().map(str::to_owned).collect());
        let with = |head: &[&str]| -> Vec<String> {
            head.iter().chain(design).map(|s| s.to_string()).collect()
        };
        let run = |args: Vec<String>, env: &[(&str, &str)]| {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            norm(self.run(&args, env))
        };
        assert_eq!(run(with(&["--no-cache"]), &[]), expected);
        assert_eq!(
            run(with(&["--no-cache"]), &[("XEZIM_COMPILE_METHODS", "0")]),
            expected
        );
        for _ in 0..2 {
            assert_eq!(run(with(&["--cache-dir", "cache"]), &[]), expected);
        }
        assert!(run(with(&["--compile", "--no-cache", "-o", "design.xez"]), &[]).is_empty());
        assert_eq!(
            run(vec!["--simulate".into(), "design.xez".into()], &[]),
            expected
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

macro_rules! one_object {
    ($name:ident, $file:literal, $ordered:expr, $doc:literal) => {
        #[doc = $doc]
        #[test]
        fn $name() {
            Fixture::new(&[(
                "design.sv",
                include_str!(concat!("unit_one_object_721/", $file, ".sv")),
            )])
            .check_all_paths(
                &["-s", "top", "design.sv"],
                include_str!(concat!("unit_one_object_721/", $file, ".ref")),
                $ordered,
            );
        }
    };
}

one_object!(
    writes_reach_every_reader,
    "identity",
    true,
    "§3.12.1: module, unit function and unit class writes reach every reader, `always @` wakes, and `$unit::` passes a module's own declaration."
);
one_object!(
    every_data_type_is_shared,
    "types",
    false,
    "§3.12.1 / §6.8: scalars, vectors, reals, strings, enums, structs, arrays, queues, class handles and events, with initializers over other unit variables and a unit function."
);
one_object!(
    unit_nets_resolve_drivers_of_several_modules,
    "nets",
    true,
    "§3.12.1 / §6.6: a unit net with an initializer, a driver in one module, drivers in two modules, tri, wand and supply nets."
);
one_object!(
    module_block_and_argument_declarations_shadow,
    "shadow",
    true,
    "§3.12.1 / §23.9: a module, block, generate block or formal of the same name is another object; unit subroutines and unit classes called from there still write the unit's."
);
one_object!(
    root_declaration_shadows,
    "topsh",
    false,
    "§3.12.1: the top module declares the name itself; children, a unit function and a unit class name the unit's object."
);
one_object!(
    generate_blocks_in_root_and_child,
    "topgen",
    true,
    "§3.12.1 / §27: references from generate blocks of the top and of a child."
);
one_object!(
    contested_aggregates,
    "misc",
    true,
    "§3.12.1: queues, associative and dynamic arrays, structs, class handles, strings, reals, events and multidimensional arrays that another module also declares."
);
one_object!(
    continuous_assigns_read_unit_storage,
    "ca",
    true,
    "§3.12.1 / §10.3: continuous assigns of a child read the unit's variable and net, qualified or not, and declare no implicit net."
);

/// §3.12.1, §23.3.1: two top modules, an interface, a program, an instance
/// three levels down, an instance array and a generate loop share the unit's
/// variable and queue through a unit task.
#[test]
fn several_tops_programs_and_interfaces_share() {
    Fixture::new(&[("design.sv", include_str!("unit_one_object_721/struct.sv"))]).check_all_paths(
        &["design.sv"],
        include_str!("unit_one_object_721/struct.ref"),
        true,
    );
}

/// §3.12.1, §6.8: two source files form one compilation unit; a later
/// file's initializer reads the earlier file's variables.
#[test]
fn two_files_one_unit() {
    Fixture::new(&[
        ("a.sv", include_str!("unit_one_object_721/init1.sv")),
        ("b.sv", include_str!("unit_one_object_721/init2.sv")),
    ])
    .check_all_paths(
        &["-s", "top", "a.sv", "b.sv"],
        include_str!("unit_one_object_721/init.ref"),
        true,
    );
}

/// §3.12.1: library modules (`-v`, `-y`) name the unit's variable and call
/// its task.
#[test]
fn library_modules_share() {
    let fixture = Fixture::new(&[
        ("design.sv", include_str!("unit_one_object_721/lib.sv")),
        ("libmod.v", include_str!("unit_one_object_721/libmod.v")),
    ]);
    std::fs::create_dir_all(fixture.0.join("libdir")).unwrap();
    std::fs::write(
        fixture.0.join("libdir/liby.sv"),
        include_str!("unit_one_object_721/liby.sv"),
    )
    .unwrap();
    fixture.check_all_paths(
        &[
            "-s",
            "top",
            "-v",
            "libmod.v",
            "-y",
            "libdir",
            "+libext+.sv",
            "design.sv",
        ],
        include_str!("unit_one_object_721/lib.ref"),
        true,
    );
}

/// §21.7: a unit variable is dumped once, in the top module's scope, and
/// no instance carries a copy.
#[test]
fn dumped_once_in_the_top_scope() {
    let fixture = Fixture::new(&[(
        "design.sv",
        "int g = 5;\nwire [3:0] un = g[3:0];\nmodule child; int loc; initial begin #1; g = 9; loc = g; end endmodule\nmodule top; child c();\n  initial begin $dumpfile(\"d.vcd\"); $dumpvars(0, top); #3 $finish; end\nendmodule\n",
    )]);
    fixture.run(&["--no-cache", "--wave", "-s", "top", "design.sv"], &[]);
    let vcd = std::fs::read_to_string(fixture.0.join("d.vcd")).unwrap();
    let vars: Vec<&str> = vcd
        .lines()
        .filter(|l| l.starts_with("$scope") || l.starts_with("$var") || l.starts_with("$upscope"))
        .map(|l| {
            l.split_whitespace()
                .nth(if l.starts_with("$var") { 4 } else { 2 })
                .unwrap_or("")
        })
        .collect();
    assert_eq!(vars, ["top", "g", "un", "c", "loc", "", ""], "{vcd}");
}

/// §21.8 checkpoint: the unit's storage and its shadow in the top module
/// are saved and restored.
#[test]
fn checkpoint_round_trip() {
    let fixture = Fixture::new(&[(
        "design.sv",
        include_str!("unit_one_object_721/checkpoint.sv"),
    )]);
    let expected = ["T|g=28 ug=14 q='{6, 7, 8, 9, 10, 11, 12, 13, 14} s=axxxxxxxxx"];
    let base = ["--no-cache", "-s", "top"];
    assert_eq!(
        fixture.run(&[&base[..], &["design.sv"]].concat(), &[]),
        expected
    );
    assert_eq!(
        fixture.run(
            &[&base[..], &["--save-at", "5ns,ck.ckpt", "design.sv"]].concat(),
            &[]
        ),
        expected
    );
    assert_eq!(
        fixture.run(
            &[&base[..], &["--restore", "ck.ckpt", "design.sv"]].concat(),
            &[]
        ),
        expected
    );
}
