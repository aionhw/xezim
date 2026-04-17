use std::env;
use std::path::Path;

fn print_usage() {
    eprintln!("Usage: sisvsim [mode] [options] <source_files> [plusargs]");
    eprintln!("Modes (pick one; default is 'simulate'):");
    eprintln!("  --parse          Lex + parse only, report diagnostics");
    eprintln!("  --compile        Parse + elaborate, report diagnostics (no simulation)");
    eprintln!("  --simulate       Parse + elaborate + simulate (default)");
    eprintln!("Options:");
    eprintln!("  -v               Verbose output");
    eprintln!("  -V               Print version and exit");
    eprintln!("  -I <dir>         Add directory to include search path");
    eprintln!("  -D <name>[=val]  Define a macro");
    eprintln!("  -s <topmodule>   Specify the top-level module to elaborate");
    eprintln!("  --no-sim         Alias for --compile (deprecated)");
    eprintln!("  --dump-tokens    With --parse, print the token stream");
    eprintln!("  --dump-ast       With --parse, print the AST");
    eprintln!("  --max-time <n>   Set maximum simulation time (default: 100000)");
    eprintln!("  --sim_debug      Enable simulator [DEBUG]/[OPT] output");
    eprintln!("  --dpi-lib <so>   Load a DPI shared library (.so/.dylib/.dll)");
    eprintln!("Compatibility:");
    eprintln!("  -Ifoo, -DNAME=V  Accepted");
    eprintln!("  +incdir+dir1+dir2 / +define+FOO=1+BAR Accepted");
    eprintln!("  +NAME / +NAME=VALUE passed to $test$plusargs/$value$plusargs");
    eprintln!("  -f/-c filelist   Recursive; options inside filelist are supported");
}

fn print_version() {
    println!("sisvsim version 0.1.0");
}

fn push_define_token(tok: &str, defines: &mut Vec<(String, Option<String>)>) {
    if tok.is_empty() {
        return;
    }
    if let Some(pos) = tok.find('=') {
        defines.push((tok[..pos].to_string(), Some(tok[pos + 1..].to_string())));
    } else {
        defines.push((tok.to_string(), None));
    }
}

fn push_plus_incdir(arg: &str, include_dirs: &mut Vec<String>, lib_dirs: &mut Vec<String>) {
    if !arg.starts_with("+incdir+") {
        return;
    }
    let payload = &arg[8..];
    for dir in payload.split('+').filter(|s| !s.is_empty()) {
        include_dirs.push(dir.to_string());
        lib_dirs.push(dir.to_string());
    }
}

fn push_plus_define(arg: &str, defines: &mut Vec<(String, Option<String>)>) {
    if !arg.starts_with("+define+") {
        return;
    }
    let payload = &arg[8..];
    for d in payload.split('+').filter(|s| !s.is_empty()) {
        push_define_token(d, defines);
    }
}

fn resolve_rel(base: &Path, p: &str) -> String {
    let pp = Path::new(p);
    if pp.is_absolute() {
        p.to_string()
    } else if pp.exists() {
        p.to_string()
    } else {
        base.join(pp).to_string_lossy().to_string()
    }
}

fn split_filelist_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if ch.is_whitespace() {
                    if !cur.is_empty() {
                        out.push(cur.clone());
                        cur.clear();
                    }
                } else {
                    cur.push(ch);
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn process_command_file(
    path: &str,
    source_files: &mut Vec<String>,
    include_dirs: &mut Vec<String>,
    defines: &mut Vec<(String, Option<String>)>,
    lib_dirs: &mut Vec<String>,
    plusargs: &mut Vec<String>,
) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read command file '{}': {}", path, e))?;
    let base = Path::new(path).parent().unwrap_or_else(|| Path::new("."));

    for raw in content.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        if let Some((prefix, _)) = line.split_once("//") {
            line = prefix.trim();
            if line.is_empty() {
                continue;
            }
        }
        let toks = split_filelist_line(line);
        if toks.is_empty() {
            continue;
        }

        let mut i = 0usize;
        while i < toks.len() {
            let t = toks[i].as_str();
            match t {
                "-I" => {
                    i += 1;
                    if i < toks.len() {
                        include_dirs.push(resolve_rel(base, &toks[i]));
                    }
                }
                "-D" => {
                    i += 1;
                    if i < toks.len() {
                        push_define_token(&toks[i], defines);
                    }
                }
                "-y" | "--lib" => {
                    i += 1;
                    if i < toks.len() {
                        let d = resolve_rel(base, &toks[i]);
                        lib_dirs.push(d.clone());
                        include_dirs.push(d);
                    }
                }
                "-f" | "-c" => {
                    i += 1;
                    if i < toks.len() {
                        let nested = resolve_rel(base, &toks[i]);
                        process_command_file(&nested, source_files, include_dirs, defines, lib_dirs, plusargs)?;
                    }
                }
                _ if t.starts_with("-I") && t.len() > 2 => {
                    include_dirs.push(resolve_rel(base, &t[2..]));
                }
                _ if t.starts_with("-D") && t.len() > 2 => {
                    push_define_token(&t[2..], defines);
                }
                _ if t.starts_with("-y") && t.len() > 2 => {
                    let d = resolve_rel(base, &t[2..]);
                    lib_dirs.push(d.clone());
                    include_dirs.push(d);
                }
                _ if t.starts_with("-f") && t.len() > 2 => {
                    let nested = resolve_rel(base, &t[2..]);
                    process_command_file(&nested, source_files, include_dirs, defines, lib_dirs, plusargs)?;
                }
                _ if t.starts_with("+incdir+") => {
                    push_plus_incdir(t, include_dirs, lib_dirs);
                }
                _ if t.starts_with("+define+") => {
                    push_plus_define(t, defines);
                }
                _ if t.starts_with('+') => {
                    plusargs.push(t.to_string());
                }
                _ if t.starts_with('-') => {}
                _ => {
                    source_files.push(resolve_rel(base, t));
                }
            }
            i += 1;
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let mut source_files: Vec<String> = Vec::new();
    let mut top_module: Option<String> = None;
    let mut max_time: u64 = 100_000;
    let mut dump_tokens = false;
    let mut dump_ast = false;
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode { Parse, Compile, Simulate }
    let mut mode: Mode = Mode::Simulate;
    let mut mode_explicit = false;
    let mut verbose = false;
    let mut _output_file: Option<String> = None;
    let mut lib_dirs: Vec<String> = Vec::new();
    let mut log_file: Option<String> = None;
    let mut settle_limit: Option<u32> = None;
    let mut activity_mon = false;
    let mut sdf_file: Option<String> = None;
    let mut sdf_select: Option<sisvsim::compiler::sdf::DelaySelect> = None;
    let mut aitrace = false;
    let mut sim_debug = false;
    let mut dpi_libs: Vec<String> = Vec::new();
    let mut plusargs: Vec<String> = Vec::new();
    let mut threads: usize = 1;

    let mut include_dirs: Vec<String> = Vec::new();
    let mut defines: Vec<(String, Option<String>)> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => { print_usage(); std::process::exit(0); }
            "-I" => {
                i += 1;
                if i < args.len() { include_dirs.push(args[i].clone()); }
            }
            _ if arg.starts_with("-I") && arg.len() > 2 => {
                include_dirs.push(arg[2..].to_string());
            }
            "-D" => {
                i += 1;
                if i < args.len() {
                    push_define_token(&args[i], &mut defines);
                }
            }
            _ if arg.starts_with("-D") && arg.len() > 2 => {
                push_define_token(&arg[2..], &mut defines);
            }
            "-o" => {
                i += 1;
                if i < args.len() { _output_file = Some(args[i].clone()); }
            }
            _ if arg.starts_with("-o") && arg.len() > 2 => {
                _output_file = Some(arg[2..].to_string());
            }
            "-l" => {
                i += 1;
                if i < args.len() { log_file = Some(args[i].clone()); }
            }
            _ if arg.starts_with("-l") && arg.len() > 2 => {
                log_file = Some(arg[2..].to_string());
            }
            "-s" => {
                i += 1;
                if i < args.len() { top_module = Some(args[i].clone()); }
            }
            _ if arg.starts_with("-s") && arg.len() > 2 => {
                top_module = Some(arg[2..].to_string());
            }
            "-c" | "-f" => {
                i += 1;
                if i < args.len() {
                    match process_command_file(&args[i], &mut source_files, &mut include_dirs, &mut defines, &mut lib_dirs, &mut plusargs) {
                        Ok(()) => {}
                        Err(e) => { eprintln!("{}", e); std::process::exit(1); }
                    }
                }
            }
            _ if arg.starts_with("-f") && arg.len() > 2 => {
                match process_command_file(&arg[2..], &mut source_files, &mut include_dirs, &mut defines, &mut lib_dirs, &mut plusargs) {
                    Ok(()) => {}
                    Err(e) => { eprintln!("{}", e); std::process::exit(1); }
                }
            }
            "-y" => {
                i += 1;
                if i < args.len() { lib_dirs.push(args[i].clone()); include_dirs.push(args[i].clone()); }
            }
            _ if arg.starts_with("-y") && arg.len() > 2 => {
                lib_dirs.push(arg[2..].to_string());
                include_dirs.push(arg[2..].to_string());
            }
            "--lib" => {
                i += 1;
                if i < args.len() { lib_dirs.push(args[i].clone()); include_dirs.push(args[i].clone()); }
            }
            _ if arg.starts_with("+incdir+") => {
                push_plus_incdir(arg, &mut include_dirs, &mut lib_dirs);
            }
            _ if arg.starts_with("+define+") => {
                push_plus_define(arg, &mut defines);
            }
            _ if arg.starts_with('+') => {
                plusargs.push(arg.clone());
            }
            "-v" => { verbose = true; }
            "-V" => { print_version(); std::process::exit(0); }
            "--parse" => {
                if mode_explicit && mode != Mode::Parse {
                    eprintln!("Error: --parse conflicts with previously set mode"); std::process::exit(1);
                }
                mode = Mode::Parse; mode_explicit = true;
            }
            "--compile" | "--no-sim" => {
                if mode_explicit && mode != Mode::Compile {
                    eprintln!("Error: --compile conflicts with previously set mode"); std::process::exit(1);
                }
                mode = Mode::Compile; mode_explicit = true;
            }
            "--simulate" => {
                if mode_explicit && mode != Mode::Simulate {
                    eprintln!("Error: --simulate conflicts with previously set mode"); std::process::exit(1);
                }
                mode = Mode::Simulate; mode_explicit = true;
            }
            "--dump-tokens" => { dump_tokens = true; if !mode_explicit { mode = Mode::Parse; } }
            "--dump-ast" => { dump_ast = true; if !mode_explicit { mode = Mode::Parse; } }
            "--max-time" => {
                i += 1;
                if i < args.len() {
                    max_time = args[i].parse().unwrap_or(100_000);
                }
            }
            "--settle-limit" => {
                i += 1;
                if i < args.len() {
                    settle_limit = Some(args[i].parse().unwrap_or(100));
                }
            }
            "--activity-mon" => { activity_mon = true; }
            "--sdf" => {
                i += 1;
                if i < args.len() { sdf_file = Some(args[i].clone()); }
            }
            "--sdf-min" => { sdf_select = Some(sisvsim::compiler::sdf::DelaySelect::Min); }
            "--sdf-typ" => { sdf_select = Some(sisvsim::compiler::sdf::DelaySelect::Typ); }
            "--sdf-max" => { sdf_select = Some(sisvsim::compiler::sdf::DelaySelect::Max); }
            "--aitrace" => { aitrace = true; }
            "--sim_debug" => { sim_debug = true; }
            "--threads" => {
                i += 1;
                if i < args.len() {
                    threads = args[i].parse().unwrap_or(1).max(1);
                }
            }
            _ if arg.starts_with("--threads=") => {
                threads = arg["--threads=".len()..].parse().unwrap_or(1).max(1);
            }
            "--dpi-lib" => {
                i += 1;
                if i < args.len() { dpi_libs.push(args[i].clone()); }
            }
            _ if arg.starts_with('-') => {
                eprintln!("Warning: unknown flag '{}' (ignored)", arg);
            }
            _ => {
                source_files.push(arg.clone());
            }
        }
        i += 1;
    }

    if source_files.is_empty() {
        eprintln!("Error: no source files specified");
        print_usage();
        std::process::exit(1);
    }

    if let Some(ref path) = log_file {
        if let Err(e) = sisvsim::set_log_file(path) {
            eprintln!("Error: cannot open log file '{}': {}", path, e);
            std::process::exit(1);
        }
    }

    // Fast path: if the only source file is a sisvsim compiled artifact, load
    // it and jump straight to simulation (skip parse + elaborate).
    if source_files.len() == 1 && mode == Mode::Simulate {
        let sf = &source_files[0];
        if let Ok(head) = std::fs::read(sf).as_ref().map(|v| v.iter().take(8).copied().collect::<Vec<u8>>()) {
            if head.len() == 8 && &head[..] == sisvsim::SISVSIM_BYTECODE_MAGIC {
                match sisvsim::read_compiled(sf) {
                    Ok(Some(elab)) => {
                        println!("=== sisvsim ===");
                        println!("Loaded compiled: {}", sf);
                        println!("Max time: {}", max_time);
                        println!("------------------------------");
                        sisvsim::compiler::simulator::set_sim_debug(sim_debug);
                        sisvsim::compiler::simulator::set_dpi_libs(&dpi_libs);
                        let mut sim = sisvsim::compiler::Simulator::new(elab, max_time);
                        if let Some(limit) = settle_limit { sim.settle_limit = limit; }
                        sim.activity_mon = activity_mon;
                        sim.aitrace_mode = aitrace;
                        sim.set_plusargs(&plusargs);
                        sim.set_threads(threads);
                        sim.run();
                        println!("------------------------------");
                        println!("Simulation finished at time {}", sim.time);
                        if sim.finished { println!("($finish called)"); }
                        return;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("Error loading compiled artifact '{}': {}", sf, e);
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    let mut sources: Vec<String> = Vec::new();
    let mut file_labels: Vec<String> = Vec::new();
    for sf in &source_files {
        let path = Path::new(sf);
        if !path.exists() {
            eprintln!("Error: file '{}' not found", sf);
            std::process::exit(1);
        }
        match std::fs::read_to_string(path) {
            Ok(s) => {
                file_labels.push(sf.clone());
                sources.push(s);
            }
            Err(e) => {
                eprintln!("Error: cannot read '{}': {}", sf, e);
                std::process::exit(1);
            }
        }
    }

    if mode == Mode::Parse {
        if dump_tokens {
            for (_i, (label, source)) in file_labels.iter().zip(sources.iter()).enumerate() {
                println!("=== Tokens: {} ===", label);
                let tokens = sisvsim::tokenize_file(source, None);
                for tok in &tokens {
                    println!("{:?} '{}' @ {}..{}", tok.kind, tok.text, tok.span.start, tok.span.end);
                }
            }
        }
        let mut total_desc = 0;
        let mut total_err = 0;
        let mut total_warn = 0;
        for (label, source) in file_labels.iter().zip(sources.iter()) {
            let result = sv_parser::parse(source);
            for err in &result.errors {
                let (line, col) = byte_to_line_col(&result.source_text, err.span.start);
                eprintln!("[{}] {}:{}: error: {}", label, line, col, err.message);
            }
            total_desc += result.source.descriptions.len();
            total_err += result.errors.len();
            total_warn += result.warnings.len();
            if dump_ast {
                println!("=== AST: {} ===", label);
                println!("{:#?}", result.source);
            }
        }
        println!("Parsed {} file(s): {} descriptions, {} errors, {} warnings",
            sources.len(), total_desc, total_err, total_warn);
        if total_err > 0 { std::process::exit(1); }
        return;
    }

    if mode == Mode::Compile {
        let mut total_desc = 0;
        let mut total_err = 0;
        let mut total_warn = 0;

        for (label, source) in file_labels.iter().zip(sources.iter()) {
            let result = sv_parser::parse(source);
            for err in &result.errors {
                let (line, col) = byte_to_line_col(&result.source_text, err.span.start);
                eprintln!("[{}] {}:{}: error: {}", label, line, col, err.message);
            }
            total_desc += result.source.descriptions.len();
            total_err += result.errors.len();
            total_warn += result.warnings.len();
        }
        println!("Parsed {} file(s): {} descriptions, {} errors, {} warnings",
            sources.len(), total_desc, total_err, total_warn);
        if total_err > 0 { std::process::exit(1); }

        match sisvsim::parse_and_elaborate_multi(&sources, top_module.as_deref(), &include_dirs, &source_files, &defines) {
            Ok((_defs, elab)) => {
                println!("Elaboration successful");
                if let Some(ref out) = _output_file {
                    match sisvsim::write_compiled(&elab, out) {
                        Ok(()) => println!("Wrote compiled artifact to {}", out),
                        Err(e) => { eprintln!("Error writing '{}': {}", out, e); std::process::exit(1); }
                    }
                }
            }
            Err(e) => {
                eprintln!("Simulation error: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    println!("=== sisvsim ===");
    println!("Max time: {}", max_time);
    println!("------------------------------");
    sisvsim::compiler::simulator::set_sim_debug(sim_debug);
    sisvsim::compiler::simulator::set_dpi_libs(&dpi_libs);

    match sisvsim::simulate_multi(&sources, max_time, top_module.as_deref(), &include_dirs, &source_files, settle_limit, activity_mon, sdf_file.as_deref(), sdf_select, &defines, aitrace, &plusargs, threads) {
        Ok(sim) => {
            println!("------------------------------");
            println!("Simulation finished at time {}", sim.time);
            if sim.finished { println!("($finish called)"); }
        }
        Err(e) => {
            eprintln!("Simulation error: {}", e);
            std::process::exit(1);
        }
    }
}

fn byte_to_line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= byte_offset { break; }
        if ch == '\n' { line += 1; col = 1; }
        else { col += 1; }
    }
    (line, col)
}
