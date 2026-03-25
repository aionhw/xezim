#![allow(non_camel_case_types)]
//! # sisvsim — SystemVerilog Simulator
//!
//! Uses the `sv-parser` crate for parsing, and provides elaboration + simulation.

pub use sv_parser::ast;
pub use sv_parser::lexer;
pub use sv_parser::parser;
pub use sv_parser::preprocessor;
pub use sv_parser::diagnostics;

pub mod compiler;

use sv_parser::ast::SourceText;
use sv_parser::lexer::Lexer;
use sv_parser::parser::Parser;
use sv_parser::preprocessor::Preprocessor;
use sv_parser::diagnostics::Diagnostic;

#[derive(Debug)]
pub struct ParseResult {
    pub source_text: SourceText,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParseResult {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == sv_parser::diagnostics::Severity::Error)
    }
}

pub fn parse_str(source: &str) -> Result<ParseResult, Vec<Diagnostic>> {
    let mut pp = Preprocessor::new();
    let preprocessed = pp.preprocess(source);
    let tokens = Lexer::new(&preprocessed).tokenize();
    let mut parser = Parser::new(tokens);
    let source_text = parser.parse_source_text();
    let diagnostics = parser.diagnostics().to_vec();
    Ok(ParseResult { source_text, diagnostics })
}

pub fn tokenize(source: &str) -> Vec<sv_parser::lexer::Token> {
    Lexer::new(source).tokenize()
}

pub fn simulate(source: &str, max_time: u64) -> Result<compiler::Simulator, String> {
    simulate_multi(&[source.to_string()], max_time, None, &[], &[], None, false)
}

pub fn simulate_multi(
    sources: &[String], max_time: u64, top_module_name: Option<&str>,
    lib_dirs: &[String], source_paths: &[String],
    settle_limit: Option<u32>, activity_mon: bool,
) -> Result<compiler::Simulator, String> {
    let _t0 = std::time::Instant::now();
    let mut all_descriptions = Vec::new();
    let include_dirs: Vec<std::path::PathBuf> = lib_dirs.iter().map(|d| std::path::PathBuf::from(d)).collect();

    for (i, source) in sources.iter().enumerate() {
        let source_path = source_paths.get(i).map(|p| std::path::PathBuf::from(p));
        let result = if source_path.is_some() || !include_dirs.is_empty() {
            let mut pp = Preprocessor::new();
            for dir in &include_dirs { pp.add_include_dir(dir.clone()); }
            let preprocessed = pp.preprocess_file(source, source_path.as_deref());
            let tokens = Lexer::new(&preprocessed).tokenize();
            let mut parser = Parser::new(tokens);
            Ok(ParseResult { source_text: parser.parse_source_text(), diagnostics: parser.diagnostics().to_vec() })
        } else { parse_str(source) };

        let result = result.map_err(|diags: Vec<Diagnostic>| diags.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"))?;
        if result.has_errors() {
            let errs: Vec<_> = result.diagnostics.iter()
                .filter(|d| d.severity == sv_parser::diagnostics::Severity::Error)
                .map(|d| d.to_string()).collect();
            return Err(format!("Parse errors in source {}:\n{}", i, errs.join("\n")));
        }
        all_descriptions.extend(result.source_text.descriptions);
    }

    let mut modules: ahash::AHashMap<String, ast::module::ModuleDeclaration> = ahash::AHashMap::new();
    let mut top_module = None;
    for desc in &all_descriptions {
        if let ast::Description::Module(m) = desc { modules.insert(m.name.name.clone(), m.clone()); top_module = Some(m.name.name.clone()); }
    }
    if !lib_dirs.is_empty() { resolve_library_modules(&mut modules, lib_dirs)?; }

    if let Some(name) = top_module_name {
        if modules.contains_key(name) { top_module = Some(name.to_string()); }
        else { return Err(format!("Top module '{}' not found. Available: {}", name, modules.keys().cloned().collect::<Vec<_>>().join(", "))); }
    } else {
        let mut instantiated: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in modules.values() { collect_instantiated_modules(&m.items, &mut instantiated); }
        let candidates: Vec<String> = modules.keys().filter(|n| !instantiated.contains(n.as_str())).cloned().collect();
        if candidates.len() == 1 { top_module = Some(candidates[0].clone()); }
        else if candidates.len() > 1 {
            for c in &candidates {
                if modules.get(c).unwrap().items.iter().any(|item| matches!(item, ast::decl::ModuleItem::InitialConstruct(_))) {
                    top_module = Some(c.clone()); break;
                }
            }
        }
    }

    let top_name = top_module.ok_or("No module found")?;
    let top_mod = modules.get(&top_name).ok_or_else(|| format!("Module '{}' not found", top_name))?;
    eprintln!("[PHASE] parse: {:.1}ms", _t0.elapsed().as_secs_f64() * 1000.0);

    let elab_start = std::time::Instant::now();
    let params = ahash::AHashMap::new();
    let mut elab = compiler::elaborate_module(top_mod, &params)?;
    let module_refs: ahash::AHashMap<String, &ast::module::ModuleDeclaration> =
        modules.iter().map(|(k, v)| (k.clone(), v)).collect();
    compiler::elaborate::inline_instantiations(&mut elab, &module_refs)?;
    eprintln!("[PHASE] elaborate: {:.1}ms", elab_start.elapsed().as_secs_f64() * 1000.0);

    let mut sim = compiler::Simulator::new(elab, max_time);
    if let Some(limit) = settle_limit { sim.settle_limit = limit; }
    sim.activity_mon = activity_mon;
    sim.run();
    let sim_elapsed = _t0.elapsed();
    eprintln!("[PHASE] simulate: {:.1}ms", sim_elapsed.as_secs_f64() * 1000.0);
    eprintln!("------------------------------");
    eprintln!("Simulation finished at time {}", sim.time);
    Ok(sim)
}

fn collect_instantiated_modules(items: &[ast::decl::ModuleItem], set: &mut std::collections::HashSet<String>) {
    for item in items {
        match item {
            ast::decl::ModuleItem::ModuleInstantiation(mi) => { set.insert(mi.module_name.name.clone()); }
            ast::decl::ModuleItem::GenerateRegion(gr) => collect_instantiated_modules(&gr.items, set),
            ast::decl::ModuleItem::GenerateRegion(gr) => collect_instantiated_modules(&gr.items, set),
            ast::decl::ModuleItem::GenerateIf(gi) => {
                for (_cond, items) in &gi.branches { collect_instantiated_modules(items, set); }
                
            }
            _ => {}
        }
    }
}

fn resolve_library_modules(modules: &mut ahash::AHashMap<String, ast::module::ModuleDeclaration>, lib_dirs: &[String]) -> Result<(), String> {
    loop {
        let mut unresolved = Vec::new();
        for m in modules.values() { collect_unresolved(&m.items, &mut unresolved, modules); }
        if unresolved.is_empty() { break; }
        let mut found_any = false;
        for name in &unresolved {
            for dir in lib_dirs {
                for ext in &[".v", ".sv"] {
                    let path = std::path::PathBuf::from(dir).join(format!("{}{}", name, ext));
                    if path.exists() {
                        let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
                        let inc: Vec<std::path::PathBuf> = lib_dirs.iter().map(|d| std::path::PathBuf::from(d)).collect();
                        let mut pp = Preprocessor::new();
                        for d in &inc { pp.add_include_dir(d.clone()); }
                        let preprocessed = pp.preprocess_file(&source, Some(path.as_path()));
                        let mut parser = Parser::new(Lexer::new(&preprocessed).tokenize());
                        for desc in parser.parse_source_text().descriptions {
                            if let ast::Description::Module(m) = desc { modules.insert(m.name.name.clone(), m); found_any = true; }
                        }
                        break;
                    }
                }
            }
        }
        if !found_any { break; }
    }
    Ok(())
}

fn collect_unresolved(items: &[ast::decl::ModuleItem], out: &mut Vec<String>, modules: &ahash::AHashMap<String, ast::module::ModuleDeclaration>) {
    for item in items {
        match item {
            ast::decl::ModuleItem::ModuleInstantiation(mi) => {
                if !modules.contains_key(&mi.module_name.name) && !out.contains(&mi.module_name.name) { out.push(mi.module_name.name.clone()); }
            }
            ast::decl::ModuleItem::GenerateRegion(gr) => collect_unresolved(&gr.items, out, modules),
            ast::decl::ModuleItem::GenerateRegion(gr) => collect_unresolved(&gr.items, out, modules),
            ast::decl::ModuleItem::GenerateIf(gi) => {
                for (_cond, items) in &gi.branches { collect_unresolved(items, out, modules); }
                
            }
            _ => {}
        }
    }
}
