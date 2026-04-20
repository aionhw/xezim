//! Native code generator for xezim.
//!
//! Compiles an elaborated gate-level design into a standalone Rust source file,
//! which is then compiled to a native binary. The generated binary reproduces
//! the same output as the interpreted simulator, including $display, $monitor,
//! $finish, initial blocks with #delays, and VCD-compatible signal tracing.

use ahash::AHashMap as HashMap;

use std::io::Write;
use std::path::Path;
use super::elaborate::*;

use crate::ast::expr::*;
use crate::ast::stmt::*;

/// Generate a native Rust source file from an elaborated module.
pub fn generate_native(
    elab: &ElaboratedModule,
    output_dir: &Path,
    _testbench_source: &str,
) -> Result<std::path::PathBuf, String> {
    let out_path = output_dir.join("sim_native.rs");
    let mut f = std::fs::File::create(&out_path)
        .map_err(|e| format!("Cannot create {}: {}", out_path.display(), e))?;
    let g = CodeGen::new(elab);
    g.emit_all(&mut f)?;
    Ok(out_path)
}

/// Compile the generated Rust source to a native binary.
pub fn compile_native(source_path: &Path, output_path: &Path) -> Result<(), String> {
    let status = std::process::Command::new("rustc")
        .args(&["-O", "--edition", "2021",
            "-o", output_path.to_str().unwrap_or("sim_native"),
            source_path.to_str().unwrap_or("sim_native.rs")])
        .status()
        .map_err(|e| format!("Failed to run rustc: {}", e))?;
    if status.success() { Ok(()) }
    else { Err(format!("rustc failed with exit code {:?}", status.code())) }
}

/// Linearized action from flattening initial blocks.
#[derive(Debug, Clone)]
enum EventAction {
    Assign(usize, String),           // signal_id, rhs_code
    Display(String, Vec<usize>),     // format, arg signal IDs (usize::MAX = $time)
    Monitor(String, Vec<usize>),
    Finish,
    Delay(u64),                      // marker for splitting into event chains
    ScheduleEvent(u64, usize),       // delay, event_id
}

struct CodeGen<'a> {
    elab: &'a ElaboratedModule,
    sig_names: Vec<String>,
    sig_to_id: HashMap<String, usize>,
    sig_widths: Vec<u32>,
}

impl<'a> CodeGen<'a> {
    fn new(elab: &'a ElaboratedModule) -> Self {
        let mut sig_names: Vec<String> = elab.signals.keys().cloned().collect();
        sig_names.sort();
        let sig_to_id: HashMap<String, usize> = sig_names.iter().enumerate()
            .map(|(i, n)| (n.clone(), i)).collect();
        let sig_widths: Vec<u32> = sig_names.iter()
            .map(|n| elab.signals.get(n).map_or(1, |s| s.width)).collect();
        Self { elab, sig_names, sig_to_id, sig_widths }
    }

    fn emit_all(&self, f: &mut std::fs::File) -> Result<(), String> {
        let w = |f: &mut std::fs::File, s: &str| writeln!(f, "{}", s).map_err(|e| e.to_string());
        let num = self.sig_names.len();

        w(f, "//! Auto-generated native simulation binary.")?;
        w(f, &format!("//! Design: {}  Signals: {}", self.elab.name, num))?;
        w(f, "#![allow(unused, non_snake_case, non_upper_case_globals)]")?;
        w(f, "use std::collections::BTreeMap;")?;
        w(f, "")?;

        // Signal constants + width table
        for (i, name) in self.sig_names.iter().enumerate() {
            w(f, &format!("const S_{}: usize = {};  // {} (w={})", san(name), i, name, self.sig_widths[i]))?;
        }
        w(f, &format!("const N: usize = {};", num))?;
        w(f, "")?;

        // Width array
        write!(f, "static WIDTHS: [u32; N] = [").map_err(|e| e.to_string())?;
        for (i, &w_) in self.sig_widths.iter().enumerate() {
            if i > 0 { write!(f, ",").map_err(|e| e.to_string())?; }
            write!(f, "{}", w_).map_err(|e| e.to_string())?;
        }
        w(f, "];")?;

        // Name array
        w(f, &format!("static NAMES: [&str; N] = ["))?;
        for (i, name) in self.sig_names.iter().enumerate() {
            if i > 0 { write!(f, ",").map_err(|e| e.to_string())?; }
            write!(f, "\"{}\"", name).map_err(|e| e.to_string())?;
        }
        w(f, "];")?;
        w(f, "")?;

        // SimState
        self.emit_simstate(f)?;

        // settle()
        self.emit_settle(f)?;

        // clock_edge()
        self.emit_clock_edge(f)?;

        // main() with initial blocks + event loop
        self.emit_main(f)?;

        Ok(())
    }

    fn emit_simstate(&self, f: &mut std::fs::File) -> Result<(), String> {
        let w = |f: &mut std::fs::File, s: &str| writeln!(f, "{}", s).map_err(|e| e.to_string());
        w(f, "struct SimState {")?;
        w(f, "    val: [u64; N], xz: [u64; N],  // per-signal value and unknown bits")?;
        w(f, "    prev_val: [u64; N], prev_xz: [u64; N],")?;
        w(f, "    time: u64, finished: bool,")?;
        w(f, "    monitor_fmt: Option<String>, monitor_args: Vec<usize>,")?;
        w(f, "    monitor_prev: Vec<u64>,")?;
        w(f, "    event_queue: BTreeMap<u64, Vec<usize>>,  // time -> list of event IDs")?;
        w(f, "}")?;
        w(f, "")?;
        w(f, "impl SimState {")?;
        w(f, "    fn get(&self, id: usize) -> (u64, u64) { (self.val[id], self.xz[id]) }")?;
        w(f, "    fn get_bit(&self, id: usize, bit: usize) -> (bool, bool) {")?;
        w(f, "        let m = 1u64 << bit; (self.val[id] & m != 0, self.xz[id] & m != 0)")?;
        w(f, "    }")?;
        w(f, "    fn set(&mut self, id: usize, v: u64, x: u64) {")?;
        w(f, "        let mask = if WIDTHS[id] >= 64 { u64::MAX } else { (1u64 << WIDTHS[id]) - 1 };")?;
        w(f, "        self.val[id] = v & mask; self.xz[id] = x & mask;")?;
        w(f, "    }")?;
        w(f, "    fn snapshot(&mut self) { self.prev_val = self.val; self.prev_xz = self.xz; }")?;
        w(f, "    fn posedge(&self, id: usize) -> bool {")?;
        w(f, "        let cv = self.val[id] & 1 != 0 && self.xz[id] & 1 == 0;")?;
        w(f, "        let pv = self.prev_val[id] & 1 != 0 && self.prev_xz[id] & 1 == 0;")?;
        w(f, "        cv && !pv")?;
        w(f, "    }")?;
        w(f, "    fn negedge(&self, id: usize) -> bool {")?;
        w(f, "        let cv = self.val[id] & 1 != 0 && self.xz[id] & 1 == 0;  // current is 1")?;
        w(f, "        let pv = self.prev_val[id] & 1 != 0 && self.prev_xz[id] & 1 == 0;  // prev is 1")?;
        w(f, "        !cv && pv  // was 1, now 0")?;
        w(f, "    }")?;
        // Format helpers
        w(f, "    fn format_val(&self, id: usize, spec: char) -> String {")?;
        w(f, "        if id == usize::MAX { return format!(\"{}\", self.time); }")?;
        w(f, "        if id >= N { return \"?\".to_string(); }")?;
        w(f, "        let v = self.val[id]; let x = self.xz[id]; let w = WIDTHS[id];")?;
        w(f, "        match spec {")?;
        w(f, "            'b' | 'B' => { let mut s = String::new(); for i in (0..w).rev() {")?;
        w(f, "                let bv = (v >> i) & 1; let bx = (x >> i) & 1;")?;
        w(f, "                s.push(match (bv, bx) { (0,0)=>'0', (1,0)=>'1', (0,1)=>'x', _=>'z' });")?;
        w(f, "            } s }")?;
        w(f, "            'h' | 'H' | 'x' | 'X' => { if x != 0 { let mut s = String::new();")?;
        w(f, "                for i in (0..((w+3)/4)).rev() { let nib_v = (v >> (i*4)) & 0xf; let nib_x = (x >> (i*4)) & 0xf;")?;
        w(f, "                    if nib_x == 0xf { s.push('x'); } else if nib_x != 0 { s.push('X'); }")?;
        w(f, "                    else { s.push(char::from_digit(nib_v as u32, 16).unwrap()); }")?;
        w(f, "                } s } else { format!(\"{:x}\", v) } }")?;
        w(f, "            'd' | 'D' => { if x != 0 { \"x\".to_string() } else { format!(\"{}\", v) } }")?;
        w(f, "            _ => format!(\"{}\", v),")?;
        w(f, "        }")?;
        w(f, "    }")?;
        // $display format
        w(f, "    fn display_format(&self, fmt: &str, args: &[usize]) -> String {")?;
        w(f, "        let mut result = String::new(); let mut ai = 0;")?;
        w(f, "        let mut chars = fmt.chars().peekable();")?;
        w(f, "        while let Some(c) = chars.next() {")?;
        w(f, "            if c == '%' {")?;
        w(f, "                let mut width_s = String::new();")?;
        w(f, "                while chars.peek().map_or(false, |c| c.is_ascii_digit()) { width_s.push(chars.next().unwrap()); }")?;
        w(f, "                let zero_pad = width_s.starts_with('0') && width_s.len() > 1;")?;
        w(f, "                let pad: usize = width_s.parse().unwrap_or(0);")?;
        w(f, "                if let Some(spec) = chars.next() { match spec {")?;
        w(f, "                    '%' => result.push('%'),")?;
        w(f, "                    't' | 'T' => { let s = format!(\"{}\", self.time); let padded = if pad > s.len() { if zero_pad { format!(\"{:0>w$}\", s, w=pad) } else { format!(\"{:>w$}\", s, w=pad) } } else { s }; result.push_str(&padded); ai += 1; }")?;
        w(f, "                    _ => if ai < args.len() {")?;
        w(f, "                        let s = self.format_val(args[ai], spec); ai += 1;")?;
        w(f, "                        let padded = if pad > s.len() { if zero_pad { format!(\"{:0>w$}\", s, w=pad) } else { format!(\"{:>w$}\", s, w=pad) } } else { s };")?;
        w(f, "                        result.push_str(&padded);")?;
        w(f, "                    },")?;
        w(f, "                } }")?;
        w(f, "            } else if c == '\\\\' { match chars.next() {")?;
        w(f, "                Some('n') => result.push('\\n'), Some('t') => result.push('\\t'),")?;
        w(f, "                Some(e) => { result.push('\\\\'); result.push(e); }, None => {},")?;
        w(f, "            } } else { result.push(c); }")?;
        w(f, "        } result")?;
        w(f, "    }")?;
        w(f, "    fn check_monitor(&mut self) {")?;
        w(f, "        if let Some(ref fmt) = self.monitor_fmt.clone() {")?;
        w(f, "            let mut changed = self.monitor_prev.is_empty();")?;
        w(f, "            for &id in &self.monitor_args {")?;
        w(f, "                if id < N && self.monitor_prev.len() > id && (self.val[id] != self.monitor_prev[id]) { changed = true; break; }")?;
        w(f, "            }")?;
        w(f, "            if changed { let s = self.display_format(fmt, &self.monitor_args.clone()); println!(\"{}\", s);")?;
        w(f, "                self.monitor_prev = self.val.to_vec(); }")?;
        w(f, "        }")?;
        w(f, "    }")?;
        w(f, "}")?;
        w(f, "")?;
        Ok(())
    }

    fn emit_settle(&self, f: &mut std::fs::File) -> Result<(), String> {
        let w = |f: &mut std::fs::File, s: &str| writeln!(f, "{}", s).map_err(|e| e.to_string());

        // Split into chunks to avoid LLVM OOM
        let mut chunks: Vec<Vec<String>> = Vec::new();
        let mut current_chunk = Vec::new();
        let chunk_size = 256;

        for ca in &self.elab.continuous_assigns {
            if let Some(code) = self.gen_cont_assign(ca) {
                current_chunk.push(code);
                if current_chunk.len() >= chunk_size {
                    chunks.push(std::mem::take(&mut current_chunk));
                }
            }
        }
        // Also compile always_comb / always @(*) blocks into settle
        for ab in &self.elab.always_blocks {
            use crate::ast::decl::AlwaysKind;
            if matches!(ab.kind, AlwaysKind::AlwaysComb | AlwaysKind::Always) {
                // Check if this is a combinatorial block (not edge-triggered)
                if !matches!(&ab.stmt.kind, StatementKind::TimingControl { control: TimingControl::Event(EventControl::EventExpr(_)), .. }) || matches!(ab.kind, AlwaysKind::AlwaysComb) {
                    if let Some(code) = self.gen_comb_block_settle(ab) {
                        current_chunk.push(code);
                        if current_chunk.len() >= chunk_size {
                            chunks.push(std::mem::take(&mut current_chunk));
                        }
                    }
                }
            }
        }
        if !current_chunk.is_empty() { chunks.push(current_chunk); }

        // Emit per-chunk functions
        for (ci, chunk) in chunks.iter().enumerate() {
            w(f, &format!("#[inline(never)]"))?;
            w(f, &format!("fn settle_chunk_{}(s: &mut SimState) -> bool {{", ci))?;
            w(f, "    let mut changed = false;")?;
            for line in chunk {
                w(f, &format!("    {}", line))?;
            }
            w(f, "    changed")?;
            w(f, "}")?;
        }

        w(f, "fn settle(s: &mut SimState) {")?;
        w(f, "    for _ in 0..100 {")?;
        w(f, "        let mut changed = false;")?;
        for ci in 0..chunks.len() {
            w(f, &format!("        changed |= settle_chunk_{}(s);", ci))?;
        }
        w(f, "        if !changed { break; }")?;
        w(f, "    }")?;
        w(f, "}")?;
        w(f, "")?;
        Ok(())
    }

    fn emit_clock_edge(&self, f: &mut std::fs::File) -> Result<(), String> {
        let w = |f: &mut std::fs::File, s: &str| writeln!(f, "{}", s).map_err(|e| e.to_string());
        w(f, "fn clock_edge(s: &mut SimState) {")?;
        for ab in &self.elab.always_blocks {
            if let Some(code) = self.gen_edge_block(ab) {
                w(f, &format!("    {{ {} }}", code))?;
            }
        }
        w(f, "}")?;
        w(f, "")?;
        Ok(())
    }

    fn emit_main(&self, f: &mut std::fs::File) -> Result<(), String> {
        let w = |f: &mut std::fs::File, s: &str| writeln!(f, "{}", s).map_err(|e| e.to_string());
        w(f, "fn main() {")?;
        w(f, "    let mut s = SimState {")?;
        w(f, "        val: [0u64; N], xz: [!0u64; N],")?;
        w(f, "        prev_val: [0u64; N], prev_xz: [!0u64; N],")?;
        w(f, "        time: 0, finished: false,")?;
        w(f, "        monitor_fmt: None, monitor_args: Vec::new(), monitor_prev: Vec::new(),")?;
        w(f, "        event_queue: BTreeMap::new(),")?;
        w(f, "    };")?;
        w(f, "")?;

        // Initialize supply nets and known-value signals
        for (name, sig) in &self.elab.signals {
            if let Some(&id) = self.sig_to_id.get(name) {
                let (v, x) = sig.value.raw_bits();
                let mask = if sig.width >= 64 { u64::MAX } else { (1u64 << sig.width) - 1 };
                if x & mask != mask {
                    w(f, &format!("    s.set({}, 0x{:x}, 0x{:x}); // {}", id, v & mask, x & mask, name))?;
                }
            }
        }
        w(f, "")?;

        // Flatten all initial blocks into event chains
        let mut all_events: Vec<Vec<EventAction>> = Vec::new();
        for ib in &self.elab.initial_blocks {
            let mut stmts = Vec::new();
            self.flatten_stmt(&ib.stmt, &mut stmts);
            // Split at delays: [immediate..., #d1, stmts..., #d2, stmts...]
            let mut chunks: Vec<(u64, Vec<EventAction>)> = Vec::new();
            let mut current = Vec::new();
            let mut pending_delay: u64 = 0;
            for action in stmts {
                match action {
                    EventAction::Delay(d) => {
                        // Flush current actions as a chunk with the previous pending delay
                        chunks.push((pending_delay, std::mem::take(&mut current)));
                        pending_delay = d;
                    }
                    other => current.push(other),
                }
            }
            // Remaining statements after last delay
            chunks.push((pending_delay, current));

            // First chunk: immediate (emit inline in main)
            // Subsequent chunks: event N scheduled from previous event
            if !chunks.is_empty() {
                let (first_delay, first_stmts) = &chunks[0];
                if *first_delay == 0 {
                    // Immediate: emit inline in main
                    for action in first_stmts {
                        self.emit_action(f, action, "    ")?;
                    }
                    // If there's a second chunk, schedule it directly from main
                    if chunks.len() > 1 {
                        let (delay, ref actions) = chunks[1];
                        let eid = all_events.len();
                        all_events.push(actions.clone());
                        w(f, &format!("    s.event_queue.entry(s.time + {}).or_default().push({});", delay, eid))?;
                        // Chain rest from event[eid]
                        for i in 2..chunks.len() {
                            let (delay, ref actions) = chunks[i];
                            let next_eid = all_events.len();
                            let prev = all_events.last_mut().unwrap();
                            prev.push(EventAction::ScheduleEvent(delay, next_eid));
                            all_events.push(actions.clone());
                        }
                    }
                } else {
                    // The first chunk has a delay; schedule event 0 at that delay
                    let eid = all_events.len();
                    all_events.push(chunks[0].1.clone());
                    w(f, &format!("    s.event_queue.entry(s.time + {}).or_default().push({});", first_delay, eid))?;
                    // Chain remaining chunks as events
                    for i in 1..chunks.len() {
                        let (delay, ref actions) = chunks[i];
                        let next_eid = all_events.len();
                        let prev = all_events.last_mut().unwrap();
                        prev.push(EventAction::ScheduleEvent(delay, next_eid));
                        all_events.push(actions.clone());
                    }
                }
            }
        }

        w(f, "")?;
        w(f, "    // Initial settle")?;
        w(f, "    settle(&mut s);")?;
        w(f, "    s.check_monitor();")?;
        w(f, "")?;

        let clock_id = self.find_clock_signal();

        w(f, "    // Event loop")?;
        w(f, "    while !s.finished {")?;
        w(f, "        let next_eq = s.event_queue.keys().next().copied();")?;
        if let Some(_clk_id) = clock_id {
            let half_period = self.find_clock_period().unwrap_or(5);
            w(f, &format!("        let next_clk = Some(s.time + {});", half_period))?;
        } else {
            w(f, "        let next_clk: Option<u64> = None;")?;
        }
        w(f, "        let next_time = match (next_eq, next_clk) {")?;
        w(f, "            (Some(a), Some(b)) => a.min(b),")?;
        w(f, "            (Some(a), None) => a, (None, Some(b)) => b,")?;
        w(f, "            (None, None) => break,")?;
        w(f, "        };")?;
        w(f, "        s.time = next_time;")?;
        w(f, "        s.snapshot();")?;

        if let Some(clk_id) = clock_id {
            w(f, &format!("        let cv = s.val[{}]; s.set({}, cv ^ 1, 0);", clk_id, clk_id))?;
        }

        w(f, "        if let Some(events) = s.event_queue.remove(&s.time) {")?;
        w(f, "            for eid in events { run_event(&mut s, eid); }")?;
        w(f, "        }")?;
        w(f, "        settle(&mut s);")?;
        w(f, "        clock_edge(&mut s);")?;
        w(f, "        settle(&mut s);")?;
        w(f, "        s.check_monitor();")?;
        w(f, "    }")?;
        w(f, "    eprintln!(\"Simulation finished at time {}\", s.time);")?;
        w(f, "}")?;
        w(f, "")?;

        // Generate run_event dispatch
        w(f, "fn run_event(s: &mut SimState, eid: usize) {")?;
        w(f, "    match eid {")?;
        for (eid, actions) in all_events.iter().enumerate() {
            w(f, &format!("        {} => {{", eid))?;
            for action in actions {
                self.emit_action(f, action, "            ")?;
            }
            w(f, "        }")?;
        }
        w(f, "        _ => {}")?;
        w(f, "    }")?;
        w(f, "}")?;

        Ok(())
    }

    /// Flatten a statement tree into a linear sequence of actions.
    fn flatten_stmt(&self, stmt: &Statement, out: &mut Vec<EventAction>) {
        match &stmt.kind {
            StatementKind::SeqBlock { stmts, .. } => {
                for s in stmts { self.flatten_stmt(s, out); }
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                if let (Some(id), Some(rhs)) = (self.expr_to_id_str(lvalue), self.gen_expr_wide(rvalue)) {
                    out.push(EventAction::Assign(id, rhs));
                }
            }
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                // In initial blocks, treat NBA same as blocking for codegen
                if let (Some(id), Some(rhs)) = (self.expr_to_id_str(lvalue), self.gen_expr_wide(rvalue)) {
                    out.push(EventAction::Assign(id, rhs));
                }
            }
            StatementKind::TimingControl { control: TimingControl::Delay(d), stmt: body } => {
                let delay = self.const_eval_expr(d).unwrap_or(0);
                out.push(EventAction::Delay(delay));
                self.flatten_stmt(body, out);
            }
            StatementKind::TimingControl { control: TimingControl::Event(_), stmt: body } => {
                // @(posedge clk) or @(*) — treat as "wait one clock period"
                let period = self.find_clock_period().unwrap_or(5);
                out.push(EventAction::Delay(period));
                self.flatten_stmt(body, out);
            }
            StatementKind::Repeat { count, body } => {
                let n = self.const_eval_expr(count).unwrap_or(1);
                if let StatementKind::TimingControl { control: TimingControl::Event(_), stmt: inner } = &body.kind {
                    let half = self.find_clock_period().unwrap_or(5);
                    let _full = half * 2;
                    // repeat(N) @(posedge clk): wait for N posedge events.
                    // From clock start (clk=0), first posedge at `half`, then every `full`.
                    // Nth posedge at half + (N-1)*full = half*(2N-1)
                    out.push(EventAction::Delay(half * (2 * n - 1)));
                    self.flatten_stmt(inner, out);
                } else {
                    let period = self.find_clock_period().unwrap_or(5);
                    out.push(EventAction::Delay(n * period));
                    self.flatten_stmt(body, out);
                }
            }
            StatementKind::If {  then_stmt,  .. } => {
                // For initial blocks, try to const-eval the condition
                // Otherwise emit as a runtime conditional event action
                // For now, just flatten the then-branch (best effort)
                self.flatten_stmt(then_stmt, out);
            }
            StatementKind::Expr(expr) => {
                if let ExprKind::SystemCall { name, args } = &expr.kind {
                    match name.as_str() {
                        "$display" => {
                            if let Some(ExprKind::StringLiteral(fmt)) = args.first().map(|a| &a.kind) {
                                let arg_ids: Vec<usize> = args[1..].iter()
                                    .filter_map(|a| self.resolve_syscall_arg(a))
                                    .collect();
                                out.push(EventAction::Display(fmt.clone(), arg_ids));
                            }
                        }
                        "$monitor" => {
                            if let Some(ExprKind::StringLiteral(fmt)) = args.first().map(|a| &a.kind) {
                                let arg_ids: Vec<usize> = args[1..].iter()
                                    .filter_map(|a| self.resolve_syscall_arg(a))
                                    .collect();
                                out.push(EventAction::Monitor(fmt.clone(), arg_ids));
                            }
                        }
                        "$finish" => out.push(EventAction::Finish),
                        _ => {}
                    }
                }
            }
            StatementKind::Null => {}
            _ => {} // Other statement types not yet compiled
        }
    }

    fn resolve_syscall_arg(&self, expr: &Expression) -> Option<usize> {
        // $time is special — return a sentinel
        if let ExprKind::SystemCall { name, .. } = &expr.kind {
            if name == "$time" { return Some(usize::MAX); }
        }
        self.expr_to_id_str(expr)
    }

    fn emit_action(&self, f: &mut std::fs::File, action: &EventAction, indent: &str) -> Result<(), String> {
        let w = |f: &mut std::fs::File, s: String| writeln!(f, "{}", s).map_err(|e| e.to_string());
        match action {
            EventAction::Assign(id, rhs) => {
                w(f, format!("{}{{ let (rv, rx) = {}; s.set({}, rv, rx); }}", indent, rhs, id))?;
            }
            EventAction::Display(fmt, arg_ids) => {
                let escaped = fmt.replace('\\', "\\\\").replace('"', "\\\"");
                let args_str = arg_ids.iter().map(|id| {
                    if *id == usize::MAX { "usize::MAX".into() } else { id.to_string() }
                }).collect::<Vec<_>>().join(",");
                w(f, format!("{}println!(\"{{}}\", s.display_format(\"{}\", &[{}]));", indent, escaped, args_str))?;
            }
            EventAction::Monitor(fmt, arg_ids) => {
                let escaped = fmt.replace('\\', "\\\\").replace('"', "\\\"");
                let args_str = arg_ids.iter().map(|id| {
                    if *id == usize::MAX { "usize::MAX".into() } else { id.to_string() }
                }).collect::<Vec<_>>().join(",");
                w(f, format!("{}s.monitor_fmt = Some(\"{}\".to_string());", indent, escaped))?;
                w(f, format!("{}s.monitor_args = vec![{}];", indent, args_str))?;
            }
            EventAction::Finish => {
                w(f, format!("{}s.finished = true;", indent))?;
            }
            EventAction::ScheduleEvent(delay, eid) => {
                w(f, format!("{}s.event_queue.entry(s.time + {}).or_default().push({});", indent, delay, eid))?;
            }
            EventAction::Delay(_) => {} // should not appear in emitted actions
        }
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // Expression code generation (multi-bit aware)
    // ═══════════════════════════════════════════════════════════════

    /// Generate code returning (val: u64, xz: u64) for any-width expression.
    fn gen_expr_wide(&self, expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.resolve_hier(hier) {
                    Some(format!("s.get({})", id))
                } else {
                    // Try parameters/constants
                    let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                    if let Some(val) = self.elab.parameters.get(name) {
                        let v = val.to_u64().unwrap_or(0);
                        Some(format!("(0x{:x}u64, 0u64)", v))
                    } else {
                        None
                    }
                }
            }
            ExprKind::Number(NumberLiteral::Integer { size, base, value, .. }) => {
                let w = size.unwrap_or(32);
                if value.contains('x') || value.contains('X') {
                    let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                    Some(format!("(0u64, 0x{:x}u64)", mask))
                } else {
                    let r = match base { NumberBase::Binary => 2, NumberBase::Octal => 8, NumberBase::Hex => 16, NumberBase::Decimal => 10 };
                    let v = u64::from_str_radix(&value.replace('_', ""), r).unwrap_or(0);
                    Some(format!("(0x{:x}u64, 0u64)", v))
                }
            }
            ExprKind::Number(NumberLiteral::UnbasedUnsized(c)) => {
                match c {
                    '0' => Some("(0u64, 0u64)".into()),
                    '1' => Some("(1u64, 0u64)".into()),
                    'x' | 'X' => Some("(0u64, !0u64)".into()),
                    _ => Some("(0u64, 0u64)".into()),
                }
            }
            ExprKind::Unary { op, operand } => {
                let inner = self.gen_expr_wide(operand)?;
                match op {
                    UnaryOp::BitNot => Some(format!("{{ let (v, x) = {}; (!v & !x, x) }}", inner)),
                    UnaryOp::LogNot => Some(format!("{{ let (v, x) = {}; (if v == 0 && x == 0 {{ 1u64 }} else {{ 0u64 }}, if x != 0 {{ 1u64 }} else {{ 0u64 }}) }}", inner)),
                    UnaryOp::Minus => Some(format!("{{ let (v, x) = {}; if x != 0 {{ (0,!0u64) }} else {{ ((!v).wrapping_add(1), 0) }} }}", inner)),
                    UnaryOp::BitOr => Some(format!("{{ let (v, x) = {}; (if v != 0 {{ 1u64 }} else {{ 0u64 }}, if x != 0 {{ 1u64 }} else {{ 0u64 }}) }}", inner)),
                    UnaryOp::BitAnd => Some(format!("{{ let (v, x) = {}; let w = WIDTHS[0]; (if v & ((1u64 << w) - 1) == (1u64 << w) - 1 {{ 1u64 }} else {{ 0u64 }}, if x != 0 {{ 1u64 }} else {{ 0u64 }}) }}", inner)),
                    _ => None,
                }
            }
            ExprKind::Binary { op, left, right } => {
                let l = self.gen_expr_wide(left)?;
                let r = self.gen_expr_wide(right)?;
                match op {
                    BinaryOp::BitAnd => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; (lv & rv & !(lx|rx), (lx|rx) & !((!lv & !lx) | (!rv & !rx))) }}", l, r)),
                    BinaryOp::BitOr => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; ((lv|rv) & !(lx&rx), (lx|rx) & !((lv & !lx) | (rv & !rx))) }}", l, r)),
                    BinaryOp::BitXor => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; ((lv^rv) & !(lx|rx), lx|rx) }}", l, r)),
                    // Arithmetic
                    BinaryOp::Add => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0,!0u64) }} else {{ (lv.wrapping_add(rv), 0) }} }}", l, r)),
                    BinaryOp::Sub => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0,!0u64) }} else {{ (lv.wrapping_sub(rv), 0) }} }}", l, r)),
                    BinaryOp::Mul => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0,!0u64) }} else {{ (lv.wrapping_mul(rv), 0) }} }}", l, r)),
                    // Comparison
                    BinaryOp::Eq => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0, 1) }} else {{ (if lv == rv {{ 1u64 }} else {{ 0 }}, 0) }} }}", l, r)),
                    BinaryOp::Neq => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0, 1) }} else {{ (if lv != rv {{ 1u64 }} else {{ 0 }}, 0) }} }}", l, r)),
                    BinaryOp::Lt => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0, 1) }} else {{ (if lv < rv {{ 1u64 }} else {{ 0 }}, 0) }} }}", l, r)),
                    BinaryOp::Leq => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0, 1) }} else {{ (if lv <= rv {{ 1u64 }} else {{ 0 }}, 0) }} }}", l, r)),
                    BinaryOp::Gt => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0, 1) }} else {{ (if lv > rv {{ 1u64 }} else {{ 0 }}, 0) }} }}", l, r)),
                    BinaryOp::Geq => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if lx != 0 || rx != 0 {{ (0, 1) }} else {{ (if lv >= rv {{ 1u64 }} else {{ 0 }}, 0) }} }}", l, r)),
                    // Logic
                    BinaryOp::LogAnd => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; (if (lv != 0 && lx == 0) && (rv != 0 && rx == 0) {{ 1u64 }} else {{ 0 }}, if lx != 0 || rx != 0 {{ 1u64 }} else {{ 0 }}) }}", l, r)),
                    BinaryOp::LogOr => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; (if (lv != 0 && lx == 0) || (rv != 0 && rx == 0) {{ 1u64 }} else {{ 0 }}, 0) }}", l, r)),
                    // Shift
                    BinaryOp::ShiftLeft => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if rx != 0 {{ (0,!0u64) }} else {{ (lv << (rv & 63), lx << (rv & 63)) }} }}", l, r)),
                    BinaryOp::ShiftRight => Some(format!("{{ let (lv,lx) = {}; let (rv,rx) = {}; if rx != 0 {{ (0,!0u64) }} else {{ (lv >> (rv & 63), lx >> (rv & 63)) }} }}", l, r)),
                    _ => None,
                }
            }
            ExprKind::Conditional { condition, then_expr, else_expr } => {
                let c = self.gen_expr_wide(condition)?;
                let t = self.gen_expr_wide(then_expr)?;
                let e = self.gen_expr_wide(else_expr)?;
                Some(format!("{{ let (cv,cx) = {}; if cx & 1 != 0 {{ let (tv,tx) = {}; let (ev,ex) = {}; if tv == ev && tx == ex {{ (tv,tx) }} else {{ (0, !0u64) }} }} else if cv & 1 != 0 {{ {} }} else {{ {} }} }}", c, t, e, t, e))
            }
            ExprKind::Paren(inner) => self.gen_expr_wide(inner),
            ExprKind::Index { expr: base, index } => {
                // Bit-select: base[idx] → extract single bit
                let base_code = self.gen_expr_wide(base)?;
                let idx = self.const_eval_expr(index).or_else(|| {
                    // Try gen_expr_wide for dynamic index
                    None
                })?;
                Some(format!("{{ let (bv, bx) = {}; ((bv >> {}) & 1, (bx >> {}) & 1) }}", base_code, idx, idx))
            }
            ExprKind::Concatenation(parts) => {
                // Build value by shifting and OR-ing parts (MSB first)
                if parts.is_empty() { return Some("(0u64, 0u64)".into()); }
                let mut code = String::from("{ let mut cv = 0u64; let mut cx = 0u64; ");
                // Concatenation: parts[0] is MSB, parts[last] is LSB
                for (i, p) in parts.iter().enumerate() {
                    let pc = self.gen_expr_wide(p)?;
                    let pw = self.infer_expr_width(p);
                    if i == 0 {
                        code.push_str(&format!("{{ let (pv, px) = {}; cv = pv; cx = px; }} ", pc));
                    } else {
                        code.push_str(&format!("{{ let (pv, px) = {}; cv = (cv << {}) | (pv & {}); cx = (cx << {}) | (px & {}); }} ",
                            pc, pw, if pw >= 64 { u64::MAX } else { (1u64 << pw) - 1 }, pw, if pw >= 64 { u64::MAX } else { (1u64 << pw) - 1 }));
                    }
                }
                code.push_str("(cv, cx) }");
                Some(code)
            }
            ExprKind::AssignmentPattern(parts) => {
                // Treat same as concatenation for simple cases
                let exprs = parts.iter().map(|p| p.expr().clone()).collect();
                self.gen_expr_wide(&Expression::new(ExprKind::Concatenation(exprs), expr.span))
            }
            _ => None,
        }
    }

    /// Infer the bit-width of an expression (best-effort).
    fn infer_expr_width(&self, expr: &Expression) -> u32 {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.resolve_hier(hier) { self.sig_widths[id] } else { 1 }
            }
            ExprKind::Number(NumberLiteral::Integer { size, .. }) => size.unwrap_or(32),
            ExprKind::Concatenation(parts) => parts.iter().map(|p| self.infer_expr_width(p)).sum(),
            ExprKind::Paren(inner) => self.infer_expr_width(inner),
            _ => 32,
        }
    }

    fn gen_cont_assign(&self, ca: &ContinuousAssignment) -> Option<String> {
        // Handle bit-select LHS: signal[idx] = expr
        if let ExprKind::Index { expr: base, index } = &ca.lhs.kind {
            let lhs_id = self.expr_to_id_str(base)?;
            let bit_idx = self.const_eval_expr(index)?;
            let rhs_code = self.gen_expr_wide(&ca.rhs)?;
            let mask = 1u64 << bit_idx;
            return Some(format!(
                "{{ let (rv, rx) = {rhs}; let ov = s.val[{id}]; let ox = s.xz[{id}]; \
                 let nv = (ov & !{mask}u64) | ((rv & 1) << {bit}); \
                 let nx = (ox & !{mask}u64) | ((rx & 1) << {bit}); \
                 s.set({id}, nv, nx); if s.val[{id}] != ov || s.xz[{id}] != ox {{ changed = true; }} }}",
                rhs = rhs_code, id = lhs_id, mask = mask, bit = bit_idx));
        }
        let lhs_id = self.expr_to_id_str(&ca.lhs)?;
        let rhs_code = self.gen_expr_wide(&ca.rhs)?;
        Some(format!("{{ let (rv, rx) = {rhs}; let ov = s.val[{id}]; let ox = s.xz[{id}]; s.set({id}, rv, rx); if s.val[{id}] != ov || s.xz[{id}] != ox {{ changed = true; }} }}",
            rhs = rhs_code, id = lhs_id))
    }

    /// Generate code for an always_comb block in the settle function.
    fn gen_comb_block_settle(&self, ab: &AlwaysBlock) -> Option<String> {
        let body = match &ab.stmt.kind {
            StatementKind::TimingControl { control: TimingControl::Event(_), stmt } => stmt.as_ref(),
            _ => &ab.stmt,
        };
        let code = self.gen_settle_stmt(body)?;
        Some(format!("{{ {} }}", code))
    }

    /// Generate code for a statement inside a settle (always_comb) context.
    /// Returns code that sets `changed = true` when signals change.
    fn gen_settle_stmt(&self, stmt: &Statement) -> Option<String> {
        match &stmt.kind {
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                // Handle concatenation LHS: {a, b} = expr
                if let ExprKind::Concatenation(parts) = &lvalue.kind {
                    return self.gen_concat_assign_settle(parts, rvalue);
                }
                let lhs_id = self.expr_to_id_str(lvalue)?;
                let rhs = self.gen_expr_wide(rvalue)?;
                Some(format!("{{ let (rv, rx) = {rhs}; let ov = s.val[{id}]; let ox = s.xz[{id}]; s.set({id}, rv, rx); if s.val[{id}] != ov || s.xz[{id}] != ox {{ changed = true; }} }}",
                    rhs = rhs, id = lhs_id))
            }
            StatementKind::SeqBlock { stmts, .. } => {
                let mut parts = Vec::new();
                for s in stmts {
                    if let Some(code) = self.gen_settle_stmt(s) {
                        parts.push(code);
                    }
                }
                if parts.is_empty() { None } else { Some(parts.join(" ")) }
            }
            StatementKind::If { condition, then_stmt, else_stmt, .. } => {
                let c = self.gen_expr_wide(condition)?;
                let t = self.gen_settle_stmt(then_stmt)?;
                let e = else_stmt.as_ref().and_then(|s| self.gen_settle_stmt(s)).unwrap_or_default();
                Some(format!("{{ let (cv,cx) = {}; if cv & 1 != 0 && cx & 1 == 0 {{ {} }} else if cv & 1 == 0 && cx & 1 == 0 {{ {} }} }}", c, t, e))
            }
            StatementKind::Case { expr, items, .. } => {
                let sel = self.gen_expr_wide(expr)?;
                let mut code = format!("{{ let (sv,sx) = {}; ", sel);
                let mut has_non_default = false;
                let mut default_body = None;
                for item in items {
                    if item.is_default {
                        default_body = self.gen_settle_stmt(&item.stmt);
                    } else {
                        for (pi, pat) in item.patterns.iter().enumerate() {
                            let pv = self.gen_expr_wide(pat)?;
                            let prefix = if !has_non_default && pi == 0 { "if" } else { "else if" };
                            let body = self.gen_settle_stmt(&item.stmt)?;
                            code.push_str(&format!("{} {{ let (pv,px) = {}; sx == 0 && px == 0 && sv == pv }} {{ {} }} ", prefix, pv, body));
                            has_non_default = true;
                        }
                    }
                }
                if let Some(db) = default_body {
                    if has_non_default {
                        code.push_str(&format!("else {{ {} }} ", db));
                    } else {
                        code.push_str(&format!("{{ {} }} ", db));
                    }
                }
                code.push('}');
                Some(code)
            }
            StatementKind::Null => Some(String::new()),
            _ => None,
        }
    }

    /// Generate concatenation assign: {a, b} = {expr1, expr2}
    fn gen_concat_assign_settle(&self, lhs_parts: &[Expression], rhs: &Expression) -> Option<String> {
        if let ExprKind::Concatenation(rhs_parts) = &rhs.kind {
            if lhs_parts.len() == rhs_parts.len() {
                let mut code = Vec::new();
                for (l, r) in lhs_parts.iter().zip(rhs_parts.iter()) {
                    let lid = self.expr_to_id_str(l)?;
                    let rcode = self.gen_expr_wide(r)?;
                    code.push(format!("{{ let (rv, rx) = {}; let ov = s.val[{id}]; let ox = s.xz[{id}]; s.set({id}, rv, rx); if s.val[{id}] != ov || s.xz[{id}] != ox {{ changed = true; }} }}",
                        rcode, id = lid));
                }
                return Some(code.join(" "));
            }
        }
        // Fallback: evaluate RHS, split bits across LHS parts
        let rhs_code = self.gen_expr_wide(rhs)?;
        let mut code = format!("{{ let (rv, rx) = {}; ", rhs_code);
        let mut bit_offset = 0u32;
        // Iterate in reverse (LSB first in concatenation)
        for l in lhs_parts.iter().rev() {
            let lid = self.expr_to_id_str(l)?;
            let w = self.sig_widths[lid];
            let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            code.push_str(&format!("{{ let sv = (rv >> {}) & 0x{:x}; let sx = (rx >> {}) & 0x{:x}; let ov = s.val[{id}]; let ox = s.xz[{id}]; s.set({id}, sv, sx); if s.val[{id}] != ov || s.xz[{id}] != ox {{ changed = true; }} }} ",
                bit_offset, mask, bit_offset, mask, id = lid));
            bit_offset += w;
        }
        code.push('}');
        Some(code)
    }

    fn gen_edge_block(&self, ab: &AlwaysBlock) -> Option<String> {
        match &ab.stmt.kind {
            StatementKind::TimingControl { control: TimingControl::Event(ec), stmt } => {
                let mut edge_checks = Vec::new();
                match ec {
                    EventControl::EventExpr(exprs) => {
                        for ev in exprs {
                            let name = expr_to_signal_name(&ev.expr)?;
                            let id = *self.sig_to_id.get(&name)?;
                            let edge = ev.edge.as_ref();
                            match edge {
                                Some(crate::ast::stmt::Edge::Posedge) | None =>
                                    edge_checks.push(format!("s.posedge({})", id)),
                                Some(crate::ast::stmt::Edge::Negedge) =>
                                    edge_checks.push(format!("s.negedge({})", id)),
                                Some(crate::ast::stmt::Edge::Edge) =>
                                    edge_checks.push(format!("(s.posedge({}) || s.negedge({}))", id, id)),
                            }
                        }
                    }
                    EventControl::Identifier(id) => {
                        let sig_id = *self.sig_to_id.get(&id.name)?;
                        edge_checks.push(format!("s.posedge({})", sig_id));
                    }
                    EventControl::HierIdentifier(expr) => {
                        let name = expr_to_signal_name(expr)?;
                        let id = *self.sig_to_id.get(&name)?;
                        edge_checks.push(format!("s.posedge({})", id));
                    }
                    _ => return None,
                };
                if edge_checks.is_empty() { return None; }
                let trigger = edge_checks.join(" || ");

                // Collect NBA evaluations and non-NBA statements separately
                let mut nba_evals = Vec::new();
                let mut other_stmts = Vec::new();
                self.split_nba(stmt, &mut nba_evals, &mut other_stmts);

                let mut body_parts = Vec::new();
                for (i, (_, rhs)) in nba_evals.iter().enumerate() {
                    body_parts.push(format!("let nba{} = {};", i, rhs));
                }
                body_parts.extend(other_stmts);
                for (i, (lhs_id, _)) in nba_evals.iter().enumerate() {
                    body_parts.push(format!("{{ let (rv,rx) = nba{}; s.set({}, rv, rx); }}", i, lhs_id));
                }

                let body_code = body_parts.join(" ");
                Some(format!("if {} {{ {} }}", trigger, body_code))
            }
            _ => None,
        }
    }

    /// Split a statement tree into NBA evaluations and other statements.
    fn split_nba(&self, stmt: &Statement, nbas: &mut Vec<(usize, String)>, others: &mut Vec<String>) {
        match &stmt.kind {
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                if let (Some(id), Some(rhs)) = (self.expr_to_id_str(lvalue), self.gen_expr_wide(rvalue)) {
                    nbas.push((id, rhs));
                }
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                if let (Some(id), Some(rhs)) = (self.expr_to_id_str(lvalue), self.gen_expr_wide(rvalue)) {
                    others.push(format!("{{ let (rv,rx) = {}; s.set({}, rv, rx); }}", rhs, id));
                }
            }
            StatementKind::SeqBlock { stmts, .. } => {
                for s in stmts { self.split_nba(s, nbas, others); }
            }
            StatementKind::If {    .. } => {
                // For if/else inside edge blocks, emit as inline conditional
                if let Some(code) = self.gen_stmt_inline(stmt) {
                    others.push(code);
                }
            }
            StatementKind::Expr(_expr) => {
                if let Some(code) = self.gen_stmt_inline(stmt) {
                    others.push(code);
                }
            }
            StatementKind::Null => {}
            _ => {
                if let Some(code) = self.gen_stmt_inline(stmt) {
                    others.push(code);
                }
            }
        }
    }

    fn gen_stmt_inline(&self, stmt: &Statement) -> Option<String> {
        match &stmt.kind {
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } |
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let lid = self.expr_to_id_str(lvalue)?;
                let rhs = self.gen_expr_wide(rvalue)?;
                Some(format!("{{ let (rv,rx) = {}; s.set({}, rv, rx); }}", rhs, lid))
            }
            StatementKind::If { condition, then_stmt, else_stmt, .. } => {
                let c = self.gen_expr_wide(condition)?;
                let t = self.gen_stmt_inline(then_stmt)?;
                let e = else_stmt.as_ref().and_then(|s| self.gen_stmt_inline(s)).unwrap_or_default();
                Some(format!("{{ let (cv,cx) = {}; if cv & 1 != 0 && cx & 1 == 0 {{ {} }} else if cv & 1 == 0 && cx & 1 == 0 {{ {} }} }}", c, t, e))
            }
            StatementKind::SeqBlock { stmts, .. } => {
                let parts: Vec<String> = stmts.iter().filter_map(|s| self.gen_stmt_inline(s)).collect();
                Some(parts.join(" "))
            }
            StatementKind::Expr(expr) => {
                if let ExprKind::SystemCall { name, args } = &expr.kind {
                    match name.as_str() {
                        "$display" => {
                            if let Some(ExprKind::StringLiteral(fmt)) = args.first().map(|a| &a.kind) {
                                let arg_ids: Vec<String> = args[1..].iter()
                                    .filter_map(|a| self.resolve_syscall_arg(a))
                                    .map(|id| if id == usize::MAX { "usize::MAX".into() } else { id.to_string() })
                                    .collect();
                                let escaped = fmt.replace('\\', "\\\\").replace('"', "\\\"");
                                Some(format!("println!(\"{{}}\", s.display_format(\"{}\", &[{}]));", escaped, arg_ids.join(",")))
                            } else { None }
                        }
                        "$finish" => Some("s.finished = true;".to_string()),
                        _ => Some(String::new()), // skip unknown system calls
                    }
                } else { None }
            }
            StatementKind::Null => Some(String::new()),
            _ => None,
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Helpers
    // ═══════════════════════════════════════════════════════════════

    fn resolve_hier(&self, hier: &HierarchicalIdentifier) -> Option<usize> {
        let name = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
        self.sig_to_id.get(&name).copied()
    }

    fn expr_to_id_str(&self, expr: &Expression) -> Option<usize> {
        match &expr.kind {
            ExprKind::Ident(hier) => self.resolve_hier(hier),
            _ => None,
        }
    }

    fn const_eval_expr(&self, expr: &Expression) -> Option<u64> {
        match &expr.kind {
            ExprKind::Number(NumberLiteral::Integer { value, base, .. }) => {
                let r = match base { NumberBase::Decimal => 10, NumberBase::Hex => 16, NumberBase::Binary => 2, NumberBase::Octal => 8 };
                u64::from_str_radix(&value.replace('_', ""), r).ok()
            }
            _ => None,
        }
    }

    fn find_clock_signal(&self) -> Option<usize> {
        // Check if there's an always #N clk = ~clk pattern
        for ab in &self.elab.always_blocks {
            if let StatementKind::TimingControl { control: TimingControl::Delay(_), stmt } = &ab.stmt.kind {
                if let StatementKind::BlockingAssign { lvalue, .. } = &stmt.kind {
                    if let Some(id) = self.expr_to_id_str(lvalue) {
                        return Some(id);
                    }
                }
            }
        }
        None
    }

    fn find_clock_period(&self) -> Option<u64> {
        for ab in &self.elab.always_blocks {
            if let StatementKind::TimingControl { control: TimingControl::Delay(d), .. } = &ab.stmt.kind {
                return self.const_eval_expr(d);
            }
        }
        None
    }
}

fn expr_to_signal_name(expr: &Expression) -> Option<String> {
    match &expr.kind {
        ExprKind::Ident(hier) => Some(hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".")),
        _ => None,
    }
}

fn san(name: &str) -> String {
    name.replace('.', "__").replace('[', "_").replace(']', "").replace(' ', "_")
}
