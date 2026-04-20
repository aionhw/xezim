//! SystemVerilog Simulator (xezim)
//! Main library entry point.

pub mod compiler;

pub use sv_parser::parse;
pub use sv_parser::lexer;
pub use sv_parser::preprocessor;
pub use sv_parser::diagnostics;
pub use sv_parser::ParseResult;
pub use sv_parser::ast;

/// Set the log file for simulation output.
pub fn set_log_file(_path: &str) -> Result<(), String> {
    // TODO: implement log file support
    Ok(())
}

/// Tokenize a source string.
pub fn tokenize_file(source: &str, _path: Option<&std::path::Path>) -> Vec<lexer::Token> {
    lexer::Lexer::new(source).tokenize()
}

/// Parse a source string into an AST.
pub fn parse_str(source: &str) -> Result<ParseResult, Vec<diagnostics::Diagnostic>> {
    let result = sv_parser::parse(source);
    if !result.errors.is_empty() {
        Err(result.errors)
    } else {
        Ok(result)
    }
}

/// Simulate a single source string.
pub fn simulate(source: &str, max_time: u64) -> Result<compiler::Simulator, String> {
    simulate_multi(&[source.to_string()], max_time, None, &[], &[], None, false, None, None, &[], false, &[], 1)
}

/// Magic bytes identifying a xezim compiled artifact.
pub const XEZIM_BYTECODE_MAGIC: &[u8; 8] = b"XEZIMBC\x01";

/// Serialize a compiled ElaboratedModule to a file.
pub fn write_compiled(elab: &compiler::elaborate::ElaboratedModule, path: &str) -> Result<(), String> {
    let bytes = bincode::serialize(elab).map_err(|e| format!("serialize: {}", e))?;
    let mut out = Vec::with_capacity(bytes.len() + 8);
    out.extend_from_slice(XEZIM_BYTECODE_MAGIC);
    out.extend_from_slice(&bytes);
    std::fs::write(path, &out).map_err(|e| format!("write '{}': {}", path, e))
}

/// Read a compiled artifact from a file. Returns Ok(Some(elab)) if the file is
/// a valid artifact, Ok(None) if it lacks the magic header, Err on I/O or
/// deserialization failure.
pub fn read_compiled(path: &str) -> Result<Option<compiler::elaborate::ElaboratedModule>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read '{}': {}", path, e))?;
    if bytes.len() < 8 || &bytes[..8] != XEZIM_BYTECODE_MAGIC {
        return Ok(None);
    }
    let elab = bincode::deserialize(&bytes[8..]).map_err(|e| format!("deserialize: {}", e))?;
    Ok(Some(elab))
}

#[derive(Debug, Clone)]
pub enum SourceDefinition {
    Module(ast::module::ModuleDeclaration),
    Interface(ast::module::InterfaceDeclaration),
    Program(ast::module::ProgramDeclaration),
    Class(ast::decl::ClassDeclaration),
    Package(ast::module::PackageDeclaration),
    Typedef(ast::decl::TypedefDeclaration),
}

impl SourceDefinition {
    pub fn name(&self) -> String {
        match self {
            SourceDefinition::Module(m) => m.name.name.clone(),
            SourceDefinition::Interface(i) => i.name.name.clone(),
            SourceDefinition::Program(p) => p.name.name.clone(),
            SourceDefinition::Class(c) => c.name.name.clone(),
            SourceDefinition::Package(p) => p.name.name.clone(),
            SourceDefinition::Typedef(t) => t.name.name.clone(),
        }
    }

    pub fn items(&self) -> &[ast::decl::ModuleItem] {
        match self {
            SourceDefinition::Module(m) => &m.items,
            SourceDefinition::Interface(i) => &i.items,
            SourceDefinition::Program(p) => &p.items,
            SourceDefinition::Class(_) | SourceDefinition::Package(_) | SourceDefinition::Typedef(_) => &[],
        }
    }
}

pub fn parse_and_elaborate_multi(
    sources: &[String],
    top_module_name: Option<&str>,
    include_dirs: &[String],
    source_files: &[String],
    defines: &[(String, Option<String>)],
) -> Result<(ahash::AHashMap<String, SourceDefinition>, compiler::elaborate::ElaboratedModule), String> {
    let mut all_descriptions = Vec::new();
    let mut pp = preprocessor::Preprocessor::new();
    for dir in include_dirs { pp.add_include_dir(std::path::PathBuf::from(dir)); }
    for (name, val) in defines {
        pp.define(name.clone(), preprocessor::MacroDef {
            name: name.clone(), params: None,
            body: val.clone().unwrap_or_default(),
        });
    }

    for (i, source) in sources.iter().enumerate() {
        let source_path = source_files.get(i).map(|p| std::path::PathBuf::from(p));
        let preprocessed = pp.preprocess_file(source, source_path.as_deref());
        
        let tokens = lexer::Lexer::new(&preprocessed).tokenize();
        let mut parser = parse::Parser::new(tokens);
        let source_ast = parser.parse_source_text();
        let diags = parser.diagnostics().to_vec();

        if diags.iter().any(|d| d.severity == diagnostics::Severity::Error) {
            let errs: Vec<_> = diags.iter()
                .filter(|d| d.severity == diagnostics::Severity::Error)
                .map(|d| d.to_string()).collect();
            return Err(format!("Parse errors in source {}:\n{}", i, errs.join("\n")));
        }
        all_descriptions.extend(source_ast.descriptions);
    }

    parse_and_elaborate(&all_descriptions, top_module_name, include_dirs)
}

pub fn simulate_multi(
    sources: &[String], max_time: u64, top_module_name: Option<&str>,
    include_dirs: &[String], source_paths: &[String],
    settle_limit: Option<u32>, activity_mon: bool,
    sdf_file: Option<&str>, sdf_select: Option<compiler::sdf::DelaySelect>,
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
        let sdf = compiler::sdf::parse_sdf(&sdf_content)
            .map_err(|e| format!("SDF parse error: {}", e))?;
        let select = sdf_select.unwrap_or(compiler::sdf::DelaySelect::Typ);
        let sim_timescale = 1e-9;
        let annotation = compiler::sdf::annotate_sdf(&sdf, sim_timescale, select);
        sim.sdf_annotation = Some(annotation);
    }
    sim.run();
    let sim_elapsed = _t0.elapsed();
    eprintln!("[PHASE] simulate: {:.1}ms", sim_elapsed.as_secs_f64() * 1000.0);
    eprintln!("------------------------------");
    eprintln!("Simulation finished at time {}", sim.time);
    Ok(sim)
}

fn parse_and_elaborate(
    all_descriptions: &[ast::Description],
    top_module_name: Option<&str>,
    include_dirs: &[String],
) -> Result<(ahash::AHashMap<String, SourceDefinition>, compiler::elaborate::ElaboratedModule), String> {
    let mut definitions: ahash::AHashMap<String, SourceDefinition> = ahash::AHashMap::new();
    let mut top_module = None;
    let mut top_level_imports = Vec::new();
    let mut top_level_lets = Vec::new();
    let mut top_level_functions: Vec<ast::decl::FunctionDeclaration> = Vec::new();
    let mut top_level_tasks: Vec<ast::decl::TaskDeclaration> = Vec::new();
    let mut top_level_nettypes: Vec<ast::decl::NettypeDeclaration> = Vec::new();
    for desc in all_descriptions {
        match desc {
            ast::Description::Module(m) => {
                definitions.insert(m.name.name.clone(), SourceDefinition::Module(m.clone()));
                top_module = Some(m.name.name.clone());
            }
            ast::Description::Interface(i) => {
                definitions.insert(i.name.name.clone(), SourceDefinition::Interface(i.clone()));
            }
            ast::Description::Program(p) => {
                definitions.insert(p.name.name.clone(), SourceDefinition::Program(p.clone()));
                top_module = Some(p.name.name.clone());
            }
            ast::Description::Class(c) => {
                definitions.insert(c.name.name.clone(), SourceDefinition::Class(c.clone()));
            }
            ast::Description::Package(p) => {
                definitions.insert(p.name.name.clone(), SourceDefinition::Package(p.clone()));
            }
            ast::Description::TypedefDecl(t) => {
                definitions.insert(t.name.name.clone(), SourceDefinition::Typedef(t.clone()));
            }
            ast::Description::ImportDecl(id) => {
                top_level_imports.push(id.clone());
            }
            ast::Description::PackageItem(ast::decl::PackageItem::Checker(c)) => {
                let m = ast::module::ModuleDeclaration {
                    attrs: Vec::new(),
                    kind: ast::module::ModuleKind::Module,
                    lifetime: None,
                    name: c.name.clone(),
                    params: Vec::new(),
                    ports: c.ports.clone(),
                    items: c.items.clone(),
                    endlabel: c.endlabel.clone(),
                    span: c.span,
                };
                definitions.insert(m.name.name.clone(), SourceDefinition::Module(m));
            }
            ast::Description::PackageItem(ast::decl::PackageItem::Let(l)) => {
                top_level_lets.push(l.clone());
            }
            ast::Description::PackageItem(ast::decl::PackageItem::Function(f)) => {
                top_level_functions.push(f.clone());
            }
            ast::Description::PackageItem(ast::decl::PackageItem::Task(t)) => {
                top_level_tasks.push(t.clone());
            }
            ast::Description::PackageItem(ast::decl::PackageItem::Nettype(n)) => {
                top_level_nettypes.push(n.clone());
            }
            _ => {}
        }
    }
    // Inject $unit-scope functions and tasks into every module definition so
    // they're resolvable from inside an instance.
    if !top_level_functions.is_empty() || !top_level_tasks.is_empty() || !top_level_nettypes.is_empty() {
        for def in definitions.values_mut() {
            if let SourceDefinition::Module(m) = def {
                for f in top_level_functions.iter().rev() {
                    m.items.insert(0, ast::decl::ModuleItem::FunctionDeclaration(f.clone()));
                }
                for t in top_level_tasks.iter().rev() {
                    m.items.insert(0, ast::decl::ModuleItem::TaskDeclaration(t.clone()));
                }
                for n in top_level_nettypes.iter().rev() {
                    m.items.insert(0, ast::decl::ModuleItem::NettypeDeclaration(n.clone()));
                }
            }
        }
    }
    if !include_dirs.is_empty() { resolve_library_modules(&mut definitions, include_dirs)?; }

    if let Some(name) = top_module_name {
        if definitions.contains_key(name) { top_module = Some(name.to_string()); }
        else { return Err(format!("Top module '{}' not found.", name)); }
    } else {
        let mut instantiated: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in definitions.values() { collect_instantiated_modules(m.items(), &mut instantiated); }
        let candidates: Vec<String> = definitions.keys().filter(|n| !instantiated.contains(n.as_str())).cloned().collect();
        if candidates.len() == 1 { top_module = Some(candidates[0].clone()); }
        else if candidates.len() > 1 {
            for c in &candidates {
                if definitions.get(c).unwrap().items().iter().any(|item| matches!(item, ast::decl::ModuleItem::InitialConstruct(_))) {
                    top_module = Some(c.clone()); break;
                }
            }
        }
    }

    let top_name = top_module.ok_or("No module found")?;
    let top_def = definitions.get(&top_name).ok_or_else(|| format!("Module '{}' not found", top_name))?;
    let params = ahash::AHashMap::new();

    let def_refs: ahash::AHashMap<String, compiler::elaborate::Definition> =
        definitions.iter().filter_map(|(k, v)| {
            let def = match v {
                SourceDefinition::Module(m) => compiler::elaborate::Definition::Module(m),
                SourceDefinition::Interface(i) => compiler::elaborate::Definition::Interface(i),
                SourceDefinition::Program(p) => compiler::elaborate::Definition::Program(p),
                SourceDefinition::Class(c) => compiler::elaborate::Definition::Class(c),
                SourceDefinition::Package(p) => compiler::elaborate::Definition::Package(p),
                SourceDefinition::Typedef(t) => compiler::elaborate::Definition::Typedef(t),
            };
            Some((k.clone(), def))
        }).collect();

    let elab_def = match top_def {
        SourceDefinition::Module(m) => compiler::elaborate::Definition::Module(m),
        SourceDefinition::Interface(i) => compiler::elaborate::Definition::Interface(i),
        SourceDefinition::Program(p) => compiler::elaborate::Definition::Program(p),
        SourceDefinition::Class(c) => compiler::elaborate::Definition::Class(c),
        SourceDefinition::Package(p) => compiler::elaborate::Definition::Package(p),
        _ => return Err(format!("Top-level element '{}' is not a module or program", top_name)),
    };
    let mut elab = compiler::elaborate::elaborate_module_with_defs(
        elab_def,
        &params,
        Some(&def_refs),
        &top_level_imports,
        &top_level_lets,
    )?;

    compiler::elaborate::inline_instantiations(&mut elab, &def_refs)?;
    Ok((definitions, elab))
}

fn collect_instantiated_modules(items: &[ast::decl::ModuleItem], set: &mut std::collections::HashSet<String>) {
    for item in items {
        match item {
            ast::decl::ModuleItem::ModuleInstantiation(mi) => { set.insert(mi.module_name.name.clone()); }
            ast::decl::ModuleItem::GenerateIf(gi) => {
                for (_cond, items) in &gi.branches { collect_instantiated_modules(items, set); }
            }
            ast::decl::ModuleItem::GenerateFor(gf) => collect_instantiated_modules(&gf.items, set),
            _ => {}
        }
    }
}

fn resolve_library_modules(_definitions: &mut ahash::AHashMap<String, SourceDefinition>, _include_dirs: &[String]) -> Result<(), String> {
    // TODO: implement library module resolution
    Ok(())
}

pub fn compile_native(
    _sources: &[String],
    _top_module: Option<&str>,
    _include_dirs: &[String],
    _source_files: &[String],
    _defines: &[(String, Option<String>)],
) -> Result<std::path::PathBuf, String> {
    Err("Native compilation not yet implemented in this refactored lib.rs".to_string())
}

pub fn log_println(s: &str) { println!("{}", s); }
pub fn log_eprintln(s: &str) { eprintln!("{}", s); }
