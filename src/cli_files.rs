//! Options that take their values from a file:
//! - `--fst-scope-file <file>` / `-fst_scope_file <file>`: FST dump scopes,
//!   the same as one `--fst-scope` per entry;
//! - `-xezim_env <file>` / `--xezim-env <file>`: `XEZIM_*` environment
//!   settings, applied before xezim reads any of them.
//!
//! Both files take `#` and `//` comments and blank lines. Pure parsing (no
//! simulator state), so tests/misc/cli_files.rs includes this file directly.

/// Strip a `#` or `//` comment and surrounding whitespace from one line.
fn strip_comment(line: &str) -> &str {
    let cut = [line.find('#'), line.find("//")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(line.len());
    line[..cut].trim()
}

/// The scopes listed in an FST scope file: one or more per line, separated
/// by whitespace or commas.
pub(crate) fn parse_scope_file(text: &str) -> Vec<String> {
    text.lines()
        .map(strip_comment)
        .flat_map(|l| {
            l.split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The prefix every accepted variable name has. Spelled with `concat!` so the
/// env-var registry check (which scans for string literals naming
/// `XEZIM_*` variables) does not take the bare prefix for a variable.
const XEZIM_PREFIX: &str = concat!("XEZIM", "_");

/// One setting from an env file: `Some(value)` sets the variable, `None`
/// removes it.
pub(crate) type EnvSetting = (String, Option<String>);

/// Parse an env file. Accepted line forms:
/// `NAME=value`, `export NAME=value`, `setenv NAME value`, `NAME value`,
/// `unsetenv NAME` and `unset NAME`. Values may be quoted with `"` or `'`.
/// Only `XEZIM_*` names are accepted; anything else is an error that names
/// the line.
pub(crate) fn parse_env_file(text: &str) -> Result<Vec<EnvSetting>, String> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        let lineno = n + 1;
        let (name, value): (&str, Option<&str>) = if let Some(rest) = line
            .strip_prefix("unsetenv ")
            .or_else(|| line.strip_prefix("unset "))
        {
            (rest.trim(), None)
        } else {
            let body = line
                .strip_prefix("export ")
                .or_else(|| line.strip_prefix("setenv "))
                .unwrap_or(line)
                .trim();
            match body.find(|c: char| c == '=' || c.is_whitespace()) {
                Some(i) => (body[..i].trim(), Some(body[i + 1..].trim())),
                None => (body, Some("")),
            }
        };
        if !name.starts_with(XEZIM_PREFIX)
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!(
                "line {}: '{}' is not an XEZIM_* variable",
                lineno, name
            ));
        }
        let value = value.map(|v| {
            let v = v.trim();
            let quoted = v.len() >= 2
                && ((v.starts_with('"') && v.ends_with('"'))
                    || (v.starts_with('\'') && v.ends_with('\'')));
            if quoted { &v[1..v.len() - 1] } else { v }.to_string()
        });
        out.push((name.to_string(), value));
    }
    Ok(out)
}

/// The value of an option spelled `-name <v>`, `--name <v>`, `-name=v` or
/// `--name=v`, for each spelling in `names` (given without dashes). Returns
/// `Some(Some(v))` for the `=` form, `Some(None)` when the value is the next
/// argument, and `None` when `arg` is not this option.
pub(crate) fn option_value<'a>(arg: &'a str, names: &[&str]) -> Option<Option<&'a str>> {
    let body = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-'))?;
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (body, None),
    };
    names.contains(&name).then_some(value)
}

/// Spellings of the env-file option.
pub(crate) const XEZIM_ENV_NAMES: &[&str] = &["xezim_env", "xezim-env"];
/// Spellings of the FST scope-file option.
pub(crate) const FST_SCOPE_FILE_NAMES: &[&str] = &["fst_scope_file", "fst-scope-file"];

/// The env files named on the command line, in order.
pub(crate) fn env_files_in_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match option_value(&args[i], XEZIM_ENV_NAMES) {
            Some(Some(v)) => files.push(v.to_string()),
            Some(None) => {
                i += 1;
                match args.get(i) {
                    Some(v) => files.push(v.clone()),
                    None => return Err("-xezim_env requires a file".to_string()),
                }
            }
            None => {}
        }
        i += 1;
    }
    Ok(files)
}

/// Register the complete parsed runtime plusargs with VPI/DPI in their
/// original order. Remove only positions the CLI parser classified as plusargs;
/// option operands, including a filename equal to a plusarg, must survive.
pub(crate) fn runtime_argv(
    args: &[String],
    plusargs: &[String],
    direct_plusarg_positions: &[usize],
) -> Vec<String> {
    // The parser visits raw argument positions in increasing order.
    let mut argv: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(index, _)| direct_plusarg_positions.binary_search(index).is_err())
        .map(|(_, arg)| arg.clone())
        .collect();
    argv.extend_from_slice(plusargs);
    argv
}
