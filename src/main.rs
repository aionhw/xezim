//! sisvsim: CLI tool for parsing and simulating SystemVerilog files.
//! Supports iverilog-compatible command-line arguments.

use std::env;
use std::path::Path;
use sisvsim::diagnostics::format_diagnostic;

fn print_usage() {
    eprintln!("Usage: sisvsim [options] sourcefile ...");
    eprintln!();
    eprintln!("A SystemVerilog simulator compatible with common iverilog flags.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -o <file>        Place output in <file> (not yet used)");
    eprintln!("  -s <topmodule>   Specify the top-level module to elaborate");
    eprintln!("  -c <cmdfile>     Read source file list from <cmdfile>");
    eprintln!("  -f <cmdfile>     Same as -c");
    eprintln!("  -I <dir>         Add include directory (not yet used)");
    eprintln!("  -D <macro>[=val] Define preprocessor macro (not yet used)");
    eprintln!("  -g <spec>        Language generation (e.g. -g2012, ignored)");
    eprintln!("  -y <dir>         Library directory for module resolution");
    eprintln!("  --lib <dir>      Library directory for module resolution (same as -y)");
    eprintln!("  -v               Be verbose");
    eprintln!("  -V               Print version and exit");
    eprintln!("  -E               Preprocess only (not yet used)");
    eprintln!("  --sim            Run simulation (default when source files given)");
    eprintln!("  --no-sim         Parse only, do not simulate");
    eprintln!("  --dump-tokens    Print the token stream");
    eprintln!("  --dump-ast       Print the AST");
    eprintln!("  --max-time <N>   Set simulation time limit (default: 100000)");
    eprintln!("  --settle-limit <N> Combinatorial settle iteration limit (default: 100)");
    eprintln!("  --activity-mon     Show top-10 most triggered blocks and toggling signals");
    eprintln!("  -Wall            Enable all warnings");
    eprintln!("  -W <type>        Enable/disable warnings (ignored)");
}

fn print_version() {
    eprintln!("sisvsim 0.1.0 (SystemVerilog Simulator)");
    eprintln!("Targeting IEEE 1800-2017/2023");
}

/// Read a command file: one source file per line, # comments, +args ignored.
fn read_command_file(path: &str) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Error reading command file '{}': {}", path, e))?;
    let mut files = Vec::new();
    let mut in_block_comment = false;
    for line in content.lines() {
        let line = line.trim();
        // Block comment handling
        if in_block_comment {
            if let Some(idx) = line.find("*/") {
                let remainder = line[idx + 2..].trim();
                in_block_comment = false;
                if !remainder.is_empty() && !remainder.starts_with('#') && !remainder.starts_with('+') && !remainder.starts_with("//") {
                    files.push(remainder.to_string());
                }
            }
            continue;
        }
        if let Some(idx) = line.find("/*") {
            in_block_comment = true;
            let before = line[..idx].trim();
            if !before.is_empty() && !before.starts_with('#') && !before.starts_with('+') {
                files.push(before.to_string());
            }
            if line[idx + 2..].find("*/").is_some() {
                in_block_comment = false;
            }
            continue;
        }
        // Line comment
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(idx) = line.find("//") {
            let before = line[..idx].trim();
            if !before.is_empty() && !before.starts_with('+') {
                files.push(before.to_string());
            }
            continue;
        }
        // +args: compiler arguments, skip
        if line.starts_with('+') { continue; }
        // -flags inside command files
        if line.starts_with('-') { continue; }
        // Source file
        files.push(line.to_string());
    }
    Ok(files)
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
    let mut no_sim = false;
    let mut verbose = false;
    let mut _output_file: Option<String> = None;
    let mut lib_dirs: Vec<String> = Vec::new();
    let mut settle_limit: Option<u32> = None;
    let mut activity_mon = false;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            // iverilog-compatible flags
            "-o" => {
                i += 1;
                if i < args.len() { _output_file = Some(args[i].clone()); }
            }
            "-s" => {
                i += 1;
                if i < args.len() { top_module = Some(args[i].clone()); }
            }
            "-c" | "-f" => {
                i += 1;
                if i < args.len() {
                    match read_command_file(&args[i]) {
                        Ok(files) => source_files.extend(files),
                        Err(e) => { eprintln!("{}", e); std::process::exit(1); }
                    }
                }
            }
            "-g" => {
                // -g <generation> — accept and skip
                i += 1;
            }
            "-y" => {
                i += 1;
                if i < args.len() { lib_dirs.push(args[i].clone()); }
            }
            "--lib" => {
                i += 1;
                if i < args.len() { lib_dirs.push(args[i].clone()); }
            }
            "-v" => { verbose = true; }
            "-V" => { print_version(); std::process::exit(0); }
            "-E" => { /* preprocess only — not yet */ }
            "-Wall" => { /* accept and ignore */ }
            // sisvsim-specific flags
            "--sim" => { /* default behavior */ }
            "--no-sim" => { no_sim = true; }
            "--dump-tokens" => { dump_tokens = true; no_sim = true; }
            "--dump-ast" => { dump_ast = true; no_sim = true; }
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
            _ if arg.starts_with("-o") && arg.len() > 2 => {
                _output_file = Some(arg[2..].to_string());
            }
            _ if arg.starts_with("-s") && arg.len() > 2 => {
                top_module = Some(arg[2..].to_string());
            }
            _ if arg.starts_with("-c") && arg.len() > 2 => {
                match read_command_file(&arg[2..]) {
                    Ok(files) => source_files.extend(files),
                    Err(e) => { eprintln!("{}", e); std::process::exit(1); }
                }
            }
            _ if arg.starts_with("-f") && arg.len() > 2 => {
                match read_command_file(&arg[2..]) {
                    Ok(files) => source_files.extend(files),
                    Err(e) => { eprintln!("{}", e); std::process::exit(1); }
                }
            }
            _ if arg.starts_with("-g") => { /* -g2012, -g2005-sv, etc — ignore */ }
            _ if arg.starts_with("-I") => { /* include dir */ }
            _ if arg.starts_with("-D") => { /* define */ }
            _ if arg.starts_with("-W") => { /* warning flags */ }
            _ if arg.starts_with("-y") && arg.len() > 2 => { lib_dirs.push(arg[2..].to_string()); }
            _ if arg.starts_with("-y") => { /* -y with no arg, ignore */ }
            _ if arg.starts_with("-l") => { /* library file */ }
            _ if arg.starts_with("-t") => { /* target type */ }
            _ if arg.starts_with("-p") => { /* target flag */ }
            _ if arg.starts_with("-T") => { /* timing */ }
            _ if arg.starts_with("-B") => { /* tool path */ }
            _ if arg.starts_with("-N") => { /* netlist dump */ }
            _ if arg.starts_with("-M") => { /* dependency file */ }
            _ if arg.starts_with("-m") => { /* VPI module */ }
            _ if arg.starts_with("-L") => { /* VPI path */ }
            _ if arg.starts_with('-') => {
                eprintln!("Warning: unknown flag '{}' (ignored)", arg);
            }
            _ => {
                // Positional argument = source file
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

    // Read all source files
    let mut sources: Vec<String> = Vec::new();
    let mut file_labels: Vec<String> = Vec::new();
    for sf in &source_files {
        let path = Path::new(sf);
        if !path.exists() {
            eprintln!("Error: file '{}' not found", sf);
            std::process::exit(1);
        }
        if path.is_dir() {
            eprintln!("Error: '{}' is a directory, not a source file", sf);
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

    // Dump tokens mode
    if dump_tokens {
        for (label, source) in file_labels.iter().zip(sources.iter()) {
            println!("=== Tokens: {} ===", label);
            let tokens = sisvsim::tokenize(source);
            for tok in &tokens {
                println!("{:?} '{}' @ {}..{}", tok.kind, tok.text, tok.span.start, tok.span.end);
            }
        }
        return;
    }

    // Dump AST mode
    if dump_ast {
        for (label, source) in file_labels.iter().zip(sources.iter()) {
            println!("=== AST: {} ===", label);
            match sisvsim::parse_str(source) {
                Ok(result) => {
                    for diag in &result.diagnostics {
                        eprintln!("{}", format_diagnostic(source, diag));
                    }
                    println!("{:#?}", result.source_text);
                }
                Err(diags) => {
                    for diag in &diags { eprintln!("{}", format_diagnostic(source, diag)); }
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    // Parse-only mode
    if no_sim {
        let mut total_desc = 0;
        let mut total_err = 0;
        let mut total_warn = 0;
        for (label, source) in file_labels.iter().zip(sources.iter()) {
            match sisvsim::parse_str(source) {
                Ok(result) => {
                    for diag in &result.diagnostics {
                        eprintln!("[{}] {}", label, format_diagnostic(source, diag));
                    }
                    total_desc += result.source_text.descriptions.len();
                    total_err += result.diagnostics.iter()
                        .filter(|d| d.severity == sisvsim::diagnostics::Severity::Error).count();
                    total_warn += result.diagnostics.iter()
                        .filter(|d| d.severity == sisvsim::diagnostics::Severity::Warning).count();
                }
                Err(diags) => {
                    for diag in &diags { eprintln!("[{}] {}", label, format_diagnostic(source, diag)); }
                    std::process::exit(1);
                }
            }
        }
        println!("Parsed {} file(s): {} descriptions, {} errors, {} warnings",
            sources.len(), total_desc, total_err, total_warn);
        if total_err > 0 { std::process::exit(1); }
        return;
    }

    // Simulation mode (default)
    println!("=== sisvsim ===");
    if file_labels.len() == 1 {
        println!("File: {}", file_labels[0]);
    } else {
        println!("Files: {}", file_labels.join(", "));
    }
    if verbose {
        println!("Max time: {}", max_time);
        if let Some(ref t) = top_module { println!("Top module: {}", t); }
        if !lib_dirs.is_empty() { println!("Library dirs: {}", lib_dirs.join(", ")); }
    } else {
        println!("Max time: {}", max_time);
    }
    println!("------------------------------");

    match sisvsim::simulate_multi(&sources, max_time, top_module.as_deref(), &lib_dirs, &source_files, settle_limit, activity_mon) {
        Ok(sim) => {
            println!("------------------------------");
            println!("Simulation finished at time {}", sim.time);
            if sim.finished {
                println!("($finish called)");
            }
        }
        Err(e) => {
            eprintln!("Simulation error: {}", e);
            std::process::exit(1);
        }
    }
}
