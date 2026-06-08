use std::io::{Read, Write};
use std::net::TcpStream;

use serde::{Deserialize, Serialize};

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
    addr: String,
    commands: Vec<Command>,
}

#[derive(Clone, Debug)]
enum Command {
    Info,
    Hierarchy,
    Track(u64),
    Untrack(u64),
    Values,
    Run(Option<u64>),
    Pause,
    Smoke,
    Ez,
}

fn parse_args() -> Result<Args, String> {
    let mut addr = "127.0.0.1:6967".to_string();
    let mut commands = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" | "-a" => {
                addr = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires an address value"))?;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: synthetic-surfer [--addr HOST:PORT] [info|hierarchy|track:ID|untrack:ID|values|run[:TIME]|run-end|pause|smoke|ez]..."
                );
                std::process::exit(0);
            }
            other => commands.push(parse_command(other)?),
        }
    }

    if commands.is_empty() {
        commands.push(Command::Smoke);
    }

    Ok(Args { addr, commands })
}

fn parse_command(command: &str) -> Result<Command, String> {
    match command {
        "info" => Ok(Command::Info),
        "hierarchy" => Ok(Command::Hierarchy),
        "values" => Ok(Command::Values),
        "pause" => Ok(Command::Pause),
        "smoke" => Ok(Command::Smoke),
        "ez" | "picorv32-ez" => Ok(Command::Ez),
        "run" => Ok(Command::Run(None)),
        "run-end" => Ok(Command::Run(None)),
        other if other.starts_with("run:") => {
            let time = other[4..]
                .parse::<u64>()
                .map_err(|e| format!("invalid run time '{}': {e}", &other[4..]))?;
            Ok(Command::Run(Some(time)))
        }
        other if other.starts_with("track:") => {
            let id = other[6..]
                .parse::<u64>()
                .map_err(|e| format!("invalid track id '{}': {e}", &other[6..]))?;
            Ok(Command::Track(id))
        }
        other if other.starts_with("untrack:") => {
            let id = other[8..]
                .parse::<u64>()
                .map_err(|e| format!("invalid untrack id '{}': {e}", &other[8..]))?;
            Ok(Command::Untrack(id))
        }
        other => Err(format!("unknown command '{other}'")),
    }
}

fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    let mut frame = vec![0u8; len];
    stream.read_exact(&mut frame)?;
    Ok(frame)
}

fn write_frame(stream: &mut TcpStream, frame: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(frame.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame too large"))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(frame)?;
    stream.flush()
}

fn transact(
    stream: &mut TcpStream,
    msg: &SurferToSimulatorMessage,
) -> Result<SimulatorToSurferMessage, String> {
    let encoded = bincode::serialize(msg).map_err(|e| format!("serialize request: {e}"))?;
    write_frame(stream, &encoded).map_err(|e| format!("write request: {e}"))?;
    let frame = read_frame(stream).map_err(|e| format!("read response: {e}"))?;
    bincode::deserialize(&frame).map_err(|e| format!("decode response: {e}"))
}

fn print_response(label: &str, response: &SimulatorToSurferMessage) {
    match response {
        SimulatorToSurferMessage::SimulatorInfo { name, version } => {
            println!("{label}: simulator {name} {version}");
        }
        SimulatorToSurferMessage::Hierarchy {
            hierarchy,
            time_scale,
        } => {
            let scopes = hierarchy
                .iter()
                .filter(|element| matches!(element, HierarchyElement::PushScope { .. }))
                .count();
            let vars = hierarchy
                .iter()
                .filter(|element| matches!(element, HierarchyElement::AddVar { .. }))
                .count();
            let first_vars: Vec<_> = hierarchy
                .iter()
                .filter_map(|element| match element {
                    HierarchyElement::AddVar { id, name, .. } => Some(format!("{id}:{name}")),
                    _ => None,
                })
                .take(8)
                .collect();
            println!(
                "{label}: hierarchy elements={} scopes={} vars={} timescale={:?} {:?}",
                hierarchy.len(),
                scopes,
                vars,
                time_scale.multiplier,
                time_scale.unit
            );
            if !first_vars.is_empty() {
                println!("{label}: first vars {}", first_vars.join(", "));
            }
        }
        SimulatorToSurferMessage::Acknowledge { success } => {
            println!("{label}: ack success={success}");
        }
        SimulatorToSurferMessage::ValueChanges {
            time_steps,
            complete,
        } => {
            let change_count: usize = time_steps.iter().map(|step| step.changes.len()).sum();
            println!(
                "{label}: value changes steps={} changes={} complete={}",
                time_steps.len(),
                change_count,
                complete
            );
            for step in time_steps.iter().take(4) {
                let sample: Vec<_> = step
                    .changes
                    .iter()
                    .take(8)
                    .map(|change| format!("{}={}", change.id, value_preview(&change.value)))
                    .collect();
                println!(
                    "{label}: t={} {}",
                    step.timestamp,
                    if sample.is_empty() {
                        "<no changes>".to_string()
                    } else {
                        sample.join(", ")
                    }
                );
            }
        }
        SimulatorToSurferMessage::CurrentSimulationTime { time } => {
            println!("{label}: current time {time}");
        }
    }
}

fn value_preview(value: &SignalValue) -> String {
    match value {
        SignalValue::VCDValue(bytes) => String::from_utf8_lossy(bytes).to_string(),
        SignalValue::Real(value) => value.to_string(),
        SignalValue::FSTValue(bytes) => format!("fst:{}B", bytes.len()),
    }
}

fn first_var_id(response: &SimulatorToSurferMessage) -> Option<u64> {
    first_var_ids(response, 1).into_iter().next()
}

fn first_var_ids(response: &SimulatorToSurferMessage, count: usize) -> Vec<u64> {
    match response {
        SimulatorToSurferMessage::Hierarchy { hierarchy, .. } => {
            hierarchy
                .iter()
                .filter_map(|element| match element {
                    HierarchyElement::AddVar { id, .. } => Some(*id),
                    _ => None,
                })
                .take(count)
                .collect()
        }
        _ => Vec::new(),
    }
}

fn run_command(
    stream: &mut TcpStream,
    command: &Command,
    last_hierarchy: &mut Option<SimulatorToSurferMessage>,
) -> Result<(), String> {
    match command {
        Command::Info => {
            let response = transact(stream, &SurferToSimulatorMessage::RequestSimulatorInfo)?;
            print_response("info", &response);
        }
        Command::Hierarchy => {
            let response = transact(stream, &SurferToSimulatorMessage::RequestHierarchy)?;
            print_response("hierarchy", &response);
            *last_hierarchy = Some(response);
        }
        Command::Track(id) => {
            let response = transact(stream, &SurferToSimulatorMessage::TrackVariableChanges {
                id: *id,
            })?;
            print_response(&format!("track:{id}"), &response);
        }
        Command::Untrack(id) => {
            let response = transact(stream, &SurferToSimulatorMessage::UntrackVariableChanges {
                id: *id,
            })?;
            print_response(&format!("untrack:{id}"), &response);
        }
        Command::Values => {
            let response = transact(stream, &SurferToSimulatorMessage::GetValueChanges)?;
            print_response("values", &response);
        }
        Command::Run(time) => {
            let response = transact(stream, &SurferToSimulatorMessage::RunSimulation {
                time: *time,
            })?;
            print_response("run", &response);
        }
        Command::Pause => {
            let response = transact(stream, &SurferToSimulatorMessage::PauseSimulation)?;
            print_response("pause", &response);
        }
        Command::Smoke => {
            run_smoke(stream, last_hierarchy)?;
        }
        Command::Ez => {
            run_picorv32_ez(stream, last_hierarchy)?;
        }
    }
    Ok(())
}

fn run_smoke(
    stream: &mut TcpStream,
    last_hierarchy: &mut Option<SimulatorToSurferMessage>,
) -> Result<(), String> {
    let info = transact(stream, &SurferToSimulatorMessage::RequestSimulatorInfo)?;
    print_response("smoke/info", &info);

    let hierarchy = transact(stream, &SurferToSimulatorMessage::RequestHierarchy)?;
    print_response("smoke/hierarchy", &hierarchy);
    let id = first_var_id(&hierarchy);
    *last_hierarchy = Some(hierarchy);

    let Some(id) = id else {
        println!("smoke: no variable id available to track");
        return Ok(());
    };

    let track = transact(stream, &SurferToSimulatorMessage::TrackVariableChanges { id })?;
    print_response(&format!("smoke/track:{id}"), &track);

    let values = transact(stream, &SurferToSimulatorMessage::GetValueChanges)?;
    print_response("smoke/values", &values);

    let run = transact(stream, &SurferToSimulatorMessage::RunSimulation { time: Some(0) })?;
    print_response("smoke/run:0", &run);

    Ok(())
}

fn run_picorv32_ez(
    stream: &mut TcpStream,
    last_hierarchy: &mut Option<SimulatorToSurferMessage>,
) -> Result<(), String> {
    let info = transact(stream, &SurferToSimulatorMessage::RequestSimulatorInfo)?;
    print_response("ez/info", &info);

    let hierarchy = transact(stream, &SurferToSimulatorMessage::RequestHierarchy)?;
    print_response("ez/hierarchy", &hierarchy);
    let ids = first_var_ids(&hierarchy, 8);
    *last_hierarchy = Some(hierarchy);

    if ids.is_empty() {
        println!("ez: no variable id available to track");
        return Ok(());
    }

    for id in &ids {
        let track = transact(stream, &SurferToSimulatorMessage::TrackVariableChanges {
            id: *id,
        })?;
        print_response(&format!("ez/track:{id}"), &track);
    }

    let run = transact(stream, &SurferToSimulatorMessage::RunSimulation { time: None })?;
    print_response("ez/run-end", &run);
    Ok(())
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };

    let mut stream = TcpStream::connect(&args.addr).unwrap_or_else(|e| {
        eprintln!("failed to connect to {}: {e}", args.addr);
        std::process::exit(1);
    });
    let mut last_hierarchy = None;

    for command in &args.commands {
        if let Err(e) = run_command(&mut stream, command, &mut last_hierarchy) {
            eprintln!("command {:?} failed: {e}", command);
            std::process::exit(1);
        }
    }
}
