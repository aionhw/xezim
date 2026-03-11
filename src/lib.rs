#![allow(non_camel_case_types)]
//! # sisvsim — SystemVerilog Simulator
//!
//! A SystemVerilog parser targeting IEEE 1800-2017/2023 compliance.
//! Built as a hand-written recursive descent parser in Rust.
//!
//! ## Usage
//!
//! ```rust
//! use sisvsim::parse_str;
//!
//! let result = parse_str("module top; endmodule");
//! assert!(result.is_ok());
//! ```

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod preprocessor;
pub mod diagnostics;
pub mod compiler;

use ast::SourceText;
use lexer::Lexer;
use parser::Parser;
use preprocessor::Preprocessor;
use diagnostics::Diagnostic;

/// Result of parsing: AST + diagnostics.
#[derive(Debug)]
pub struct ParseResult {
    pub source_text: SourceText,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParseResult {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == diagnostics::Severity::Error)
    }
}

/// Parse a SystemVerilog source string.
pub fn parse_str(source: &str) -> Result<ParseResult, Vec<Diagnostic>> {
    let mut pp = Preprocessor::new();
    let preprocessed = pp.preprocess(source);
    let lexer = Lexer::new(&preprocessed);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let source_text = parser.parse_source_text();
    let diagnostics = parser.diagnostics().to_vec();
    Ok(ParseResult { source_text, diagnostics })
}

/// Parse a SystemVerilog source file.
pub fn parse_file(path: &std::path::Path) -> Result<ParseResult, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    parse_str(&source).map_err(|diags| {
        diags.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")
    })
}

/// Tokenize a SystemVerilog source string.
pub fn tokenize(source: &str) -> Vec<lexer::Token> {
    let lexer = Lexer::new(source);
    lexer.tokenize()
}

/// Simulate a SystemVerilog source string.
/// Finds the top-level module (last module or one with initial blocks) and runs it.
pub fn simulate(source: &str, max_time: u64) -> Result<compiler::Simulator, String> {
    simulate_multi(&[source.to_string()], max_time, None)
}

/// Simulate multiple SystemVerilog source strings.
/// All modules from all sources are collected. The top module is selected automatically
/// (preferring modules with initial blocks) unless `top_module` is specified.
pub fn simulate_multi(sources: &[String], max_time: u64, top_module_name: Option<&str>) -> Result<compiler::Simulator, String> {
    let mut all_descriptions = Vec::new();

    for (i, source) in sources.iter().enumerate() {
        let result = parse_str(source).map_err(|diags| {
            diags.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")
        })?;

        if result.has_errors() {
            let errs: Vec<_> = result.diagnostics.iter()
                .filter(|d| d.severity == diagnostics::Severity::Error)
                .map(|d| d.to_string())
                .collect();
            return Err(format!("Parse errors in source {}:\n{}", i, errs.join("\n")));
        }

        all_descriptions.extend(result.source_text.descriptions);
    }

    // Collect all modules by name
    let mut modules: std::collections::HashMap<String, ast::module::ModuleDeclaration> = std::collections::HashMap::new();
    let mut top_module = None;
    for desc in &all_descriptions {
        if let ast::Description::Module(m) = desc {
            modules.insert(m.name.name.clone(), m.clone());
            top_module = Some(m.name.name.clone());
        }
    }

    // If user specified a top module, use it
    if let Some(name) = top_module_name {
        if modules.contains_key(name) {
            top_module = Some(name.to_string());
        } else {
            return Err(format!("Top module '{}' not found. Available: {}", name,
                modules.keys().cloned().collect::<Vec<_>>().join(", ")));
        }
    } else {
        // Find the top module: the one NOT instantiated by any other module.
        // Collect all instantiated module names.
        let mut instantiated: std::collections::HashSet<String> = std::collections::HashSet::new();
        for desc in &all_descriptions {
            if let ast::Description::Module(m) = desc {
                collect_instantiated_modules(&m.items, &mut instantiated);
            }
        }
        // The top module is one that exists but is never instantiated
        let candidates: Vec<String> = modules.keys()
            .filter(|name| !instantiated.contains(name.as_str()))
            .cloned().collect();
        if candidates.len() == 1 {
            top_module = Some(candidates[0].clone());
        } else if candidates.len() > 1 {
            // Multiple uninstantiated modules — prefer one with initial blocks (testbench)
            for c in &candidates {
                let m = modules.get(c).unwrap();
                let has_initial = m.items.iter().any(|item| matches!(item, ast::decl::ModuleItem::InitialConstruct(_)));
                if has_initial {
                    top_module = Some(c.clone());
                    break;
                }
            }
            if top_module.is_none() || !candidates.contains(&top_module.clone().unwrap_or_default()) {
                top_module = Some(candidates[0].clone());
            }
        }
        // else: candidates is empty (circular?), fall through with last-parsed default
    }

    let top_name = top_module.ok_or("No module found in source")?;
    let module = modules.get(&top_name).ok_or(format!("Module '{}' not found", top_name))?;
    let params = std::collections::HashMap::new();
    let mut elab = compiler::elaborate_module(module, &params)?;

    // Inline module instantiations (using references from the modules map)
    let module_refs: std::collections::HashMap<String, &ast::module::ModuleDeclaration> =
        modules.iter().map(|(k, v)| (k.clone(), v)).collect();
    compiler::elaborate::inline_instantiations(&mut elab, &module_refs)?;

    let mut sim = compiler::Simulator::new(elab, max_time);
    sim.run();
    Ok(sim)
}

/// Recursively collect all module names that are instantiated within a list of module items.
fn collect_instantiated_modules(items: &[ast::decl::ModuleItem], out: &mut std::collections::HashSet<String>) {
    use ast::decl::ModuleItem;
    for item in items {
        match item {
            ModuleItem::ModuleInstantiation(inst) => {
                out.insert(inst.module_name.name.clone());
            }
            ModuleItem::GenerateRegion(gr) => {
                collect_instantiated_modules(&gr.items, out);
            }
            ModuleItem::GenerateIf(gi) => {
                for (_, items) in &gi.branches {
                    collect_instantiated_modules(items, out);
                }
            }
            _ => {}
        }
    }
}
