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

/// How many failures of one class (or one `std::randomize` call site) are
/// reported before the warning goes quiet, so a retry loop does not flood.
const RAND_FAIL_REPORTS: u32 = 3;

/// Whether failed randomize() calls are reported (`XEZIM_RAND_WARN=0`
/// silences them; read once).
fn rand_warn_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("XEZIM_RAND_WARN").map_or(true, |v| v != "0"))
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
    /// §18.6.3/§18.11: a randomize() that finds no solution returns 0 and
    /// leaves its random variables unchanged. Like the reference
    /// simulator's report of an unsolvable constraint set, say so on stderr:
    /// the class, its active constraint blocks with their locations, and
    /// whether the solver proved that no solution exists (`proven`) or only
    /// found none. Called for the outermost failing call only.
    pub(super) fn rand_fail_warn(
        &mut self,
        class_name: &str,
        constraints: &[ClassConstraint],
        constraint_src_files: &[Option<u32>],
        proven: bool,
    ) {
        if !rand_warn_enabled() {
            return;
        }
        let mut blocks: Vec<String> = Vec::new();
        for (ci, con) in constraints.iter().enumerate() {
            if con.name.name == "__back_ref__" {
                continue;
            }
            let name = match con.name.name.as_str() {
                "__inline__" => "with {...}",
                n => n,
            };
            let src = constraint_src_files.get(ci).copied().flatten();
            // An inline block's own span is synthetic: use its first item's.
            let span = match con.name.name.as_str() {
                "__inline__" => con.items.first().and_then(item_span).unwrap_or(con.span),
                _ => con.span,
            };
            let loc = self
                .span_file_and_line(span, src)
                .map(|(f, l)| {
                    let f = std::path::Path::new(&f)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or(f);
                    format!(" ({}:{})", f, l)
                })
                .unwrap_or_default();
            let b = format!("{}{}", name, loc);
            if !blocks.contains(&b) {
                blocks.push(b);
            }
        }
        let what = if proven {
            "its constraints have no solution"
        } else {
            "no solution found for its constraints"
        };
        self.rand_fail_report(
            format!("{}|{}", class_name, blocks.join("|")),
            format!("randomize() failed for class '{}': {}", class_name, what),
            &blocks,
            "class",
        );
    }

    /// The same for `std::randomize() with {...}` (§18.12): the call
    /// site's `with` block.
    pub(super) fn rand_fail_warn_std(&mut self, items: &[ConstraintItem]) {
        if !rand_warn_enabled() {
            return;
        }
        let src = self.current_src_file();
        let span = match (
            items.first().and_then(item_span),
            items.last().and_then(item_span),
        ) {
            (Some(a), Some(b)) => Some(crate::ast::Span::new(a.start, b.end)),
            _ => None,
        };
        let loc = span
            .and_then(|sp| self.span_file_and_line(sp, src))
            .map(|(f, l)| {
                let f = std::path::Path::new(&f)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(f);
                format!("{}:{}", f, l)
            })
            .unwrap_or_else(|| "?".to_string());
        self.rand_fail_report(
            format!("std::randomize@{}", loc),
            "std::randomize() failed: no solution found for its constraints".to_string(),
            &[format!("with {{...}} ({})", loc)],
            "call",
        );
    }

    fn rand_fail_report(&mut self, key: String, head: String, blocks: &[String], kind: &str) {
        let n = self.rand_fail_reports.entry(key).or_insert(0);
        *n += 1;
        let n = *n;
        if n > RAND_FAIL_REPORTS {
            return;
        }
        const SHOWN: usize = 6;
        let mut list = if blocks.is_empty() {
            "no constraint blocks".to_string()
        } else {
            blocks[..blocks.len().min(SHOWN)].join(", ")
        };
        if blocks.len() > SHOWN {
            list.push_str(&format!(", and {} more", blocks.len() - SHOWN));
        }
        let mut tail = String::new();
        if n == RAND_FAIL_REPORTS {
            tail.push_str(&format!(
                "; further failures of this {} are not reported",
                kind
            ));
        }
        if !rand_diag_enabled() && !self.rand_fail_hinted {
            self.rand_fail_hinted = true;
            tail.push_str("; XEZIM_RAND_DIAG=1 lists the unsatisfied constraint items");
        }
        eprintln!("[xezim][warning] {} ({}); returned 0{}", head, list, tail);
    }

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

    /// `file:line` and the whitespace-collapsed source text of an item.
    fn constraint_item_source(
        &self,
        item: &ConstraintItem,
        src_file: Option<u32>,
    ) -> Option<(String, String)> {
        let span = item_span(item)?;
        let i = match src_file.map(|i| i as usize) {
            Some(i)
                if self
                    .module
                    .source_texts
                    .get(i)
                    .is_some_and(|t| span.end <= t.len()) =>
            {
                i
            }
            Some(_) => return None,
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
        constraint_src_files: &[Option<u32>],
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
            .filter(|c| c.name.name != "__back_ref__")
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
                self.rand_diag_item(
                    &constraints[ci],
                    constraint_src_files.get(ci).copied().flatten(),
                    ii,
                )
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
                            self.rand_diag_item(
                                con,
                                constraint_src_files.get(cj).copied().flatten(),
                                jj,
                            )
                        );
                    }
                }
            }
        }
    }

    /// `block (file:line): text` for item `ii` of `con`.
    fn rand_diag_item(&self, con: &ClassConstraint, src_file: Option<u32>, ii: usize) -> String {
        let block = match con.name.name.as_str() {
            "__inline__" => "with {...}",
            n => n,
        };
        match self.constraint_item_source(&con.items[ii], src_file) {
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
