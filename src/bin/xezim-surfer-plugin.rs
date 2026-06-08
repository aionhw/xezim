use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use xezim::compiler::simulator::SignalMetadata;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TimeScale {
    unit: TimeUnit,
    multiplier: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
enum TimeUnit {
    ZeptoSeconds,
    AttoSeconds,
    FemtoSeconds,
    PicoSeconds,
    NanoSeconds,
    MicroSeconds,
    MilliSeconds,
    Seconds,
    None,
    Auto,
}

#[derive(Serialize, Deserialize, Debug)]
enum SimulatorToSurferMessage {
    SimulatorInfo {
        name: String,
        version: String,
    },
    Hierarchy {
        hierarchy: Vec<HierarchyElement>,
        time_scale: TimeScale,
    },
    Acknowledge {
        success: bool,
    },
    ValueChanges {
        time_steps: Vec<TimeStep>,
        complete: bool,
    },
    CurrentSimulationTime {
        time: i64,
    },
}

#[derive(Serialize, Deserialize, Debug)]
enum SurferToSimulatorMessage {
    RequestSimulatorInfo,
    RunSimulation { time: Option<u64> },
    PauseSimulation,
    RequestHierarchy,
    TrackVariableChanges { id: u64 },
    UntrackVariableChanges { id: u64 },
    GetValueChanges,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
enum HierarchyElement {
    PushScope {
        name: String,
        scope_type: ScopeType,
    },
    PopScope,
    AddVar {
        id: u64,
        name: String,
        index: Option<VarIndex>,
        scope: Option<String>,
        type_name: String,
        var_type: VarType,
        signal_encoding: SignalEncoding,
        var_direction: Option<VarDirection>,
        enum_map: Option<Vec<(String, String)>>,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
enum ScopeType {
    Module,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
enum VarType {
    Wire,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
enum SignalEncoding {
    BitVector(u32),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
enum VarDirection {
    Input,
    Output,
    InOut,
    Buffer,
    Linkage,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
struct VarIndex {
    msb: i64,
    lsb: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TimeStep {
    timestamp: i64,
    changes: Vec<SignalChange>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SignalChange {
    id: u64,
    value: SignalValue,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
enum SignalValue {
    VCDValue(Vec<u8>),
    Real(f64),
    FSTValue(Vec<u8>),
}

#[derive(Debug)]
struct Args {
    port: u16,
    top_module: Option<String>,
    include_dirs: Vec<String>,
    defines: Vec<(String, Option<String>)>,
    source_files: Vec<String>,
    plusargs: Vec<String>,
    max_time: u64,
}

struct Session {
    hierarchy: Vec<HierarchyElement>,
    sim: Option<Mutex<xezim::compiler::Simulator>>,
}

fn parse_args() -> Result<Args, String> {
    let mut port = 6967;
    let mut top_module = None;
    let mut include_dirs = Vec::new();
    let mut defines = Vec::new();
    let mut source_files = Vec::new();
    let mut plusargs = Vec::new();
    let mut max_time = 100_000;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" | "-p" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a port value"))?;
                port = value
                    .parse::<u16>()
                    .map_err(|e| format!("invalid port '{value}': {e}"))?;
            }
            "--top" | "-s" => {
                top_module = Some(
                    args.next()
                        .ok_or_else(|| format!("{arg} requires a top module value"))?,
                );
            }
            "--include" | "-I" => {
                include_dirs.push(
                    args.next()
                        .ok_or_else(|| format!("{arg} requires a directory value"))?,
                );
            }
            "--define" | "-D" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a define value"))?;
                push_define(&value, &mut defines);
            }
            "--max-time" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a time value"))?;
                max_time = value
                    .parse::<u64>()
                    .map_err(|e| format!("invalid max time '{value}': {e}"))?;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: xezim-surfer-plugin [--port PORT] [-s TOP] [-I DIR] [-D NAME[=VALUE]] [--max-time N] <source_files> [plusargs]"
                );
                std::process::exit(0);
            }
            other if other.starts_with("-I") && other.len() > 2 => {
                include_dirs.push(other[2..].to_string());
            }
            other if other.starts_with("-D") && other.len() > 2 => {
                push_define(&other[2..], &mut defines);
            }
            other if other.starts_with("+incdir+") => {
                for dir in other[8..].split('+').filter(|s| !s.is_empty()) {
                    include_dirs.push(dir.to_string());
                }
            }
            other if other.starts_with("+define+") => {
                for define in other[8..].split('+').filter(|s| !s.is_empty()) {
                    push_define(define, &mut defines);
                }
            }
            other if other.starts_with('+') => plusargs.push(other.to_string()),
            other if other.starts_with('-') => return Err(format!("unknown argument '{other}'")),
            other => source_files.push(other.to_string()),
        }
    }
    Ok(Args {
        port,
        top_module,
        include_dirs,
        defines,
        source_files,
        plusargs,
        max_time,
    })
}

fn push_define(value: &str, defines: &mut Vec<(String, Option<String>)>) {
    if let Some((name, val)) = value.split_once('=') {
        defines.push((name.to_string(), Some(val.to_string())));
    } else {
        defines.push((value.to_string(), None));
    }
}

fn build_session(args: &Args) -> Result<Session, String> {
    if args.source_files.is_empty() {
        return Ok(Session {
            hierarchy: Vec::new(),
            sim: None,
        });
    }

    let mut sources = Vec::with_capacity(args.source_files.len());
    for source_file in &args.source_files {
        sources.push(
            std::fs::read_to_string(source_file)
                .map_err(|e| format!("read source file '{source_file}': {e}"))?,
        );
    }

    let mut sim = xezim::compile_multi(
        &sources,
        args.max_time,
        args.top_module.as_deref(),
        &args.include_dirs,
        &args.source_files,
        &args.defines,
    )?;
    sim.set_plusargs(&args.plusargs);

    let signal_metadata = sim.signal_metadata();
    Ok(Session {
        hierarchy: build_hierarchy(sim.top_module_name(), &signal_metadata),
        sim: Some(Mutex::new(sim)),
    })
}

#[derive(Default)]
struct ScopeNode {
    children: BTreeMap<String, ScopeNode>,
    vars: Vec<SignalMetadata>,
}

fn build_hierarchy(top_name: &str, signals: &[SignalMetadata]) -> Vec<HierarchyElement> {
    let mut root = ScopeNode::default();
    for signal in signals {
        let parts: Vec<_> = signal.name.split('.').collect();
        let (scope_parts, leaf) = if let Some((leaf, scope_parts)) = parts.split_last() {
            (scope_parts, *leaf)
        } else {
            continue;
        };

        let mut node = &mut root;
        for scope in scope_parts {
            node = node.children.entry((*scope).to_string()).or_default();
        }

        let mut signal = signal.clone();
        signal.name = leaf.to_string();
        node.vars.push(signal);
    }

    let mut hierarchy = Vec::new();
    hierarchy.push(HierarchyElement::PushScope {
        name: top_name.to_string(),
        scope_type: ScopeType::Module,
    });
    emit_scope_node(&root, &mut hierarchy);
    hierarchy.push(HierarchyElement::PopScope);
    hierarchy
}

fn emit_scope_node(node: &ScopeNode, hierarchy: &mut Vec<HierarchyElement>) {
    let mut vars = node.vars.clone();
    vars.sort_by(|a, b| a.name.cmp(&b.name));
    for signal in vars {
        let width = if signal.is_real {
            64
        } else {
            signal.width.max(1)
        };
        let type_name = signal.type_name.unwrap_or_else(|| {
            if signal.is_real {
                "real".to_string()
            } else if signal.is_signed {
                "signed logic".to_string()
            } else {
                "logic".to_string()
            }
        });
        hierarchy.push(HierarchyElement::AddVar {
            id: signal.id as u64,
            name: signal.name,
            index: (width > 1).then_some(VarIndex {
                msb: i64::from(width - 1),
                lsb: 0,
            }),
            scope: None,
            type_name,
            var_type: VarType::Wire,
            signal_encoding: SignalEncoding::BitVector(width),
            var_direction: None,
            enum_map: None,
        });
    }

    for (name, child) in &node.children {
        hierarchy.push(HierarchyElement::PushScope {
            name: name.clone(),
            scope_type: ScopeType::Module,
        });
        emit_scope_node(child, hierarchy);
        hierarchy.push(HierarchyElement::PopScope);
    }
}

fn read_frame(stream: &mut TcpStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match stream.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let len = u32::from_be_bytes(len) as usize;
    let mut frame = vec![0u8; len];
    stream.read_exact(&mut frame)?;
    Ok(Some(frame))
}

fn write_frame(stream: &mut TcpStream, frame: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(frame.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large"))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(frame)?;
    stream.flush()
}

fn send_message(stream: &mut TcpStream, msg: &SimulatorToSurferMessage) -> Result<(), String> {
    let encoded = bincode::serialize(msg).map_err(|e| format!("serialize response: {e}"))?;
    write_frame(stream, &encoded).map_err(|e| format!("write response: {e}"))
}

fn value_changes_for_tracked(
    session: &Session,
    tracked_ids: &HashSet<u64>,
) -> Result<Vec<TimeStep>, String> {
    let Some(sim) = &session.sim else {
        return Ok(Vec::new());
    };
    let sim = sim
        .lock()
        .map_err(|_| "simulator session lock poisoned".to_string())?;
    value_changes_for_tracked_locked(&sim, tracked_ids)
}

fn value_changes_for_tracked_locked(
    sim: &xezim::compiler::Simulator,
    tracked_ids: &HashSet<u64>,
) -> Result<Vec<TimeStep>, String> {
    let mut ids: Vec<_> = tracked_ids.iter().copied().collect();
    ids.sort_unstable();

    let changes: Vec<_> = ids
        .into_iter()
        .filter_map(|id| {
            let value = sim.signal_value_by_id(id as usize)?;
            let value = xezim::compiler::vcd_sink::format_vcd_value_bytes(&value);
            Some(SignalChange {
                id,
                value: SignalValue::VCDValue(value),
            })
        })
        .collect();

    if changes.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![TimeStep {
            timestamp: sim.time as i64,
            changes,
        }])
    }
}

fn run_simulation(
    session: &Session,
    time: Option<u64>,
    tracked_ids: &HashSet<u64>,
) -> Result<(Vec<TimeStep>, bool), String> {
    let Some(sim) = &session.sim else {
        return Ok((Vec::new(), true));
    };
    let mut sim = sim
        .lock()
        .map_err(|_| "simulator session lock poisoned".to_string())?;
    if let Some(time) = time {
        sim.max_time = time;
    }
    if !sim.finished {
        sim.simulate();
    }
    let complete = sim.finished;
    let time_steps = value_changes_for_tracked_locked(&sim, tracked_ids)?;
    Ok((time_steps, complete))
}

fn handle_client(mut stream: TcpStream, session: Arc<Session>) -> Result<(), String> {
    let mut tracked_ids = HashSet::new();
    loop {
        let Some(frame) = read_frame(&mut stream).map_err(|e| format!("read frame: {e}"))? else {
            return Ok(());
        };
        let msg: SurferToSimulatorMessage =
            bincode::deserialize(&frame).map_err(|e| format!("decode request: {e}"))?;

        let response = match msg {
            SurferToSimulatorMessage::RequestSimulatorInfo => {
                SimulatorToSurferMessage::SimulatorInfo {
                    name: "xezim".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                }
            }
            SurferToSimulatorMessage::RequestHierarchy => SimulatorToSurferMessage::Hierarchy {
                hierarchy: session.hierarchy.clone(),
                time_scale: TimeScale {
                    unit: TimeUnit::NanoSeconds,
                    multiplier: Some(1),
                },
            },
            SurferToSimulatorMessage::RunSimulation { time } => {
                let (time_steps, complete) = run_simulation(&session, time, &tracked_ids)?;
                SimulatorToSurferMessage::ValueChanges {
                    time_steps,
                    complete,
                }
            }
            SurferToSimulatorMessage::PauseSimulation => {
                SimulatorToSurferMessage::Acknowledge { success: true }
            }
            SurferToSimulatorMessage::TrackVariableChanges { id } => {
                tracked_ids.insert(id);
                SimulatorToSurferMessage::Acknowledge { success: true }
            }
            SurferToSimulatorMessage::UntrackVariableChanges { id } => {
                tracked_ids.remove(&id);
                SimulatorToSurferMessage::Acknowledge { success: true }
            }
            SurferToSimulatorMessage::GetValueChanges => SimulatorToSurferMessage::ValueChanges {
                time_steps: value_changes_for_tracked(&session, &tracked_ids)?,
                complete: true,
            },
        };

        send_message(&mut stream, &response)?;
    }
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let session = match build_session(&args) {
        Ok(session) => Arc::new(session),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let addr = format!("127.0.0.1:{}", args.port);
    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| {
        eprintln!("failed to bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!("xezim Surfer plugin listening on {addr}");
    eprintln!(
        "xezim Surfer plugin hierarchy contains {} elements and simulator_loaded={}",
        session.hierarchy.len(),
        session.sim.is_some()
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_client(stream, Arc::clone(&session)) {
                    eprintln!("client error: {e}");
                }
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}
