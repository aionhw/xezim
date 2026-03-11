//! SystemVerilog preprocessor (IEEE 1800-2017 §22)
//!
//! Handles `define, `ifdef/`ifndef/`else/`endif, `include, `undef, etc.
//! This is a simplified preprocessor suitable for parsing purposes.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub params: Option<Vec<String>>,
    pub body: String,
}

pub struct Preprocessor {
    defines: HashMap<String, MacroDef>,
}

impl Preprocessor {
    pub fn new() -> Self {
        Self {
            defines: HashMap::new(),
        }
    }

    pub fn with_defines(defines: HashMap<String, String>) -> Self {
        let mut pp = Self::new();
        for (k, v) in defines {
            pp.defines.insert(k.clone(), MacroDef {
                name: k,
                params: None,
                body: v,
            });
        }
        pp
    }

    pub fn define(&mut self, name: String, value: MacroDef) {
        self.defines.insert(name, value);
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.defines.contains_key(name)
    }

    /// Simple preprocessing pass: strip directives we can't handle,
    /// expand simple defines, evaluate ifdef/ifndef blocks.
    pub fn preprocess(&mut self, source: &str) -> String {
        let mut output = String::with_capacity(source.len());
        let mut lines = source.lines().peekable();
        let mut ifdef_stack: Vec<bool> = Vec::new(); // true = active

        while let Some(line) = lines.next() {
            let trimmed = line.trim();

            // Strip (* ... *) attributes (IEEE 1800-2017 §5.12)
            // These are synthesis/tool directives that don't affect simulation
            if trimmed.starts_with("(*") && trimmed.ends_with("*)") {
                output.push('\n');
                continue;
            }

            if trimmed.starts_with("`define") {
                if ifdef_stack.iter().all(|&b| b) {
                    self.parse_define(trimmed);
                }
                // Don't output `define lines
                output.push('\n');
                continue;
            }

            if trimmed.starts_with("`undef") {
                if ifdef_stack.iter().all(|&b| b) {
                    let name = trimmed[6..].trim().to_string();
                    self.defines.remove(&name);
                }
                output.push('\n');
                continue;
            }

            if trimmed.starts_with("`ifdef") {
                let name = trimmed[6..].trim();
                ifdef_stack.push(self.is_defined(name));
                output.push('\n');
                continue;
            }

            if trimmed.starts_with("`ifndef") {
                let name = trimmed[7..].trim();
                ifdef_stack.push(!self.is_defined(name));
                output.push('\n');
                continue;
            }

            if trimmed.starts_with("`else") {
                if let Some(last) = ifdef_stack.last_mut() {
                    *last = !*last;
                }
                output.push('\n');
                continue;
            }

            if trimmed.starts_with("`endif") {
                ifdef_stack.pop();
                output.push('\n');
                continue;
            }

            // Skip inactive blocks
            if !ifdef_stack.iter().all(|&b| b) {
                output.push('\n');
                continue;
            }

            // Skip `include, `timescale (pass through for now)
            if trimmed.starts_with("`include") || trimmed.starts_with("`timescale") {
                output.push('\n');
                continue;
            }

            // Expand macros in the line
            let expanded = self.expand_macros(line);
            // Strip inline (* ... *) attributes
            let expanded = Self::strip_attributes(&expanded);
            output.push_str(&expanded);
            output.push('\n');
        }

        output
    }

    fn parse_define(&mut self, line: &str) {
        let rest = line[7..].trim(); // after `define
        // Find name
        let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
        let name = rest[..name_end].to_string();
        let after_name = &rest[name_end..];
        
        // Check for parameterized macro: `define NAME(param1, param2) body
        let (params, body) = if after_name.starts_with('(') {
            // Find closing paren
            if let Some(close) = after_name.find(')') {
                let param_str = &after_name[1..close];
                let params: Vec<String> = param_str.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let body = after_name[close + 1..].trim().to_string();
                (Some(params), body)
            } else {
                (None, after_name.trim().to_string())
            }
        } else {
            (None, after_name.trim().to_string())
        };
        
        if !name.is_empty() {
            self.defines.insert(name.clone(), MacroDef {
                name,
                params,
                body,
            });
        }
    }

    fn expand_macros(&self, line: &str) -> String {
        let mut result = self.expand_macros_once(line);
        // Recursively expand up to 16 times to handle nested macros
        for _ in 0..16 {
            if !result.contains('`') { break; }
            let next = self.expand_macros_once(&result);
            if next == result { break; }
            result = next;
        }
        result
    }

    fn expand_macros_once(&self, line: &str) -> String {
        let mut result = String::with_capacity(line.len());
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'`' {
                i += 1;
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let macro_name = &line[start..i];
                if let Some(def) = self.defines.get(macro_name) {
                    if def.params.is_some() && i < bytes.len() && bytes[i] == b'(' {
                        // Parameterized macro: find arguments
                        let args = Self::extract_macro_args(line, &mut i);
                        let params = def.params.as_ref().unwrap();
                        let mut body = def.body.clone();
                        for (pi, pname) in params.iter().enumerate() {
                            if let Some(arg) = args.get(pi) {
                                body = body.replace(pname, arg);
                            }
                        }
                        result.push_str(&body);
                    } else {
                        result.push_str(&def.body);
                    }
                } else {
                    result.push('`');
                    result.push_str(macro_name);
                }
            } else {
                result.push(line[i..].chars().next().unwrap());
                i += 1;
            }
        }
        result
    }
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Preprocessor {
    /// Strip (* ... *) Verilog attributes from a line
    /// Extract parenthesized macro arguments, handling nested parens.
    /// `i` should point at the opening '('. After return, `i` is past the closing ')'.
    fn extract_macro_args(line: &str, i: &mut usize) -> Vec<String> {
        let bytes = line.as_bytes();
        *i += 1; // skip '('
        let mut args = Vec::new();
        let mut depth = 1;
        let mut arg_start = *i;
        while *i < bytes.len() && depth > 0 {
            match bytes[*i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        let arg = line[arg_start..*i].trim().to_string();
                        if !arg.is_empty() || !args.is_empty() {
                            args.push(arg);
                        }
                        *i += 1; // skip ')'
                        return args;
                    }
                }
                b',' if depth == 1 => {
                    args.push(line[arg_start..*i].trim().to_string());
                    arg_start = *i + 1;
                }
                _ => {}
            }
            *i += 1;
        }
        args
    }

    fn strip_attributes(line: &str) -> String {
        let mut result = String::with_capacity(line.len());
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if i + 1 < bytes.len() && bytes[i] == b'(' && bytes[i + 1] == b'*' {
                // Check this isn't inside a string
                // Find matching *)
                let mut j = i + 2;
                while j + 1 < bytes.len() {
                    if bytes[j] == b'*' && bytes[j + 1] == b')' {
                        j += 2;
                        break;
                    }
                    j += 1;
                }
                if j <= bytes.len() {
                    // Replace attribute with space to preserve spacing
                    result.push(' ');
                    i = j;
                    continue;
                }
            }
            result.push(bytes[i] as char);
            i += 1;
        }
        result
    }
}
