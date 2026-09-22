//! GTKWave/RTLBrowse `.stems` companion file for source-annotated tracing.
//!
//! GTKWave's `rtlbrowse` sub-tool opens a design's source and annotates live
//! signal values onto the lines only when it is given a stems file that
//! names each module's source file and the instance tree. Verilator feeds it
//! one through `xml2stems`; xezim writes the same line format directly from
//! the elaborated module, so `gtkwave -t out.stems out.fst` works without a
//! converter step.
//!
//! The format is the one-record-per-line grammar that
//! `contrib/xml2stems/xml2stems.cc` emits and `contrib/rtlbrowse/stem_recurse.c`
//! parses. xezim writes two of its record kinds:
//!
//! ```text
//! ++ module <name> file <path> lines <start> - <end>
//! ++ comp <instance> type <module-name> parent <parent-module-name>
//! ```
//!
//! A `++ module` record maps a module DEFINITION to its source file (and,
//! when the preprocessed text is available, the line its `module` keyword
//! starts on); a `++ comp` record links an instance to the definition it
//! instantiates and to the module that contains it. RTLBrowse turns the
//! comp records into the hierarchy tree it shows and opens a module's
//! source at the module record's line when a tree item is clicked.
//!
//! Three honest limits:
//! * The stems grammar tokenizes fields with `%s`, so a source path that
//!   contains whitespace is split across tokens and misparsed by RTLBrowse.
//!   xezim passes paths through verbatim; paths with spaces are the author's
//!   to avoid, exactly as with `xml2stems`.
//! * The declaration line comes from a scan of the preprocessed text for a
//!   line whose first token is a header keyword (`module`, `interface`,
//!   `program`, or `macromodule`) and whose next token is the definition's
//!   name. A compiled-artifact run carries no preprocessed text, so those
//!   fall back to line 1; so does any header the scan does not recognize
//!   (an escaped identifier is the common case). The source window still
//!   opens at the top of the file either way.
//! * A module defined inside a file pulled in by `\`include` is reported
//!   against the including file, at the position the preprocessor spliced it
//!   to: xezim-core attributes every spliced-in description to its top-level
//!   source and keeps no include provenance. The stems records stay
//!   structurally valid (roots, hierarchy, and comp parents all resolve),
//!   but the file/line pair can point at a blank or spliced line rather than
//!   the real declaration.

use std::io::Write;

use super::elaborate::ElaboratedModule;

/// Source position of one module definition, as a `++ module` record needs
/// it.
pub struct ModuleSite {
    /// Source file the module is declared in. `<unknown>` when the design
    /// carries no sources (e.g. a compiled artifact).
    pub file: String,
    /// 1-based declaration line; 1 when it cannot be determined.
    pub line: u32,
}

/// Truncate an identifier token at its first non-identifier character, so
/// `foo(`/`foo;`/`foo #(...)` from a whitespace split all compare as `foo`.
fn head_token(t: &str) -> &str {
    let cut = t
        .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
        .unwrap_or(t.len());
    &t[..cut]
}

/// Declaration line of `name`, by scanning its file's preprocessed text for
/// a line whose first token is a header keyword (`module`, `interface`,
/// `program`, or `macromodule` — an instance's definition can use any of
/// the four) followed by `name`. Falls back to 1 when the text is absent
/// (compiled artifact) or the scan finds nothing. The preprocessor blanks
/// comment and attribute bodies to spaces before this text exists, so
/// neither can fake a header line.
fn decl_line(module: &ElaboratedModule, name: &str) -> u32 {
    let Some(&idx) = module.src_file_of_module.get(name) else {
        return 1;
    };
    let Some(text) = module.source_texts.get(idx as usize) else {
        return 1;
    };
    for (n, line) in text.lines().enumerate() {
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("module") | Some("interface") | Some("program") | Some("macromodule") => {}
            _ => continue,
        }
        if let Some(tok) = tokens.next()
            && head_token(tok) == name
        {
            return (n + 1) as u32;
        }
    }
    1
}

/// Source position of a module definition, resolving the source-file index
/// to a path and combining it with the scan result.
fn module_site(module: &ElaboratedModule, name: &str) -> ModuleSite {
    let file = module
        .src_file_of_module
        .get(name)
        .and_then(|&i| module.source_files.get(i as usize))
        .filter(|f| !f.is_empty())
        .cloned()
        .unwrap_or_else(|| "<unknown>".to_string());
    ModuleSite {
        file,
        line: decl_line(module, name),
    }
}

/// Every module definition the stems file must name: the top module plus
/// the definition of each instance, deduplicated in first-seen order so the
/// output is deterministic.
fn module_defs(module: &ElaboratedModule) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut defs = Vec::new();
    for name in std::iter::once(&module.name).chain(module.instances.iter().map(|i| &i.def_name)) {
        if seen.insert(name.clone()) {
            defs.push(name.clone());
        }
    }
    defs
}

/// Parent module name of an instance: the definition of the containing
/// instance, or the top module for a child of the top. A parent path not in
/// the instance table resolves to its last segment as a best-effort name
/// (should not happen for an elaborated design).
fn parent_module(module: &ElaboratedModule, parent: &str) -> String {
    if parent.is_empty() {
        return module.name.clone();
    }
    module
        .instances
        .iter()
        .find(|i| i.path == parent)
        .map(|i| i.def_name.clone())
        .unwrap_or_else(|| match parent.rsplit_once('.') {
            Some((_, tail)) => tail.to_string(),
            None => parent.to_string(),
        })
}

/// Write the `.stems` companion file for `module` to `path`. The FST dump
/// is the matching waveform for GTKWave's `-t output.stems output.fst`.
pub fn write_stems_file(module: &ElaboratedModule, path: &str) -> std::io::Result<()> {
    let mut out = std::fs::File::create(path)?;
    write_stems(&mut out, module)
}

/// Emit the stems records for `module` to `out`. Public so tests can verify
/// the grammar with an in-memory sink.
pub fn write_stems<W: Write>(out: &mut W, module: &ElaboratedModule) -> std::io::Result<()> {
    for name in module_defs(module) {
        let site = module_site(module, &name);
        writeln!(
            out,
            "++ module {} file {} lines {} - {}",
            name, site.file, site.line, site.line
        )?;
    }
    for inst in &module.instances {
        let cname = inst.path.rsplit('.').next().unwrap_or(inst.path.as_str());
        let pname = parent_module(module, &inst.parent);
        writeln!(
            out,
            "++ comp {} type {} parent {}",
            cname, inst.def_name, pname
        )?;
    }
    Ok(())
}
