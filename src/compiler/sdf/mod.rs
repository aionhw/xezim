//! SDF (Standard Delay Format) parser and annotator.
//! IEEE 1497-2001: Standard Delay Format (SDF) for the Electronic Design Process.
//!
//! Parses SDF files and annotates gate-level netlists with pin-to-pin delays
//! for timing-accurate simulation.
//!
//! Supported constructs:
//!   - IOPATH delays (rise/fall, min:typ:max)
//!   - INTERCONNECT delays
//!   - TIMESCALE
//!   - CELL/INSTANCE hierarchy

use std::collections::HashMap;

/// A parsed SDF delay file.
#[derive(Debug, Clone)]
pub struct SdfFile {
    pub version: String,
    pub design: String,
    pub timescale: f64, // in seconds (e.g., 1e-9 for 1ns)
    pub cells: Vec<SdfCell>,
}

/// A CELL entry in the SDF file.
#[derive(Debug, Clone)]
pub struct SdfCell {
    pub cell_type: String,
    pub instance: String, // hierarchical instance path
    pub delays: Vec<SdfDelay>,
}

/// A single delay specification.
#[derive(Debug, Clone)]
pub enum SdfDelay {
    /// IOPATH: input_port → output_port with rise/fall delays
    IoPath {
        input: String,
        output: String,
        rise: DelayTriple,
        fall: DelayTriple,
    },
    /// INTERCONNECT: source → destination wire delay
    Interconnect {
        source: String,
        dest: String,
        rise: DelayTriple,
        fall: DelayTriple,
    },
}

/// A min:typ:max delay triple. Values in SDF timescale units.
#[derive(Debug, Clone, Copy)]
pub struct DelayTriple {
    pub min: f64,
    pub typ: f64,
    pub max: f64,
}

impl DelayTriple {
    pub fn zero() -> Self { Self { min: 0.0, typ: 0.0, max: 0.0 } }
    pub fn single(v: f64) -> Self { Self { min: v, typ: v, max: v } }
}

/// Which delay value to use from the min:typ:max triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelaySelect {
    Min,
    Typ,
    Max,
}

impl DelaySelect {
    pub fn pick(&self, triple: &DelayTriple) -> f64 {
        match self {
            DelaySelect::Min => triple.min,
            DelaySelect::Typ => triple.typ,
            DelaySelect::Max => triple.max,
        }
    }
}

/// Annotated delays for simulation: maps (signal_id) → delay in simulation time units.
/// For each output signal, stores the delay from any input change to output update.
#[derive(Debug, Clone)]
pub struct SdfAnnotation {
    /// Map from output signal name → delay in simulation time units (integer).
    pub signal_delays: HashMap<String, u64>,
    /// Detailed per-pin delays: (instance.output) → Vec<(input_pin, rise_delay, fall_delay)>
    pub pin_delays: HashMap<String, Vec<PinDelay>>,
}

#[derive(Debug, Clone)]
pub struct PinDelay {
    pub input_pin: String,
    pub rise: u64, // delay in sim time units
    pub fall: u64,
}

impl SdfAnnotation {
    pub fn new() -> Self {
        Self { signal_delays: HashMap::new(), pin_delays: HashMap::new() }
    }

    /// Get the maximum delay for an output signal (conservative for settle).
    pub fn get_delay(&self, signal_name: &str) -> u64 {
        self.signal_delays.get(signal_name).copied().unwrap_or(0)
    }
}

// ═══════════════════════════════════════════════════════════════════
// SDF Parser
// ═══════════════════════════════════════════════════════════════════

/// Parse an SDF file from string content.
pub fn parse_sdf(content: &str) -> Result<SdfFile, String> {
    let mut parser = SdfParser::new(content);
    parser.parse()
}

struct SdfParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> SdfParser<'a> {
    fn new(input: &'a str) -> Self { Self { input, pos: 0 } }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos];
            if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
                self.pos += 1;
            } else if self.input[self.pos..].starts_with("//") {
                // Line comment
                while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if self.input[self.pos..].starts_with("/*") {
                // Block comment
                self.pos += 2;
                while self.pos + 1 < self.input.len() {
                    if self.input[self.pos..].starts_with("*/") { self.pos += 2; break; }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn expect_char(&mut self, ch: char) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == Some(ch) { self.pos += 1; Ok(()) }
        else { Err(format!("SDF: expected '{}' at pos {}", ch, self.pos)) }
    }

    fn read_token(&mut self) -> String {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos];
            if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r'
                || ch == b'(' || ch == b')' || ch == b'"' {
                break;
            }
            self.pos += 1;
        }
        self.input[start..self.pos].to_string()
    }

    fn read_quoted_string(&mut self) -> Result<String, String> {
        self.skip_ws();
        if self.peek() != Some('"') { return Err(format!("SDF: expected '\"' at pos {}", self.pos)); }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'"' {
            self.pos += 1;
        }
        let s = self.input[start..self.pos].to_string();
        if self.pos < self.input.len() { self.pos += 1; } // skip closing "
        Ok(s)
    }

    fn read_number(&mut self) -> f64 {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input.as_bytes()[self.pos];
            if ch.is_ascii_digit() || ch == b'.' || ch == b'-' || ch == b'+' || ch == b'e' || ch == b'E' {
                self.pos += 1;
            } else { break; }
        }
        self.input[start..self.pos].parse().unwrap_or(0.0)
    }

    fn at_end(&self) -> bool { self.pos >= self.input.len() }

    fn parse(&mut self) -> Result<SdfFile, String> {
        let mut file = SdfFile {
            version: String::new(), design: String::new(),
            timescale: 1e-9, // default 1ns
            cells: Vec::new(),
        };

        self.skip_ws();
        self.expect_char('(')?;
        let keyword = self.read_token();
        if keyword != "DELAYFILE" { return Err(format!("SDF: expected DELAYFILE, got {}", keyword)); }

        loop {
            self.skip_ws();
            if self.at_end() { break; }
            match self.peek() {
                Some(')') => { self.pos += 1; break; }
                Some('(') => {
                    self.pos += 1;
                    let kw = self.read_token();
                    match kw.as_str() {
                        "SDFVERSION" => { file.version = self.read_quoted_string()?; self.expect_char(')')?; }
                        "DESIGN" => { file.design = self.read_quoted_string()?; self.expect_char(')')?; }
                        "DATE" | "VENDOR" | "PROGRAM" | "VERSION" | "DIVIDER" | "VOLTAGE" | "PROCESS" | "TEMPERATURE" => {
                            // Skip value
                            self.skip_to_close_paren();
                        }
                        "TIMESCALE" => {
                            file.timescale = self.parse_timescale()?;
                            self.expect_char(')')?;
                        }
                        "CELL" => {
                            if let Ok(cell) = self.parse_cell() {
                                file.cells.push(cell);
                            }
                        }
                        _ => { self.skip_to_close_paren(); }
                    }
                }
                _ => { self.pos += 1; }
            }
        }

        Ok(file)
    }

    fn skip_to_close_paren(&mut self) {
        let mut depth = 1;
        while self.pos < self.input.len() && depth > 0 {
            match self.input.as_bytes()[self.pos] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'"' => { self.pos += 1; while self.pos < self.input.len() && self.input.as_bytes()[self.pos] != b'"' { self.pos += 1; } }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn parse_timescale(&mut self) -> Result<f64, String> {
        self.skip_ws();
        let num = self.read_number();
        let unit = self.read_token();
        let scale = match unit.as_str() {
            "s" => 1.0,
            "ms" => 1e-3,
            "us" => 1e-6,
            "ns" => 1e-9,
            "ps" => 1e-12,
            "fs" => 1e-15,
            _ => return Err(format!("SDF: unknown timescale unit '{}'", unit)),
        };
        Ok(num * scale)
    }

    fn parse_cell(&mut self) -> Result<SdfCell, String> {
        let mut cell = SdfCell { cell_type: String::new(), instance: String::new(), delays: Vec::new() };

        loop {
            self.skip_ws();
            match self.peek() {
                Some(')') => { self.pos += 1; break; }
                Some('(') => {
                    self.pos += 1;
                    let kw = self.read_token();
                    match kw.as_str() {
                        "CELLTYPE" => { cell.cell_type = self.read_quoted_string()?; self.expect_char(')')?; }
                        "INSTANCE" => {
                            self.skip_ws();
                            // Instance can be empty (for wildcard) or a path
                            if self.peek() == Some(')') { self.pos += 1; }
                            else {
                                cell.instance = self.read_token();
                                self.expect_char(')')?;
                            }
                        }
                        "DELAY" => { self.parse_delay_section(&mut cell)?; }
                        _ => { self.skip_to_close_paren(); }
                    }
                }
                None => break,
                _ => { self.pos += 1; }
            }
        }

        Ok(cell)
    }

    fn parse_delay_section(&mut self, cell: &mut SdfCell) -> Result<(), String> {
        loop {
            self.skip_ws();
            match self.peek() {
                Some(')') => { self.pos += 1; return Ok(()); }
                Some('(') => {
                    self.pos += 1;
                    let kw = self.read_token();
                    match kw.as_str() {
                        "ABSOLUTE" | "INCREMENT" => {
                            self.parse_delay_entries(cell)?;
                        }
                        _ => { self.skip_to_close_paren(); }
                    }
                }
                None => return Ok(()),
                _ => { self.pos += 1; }
            }
        }
    }

    fn parse_delay_entries(&mut self, cell: &mut SdfCell) -> Result<(), String> {
        loop {
            self.skip_ws();
            match self.peek() {
                Some(')') => { self.pos += 1; return Ok(()); }
                Some('(') => {
                    self.pos += 1;
                    let kw = self.read_token();
                    match kw.as_str() {
                        "IOPATH" => {
                            let input = self.read_token();
                            let output = self.read_token();
                            let rise = self.parse_delay_value()?;
                            let fall = self.parse_delay_value()?;
                            self.expect_char(')')?;
                            cell.delays.push(SdfDelay::IoPath { input, output, rise, fall });
                        }
                        "INTERCONNECT" => {
                            let source = self.read_token();
                            let dest = self.read_token();
                            let rise = self.parse_delay_value()?;
                            let fall = self.parse_delay_value()?;
                            self.expect_char(')')?;
                            cell.delays.push(SdfDelay::Interconnect { source, dest, rise, fall });
                        }
                        _ => { self.skip_to_close_paren(); }
                    }
                }
                None => return Ok(()),
                _ => { self.pos += 1; }
            }
        }
    }

    /// Parse a delay value: either (min:typ:max) or (value)
    fn parse_delay_value(&mut self) -> Result<DelayTriple, String> {
        self.skip_ws();
        if self.peek() != Some('(') {
            // Bare number
            let v = self.read_number();
            return Ok(DelayTriple::single(v));
        }
        self.pos += 1; // skip (
        self.skip_ws();

        if self.peek() == Some(')') {
            // Empty parens = zero delay
            self.pos += 1;
            return Ok(DelayTriple::zero());
        }

        let first = self.read_number();
        self.skip_ws();
        if self.peek() == Some(':') {
            // min:typ:max
            self.pos += 1;
            let typ = self.read_number();
            self.skip_ws();
            self.expect_char(':')?;
            let max = self.read_number();
            self.skip_ws();
            self.expect_char(')')?;
            Ok(DelayTriple { min: first, typ, max })
        } else if self.peek() == Some(')') {
            // Single value
            self.pos += 1;
            Ok(DelayTriple::single(first))
        } else {
            Err(format!("SDF: unexpected char in delay value at pos {}", self.pos))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Annotation: convert SDF delays into simulation-ready form
// ═══════════════════════════════════════════════════════════════════

/// Annotate an elaborated design with SDF delays.
/// `sim_timescale` is the simulation timescale in seconds (e.g., 1e-9 for 1ns).
/// `delay_select` chooses min/typ/max from the SDF triples.
pub fn annotate_sdf(
    sdf: &SdfFile,
    sim_timescale: f64,
    delay_select: DelaySelect,
) -> SdfAnnotation {
    let mut annotation = SdfAnnotation::new();
    let scale = sdf.timescale / sim_timescale;

    for cell in &sdf.cells {
        for delay in &cell.delays {
            match delay {
                SdfDelay::IoPath { input, output, rise, fall } => {
                    let out_signal = format!("{}.{}", cell.instance, output);
                    let in_pin = format!("{}.{}", cell.instance, input);
                    let rise_ticks = (delay_select.pick(rise) * scale).round() as u64;
                    let fall_ticks = (delay_select.pick(fall) * scale).round() as u64;
                    let max_delay = rise_ticks.max(fall_ticks);

                    // Store max delay for the output signal (used by settle)
                    let existing = annotation.signal_delays.entry(out_signal.clone()).or_insert(0);
                    *existing = (*existing).max(max_delay);

                    // Store per-pin delay
                    annotation.pin_delays.entry(out_signal).or_insert_with(Vec::new)
                        .push(PinDelay { input_pin: in_pin, rise: rise_ticks, fall: fall_ticks });
                }
                SdfDelay::Interconnect { source: _, dest, rise, fall } => {
                    let rise_ticks = (delay_select.pick(rise) * scale).round() as u64;
                    let fall_ticks = (delay_select.pick(fall) * scale).round() as u64;
                    let max_delay = rise_ticks.max(fall_ticks);
                    let existing = annotation.signal_delays.entry(dest.clone()).or_insert(0);
                    *existing = (*existing).max(max_delay);
                }
            }
        }
    }

    annotation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sdf() {
        let sdf_content = r#"
(DELAYFILE
  (SDFVERSION "4.0")
  (DESIGN "test")
  (TIMESCALE 1ns)
  (CELL
    (CELLTYPE "INV")
    (INSTANCE u1)
    (DELAY
      (ABSOLUTE
        (IOPATH A Y (0.05:0.1:0.15) (0.04:0.08:0.12))
      )
    )
  )
)
"#;
        let sdf = parse_sdf(sdf_content).unwrap();
        assert_eq!(sdf.version, "4.0");
        assert_eq!(sdf.design, "test");
        assert_eq!(sdf.cells.len(), 1);
        assert_eq!(sdf.cells[0].cell_type, "INV");
        assert_eq!(sdf.cells[0].instance, "u1");
        assert_eq!(sdf.cells[0].delays.len(), 1);
        if let SdfDelay::IoPath { input, output, rise, fall } = &sdf.cells[0].delays[0] {
            assert_eq!(input, "A");
            assert_eq!(output, "Y");
            assert!((rise.typ - 0.1).abs() < 1e-9);
            assert!((fall.typ - 0.08).abs() < 1e-9);
        } else { panic!("expected IoPath"); }
    }

    #[test]
    fn test_annotate_typ() {
        let sdf_content = r#"
(DELAYFILE
  (SDFVERSION "4.0")
  (TIMESCALE 1ns)
  (CELL
    (CELLTYPE "BUF")
    (INSTANCE top.buf1)
    (DELAY
      (ABSOLUTE
        (IOPATH A X (0.1:0.2:0.3) (0.15:0.25:0.35))
      )
    )
  )
)
"#;
        let sdf = parse_sdf(sdf_content).unwrap();
        let ann = annotate_sdf(&sdf, 1e-9, DelaySelect::Typ);
        // 0.25ns max(rise=0.2, fall=0.25) in 1ns timescale → rounds to 0 ticks
        // With ps timescale (1e-12), 0.25ns = 250ps = 250 ticks
        let ann_ps = annotate_sdf(&sdf, 1e-12, DelaySelect::Typ);
        assert_eq!(ann_ps.get_delay("top.buf1.X"), 250);
    }
}
