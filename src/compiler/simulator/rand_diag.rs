//! `XEZIM_RAND_DIAG=1`: a report on stderr when `randomize()` fails — the
//! class, the variables and constraint blocks switched off (`rand_mode` /
//! `constraint_mode`), and the constraint items the rejected trials left
//! unsatisfied, most often first, with their source text and location.
use super::*;

/// Whether `XEZIM_RAND_DIAG` is set (read once).
pub(super) fn rand_diag_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("XEZIM_RAND_DIAG").is_some())
}

/// Per-call tally of the constraint items each rejected trial left
/// unsatisfied, keyed by (constraint block, item) index.
#[derive(Default)]
pub(super) struct RandDiag {
    trials: u32,
    fails: HashMap<(usize, usize), u32>,
}

/// The source span of a constraint item.
fn item_span(item: &ConstraintItem) -> Option<crate::ast::Span> {
    match item {
        ConstraintItem::Expr(e) => Some(e.span),
        ConstraintItem::Inside { span, .. }
        | ConstraintItem::Implication { span, .. }
        | ConstraintItem::IfElse { span, .. }
        | ConstraintItem::Foreach { span, .. }
        | ConstraintItem::Solve { span, .. }
        | ConstraintItem::Unique { span, .. } => Some(*span),
        ConstraintItem::Soft(inner) => item_span(inner),
        ConstraintItem::Block(items) => {
            let first = items.first().and_then(item_span)?;
            let last = items.last().and_then(item_span)?;
            Some(crate::ast::Span::new(first.start, last.end))
        }
    }
}

impl Simulator {
    /// Record which items of `constraints` the current (rejected) trial
    /// leaves unsatisfied.
    pub(super) fn rand_diag_tally(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        diag: &mut RandDiag,
    ) {
        diag.trials += 1;
        for (ci, con) in constraints.iter().enumerate() {
            for (ii, item) in con.items.iter().enumerate() {
                if !Self::constraint_unmodeled(item) && !self.check_constraint_item(handle, item) {
                    *diag.fails.entry((ci, ii)).or_default() += 1;
                }
            }
        }
    }

    /// The source file a constraint block was parsed from: the one whose
    /// text at the block's span is that block's declaration. A span is an
    /// offset into its own file's preprocessed text.
    fn constraint_block_file(&self, con: &ClassConstraint) -> Option<usize> {
        let span = con.span;
        if span.start >= span.end {
            return None;
        }
        self.module.source_texts.iter().position(|t| {
            t.get(span.start..span.end).is_some_and(|s| {
                let s = s.trim_start();
                s.starts_with("constraint") && s.contains(con.name.name.as_str())
            })
        })
    }

    /// `file:line` and the whitespace-collapsed source text of an item.
    fn constraint_item_source(
        &self,
        con: &ClassConstraint,
        item: &ConstraintItem,
    ) -> Option<(String, String)> {
        let span = item_span(item)?;
        let file = self.constraint_block_file(con);
        let i = match file {
            Some(i) => i,
            None => {
                let mut fits = self
                    .module
                    .source_texts
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| span.end <= t.len());
                let (i, _) = fits.next()?;
                if fits.next().is_some() {
                    return None;
                }
                i
            }
        };
        let text = self.module.source_texts[i].get(span.start..span.end)?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let loc = match self.span_file_and_line(span, Some(i as u32)) {
            Some((f, l)) => format!(
                "{}:{}",
                std::path::Path::new(&f)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(f),
                l
            ),
            None => "?".to_string(),
        };
        Some((loc, text))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn rand_diag_report(
        &self,
        class_name: &str,
        rand_props: &[(String, u32)],
        rand_colls: &[RandColl],
        rand_obj_props: &[String],
        rand_disabled: &HashSet<String>,
        constraint_disabled: &HashSet<String>,
        constraints: &[ClassConstraint],
        diag: &RandDiag,
    ) {
        eprintln!(
            "[rand-diag] randomize() failed: class {} ({} rejected trials)",
            class_name, diag.trials
        );
        let mut vars: Vec<&str> = rand_props.iter().map(|(n, _)| n.as_str()).collect();
        vars.extend(rand_colls.iter().map(|c| c.prop.as_str()));
        vars.extend(rand_obj_props.iter().map(String::as_str));
        eprintln!("[rand-diag]   rand variables: {}", list(vars.clone()));
        eprintln!(
            "[rand-diag]   rand_mode off: {}",
            list(rand_disabled.iter().map(String::as_str).collect())
        );
        let blocks: Vec<&str> = constraints
            .iter()
            .map(|c| match c.name.name.as_str() {
                "__inline__" => "with {...}",
                n => n,
            })
            .collect();
        eprintln!("[rand-diag]   constraint blocks: {}", list(blocks));
        eprintln!(
            "[rand-diag]   constraint_mode off: {}",
            list(constraint_disabled.iter().map(String::as_str).collect())
        );
        let mut fails: Vec<(&(usize, usize), &u32)> = diag.fails.iter().collect();
        fails.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        if fails.is_empty() {
            eprintln!(
                "[rand-diag]   unsatisfied items: none recorded (the failure came from a solver that does not report items)"
            );
            return;
        }
        // The rand variables each active item reads, to name the items an
        // unsatisfied one competes with for the same variables.
        let rand_names: HashSet<&str> = vars.iter().copied().collect();
        let item_vars: Vec<Vec<HashSet<String>>> = constraints
            .iter()
            .map(|c| {
                c.items
                    .iter()
                    .map(|item| {
                        let mut ids = HashSet::default();
                        self.collect_item_idents(item, &mut ids);
                        ids.retain(|n| rand_names.contains(n.as_str()));
                        ids
                    })
                    .collect()
            })
            .collect();
        eprintln!("[rand-diag]   unsatisfied items (rejected trials):");
        for (&(ci, ii), &n) in fails {
            eprintln!(
                "[rand-diag]     {}/{} {}",
                n,
                diag.trials,
                self.rand_diag_item(&constraints[ci], ii)
            );
            let own = &item_vars[ci][ii];
            if own.is_empty() {
                continue;
            }
            let mut shared: Vec<&str> = own.iter().map(String::as_str).collect();
            shared.sort_unstable();
            for (cj, (con, vars_of)) in constraints.iter().zip(&item_vars).enumerate() {
                for (jj, vars) in vars_of.iter().enumerate() {
                    if (cj, jj) != (ci, ii) && !vars.is_disjoint(own) {
                        eprintln!(
                            "[rand-diag]       related ({}): {}",
                            shared.join(", "),
                            self.rand_diag_item(con, jj)
                        );
                    }
                }
            }
        }
    }

    /// `block (file:line): text` for item `ii` of `con`.
    fn rand_diag_item(&self, con: &ClassConstraint, ii: usize) -> String {
        let block = match con.name.name.as_str() {
            "__inline__" => "with {...}",
            n => n,
        };
        match self.constraint_item_source(con, &con.items[ii]) {
            Some((loc, text)) => format!("{} ({}): {}", block, loc, text),
            None => format!("{} (item {}): {:?}", block, ii, con.items[ii]),
        }
    }
}

/// A sorted, comma-separated list, or `-` when empty.
fn list(mut names: Vec<&str>) -> String {
    if names.is_empty() {
        return "-".to_string();
    }
    names.sort_unstable();
    names.dedup();
    names.join(", ")
}
