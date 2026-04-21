//! xezim — SystemVerilog bytecode interpreter.
//!
//! Parsing, elaboration, and shared runtime primitives live in the
//! sibling `xezim-core` crate. This crate adds the event-driven
//! interpreter (`simulator`) and bytecode IR (`bytecode`).
//!
//! For ahead-of-time native compilation, use the `xezim-b` crate.

pub mod compiler;

// Re-export xezim-core surface so existing `xezim::...` paths keep working.
pub use xezim_core::{
    ast, diagnostics, lexer, parse, preprocessor, sv_parser, ParseResult, SourceDefinition,
    XEZIM_BYTECODE_MAGIC, log_eprintln, log_println, parse_and_elaborate_multi, parse_str,
    read_compiled, set_log_file, tokenize_file, write_compiled,
};

/// Simulate a single source string.
pub fn simulate(source: &str, max_time: u64) -> Result<compiler::Simulator, String> {
    simulate_multi(&[source.to_string()], max_time, None, &[], &[], None, false, None, None, &[], false, &[], 1)
}

pub fn simulate_multi(
    sources: &[String], max_time: u64, top_module_name: Option<&str>,
    include_dirs: &[String], source_paths: &[String],
    settle_limit: Option<u32>, activity_mon: bool,
    sdf_file: Option<&str>, sdf_select: Option<xezim_core::sdf::DelaySelect>,
    defines: &[(String, Option<String>)],
    aitrace: bool,
    plusargs: &[String],
    threads: usize,
) -> Result<compiler::Simulator, String> {
    let _t0 = std::time::Instant::now();
    let (_definitions, elab) = parse_and_elaborate_multi(sources, top_module_name, include_dirs, source_paths, defines)?;

    eprintln!("[PHASE] elaborate: {:.1}ms", _t0.elapsed().as_secs_f64() * 1000.0);

    let mut sim = compiler::Simulator::new(elab, max_time);
    if let Some(limit) = settle_limit { sim.settle_limit = limit; }
    sim.activity_mon = activity_mon;
    sim.aitrace_mode = aitrace;
    sim.set_plusargs(plusargs);
    sim.set_threads(threads);

    if let Some(sdf_path) = sdf_file {
        let sdf_content = std::fs::read_to_string(sdf_path)
            .map_err(|e| format!("Cannot read SDF file '{}': {}", sdf_path, e))?;
        let sdf = xezim_core::sdf::parse_sdf(&sdf_content)
            .map_err(|e| format!("SDF parse error: {}", e))?;
        let select = sdf_select.unwrap_or(xezim_core::sdf::DelaySelect::Typ);
        let sim_timescale = 1e-9;
        let annotation = xezim_core::sdf::annotate_sdf(&sdf, sim_timescale, select);
        sim.sdf_annotation = Some(annotation);
    }
    sim.run();
    let sim_elapsed = _t0.elapsed();
    eprintln!("[PHASE] simulate: {:.1}ms", sim_elapsed.as_secs_f64() * 1000.0);
    eprintln!("------------------------------");
    eprintln!("Simulation finished at time {}", sim.time);
    Ok(sim)
}
