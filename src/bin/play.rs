use anyhow::{Context, anyhow, bail};
use fleet_sim::{
    Cargo, FleetCommand, Init, InitCommand, MutCommand, Tick, TopCommand, VesselLocation,
    ViewCommand,
    sim::{Fleet, Operation, Status, View, WorldState},
};
use serde::Serialize;
use std::{
    env, fs,
    io::{self, BufRead},
    path::Path,
};

const HELP: &str = r#"Fleet simulator command grammar:

  init ROWS VESSELS THROUGHPUT [CARGO]
  tick [CARGO]
  get_state | state
  get_state_at TICK | state_at TICK
  set_destination VESSEL DESTINATION | set_dest VESSEL DESTINATION
  dock VESSEL
  undock VESSEL
  load VESSEL DESTINATION
  unload VESSEL DESTINATION
  swap VESSEL LOAD_DESTINATION UNLOAD_DESTINATION
  help

Value formats:
  ROWS        comma-separated travel times; semicolon-separated rows
  VESSELS     comma-separated ID:LOCATION entries
  THROUGHPUT  comma-separated dock throughput values
  CARGO       comma-separated LOCATION:DESTINATION:QUANTITY entries

Use - for an empty list. Blank lines and lines beginning with # are ignored.
Command results are written as JSON to stdout; this help is written to stderr.

Pass a file containing one `init` command to initialize before reading stdin:
  cargo run --bin play -- FLEET_FILE"#;

#[derive(Serialize)]
#[serde(tag = "status", content = "payload", rename_all = "lowercase")]
enum PlayStatus<'a> {
    Ok(PlayState<'a>),
    #[serde(rename = "error")]
    Err {
        message: String,
    },
}

#[derive(Serialize)]
struct PlayState<'a> {
    #[serde(flatten)]
    state: WorldState<'a>,
    ops: Vec<Operation<'a>>,
}

fn play_status(fleet: &Fleet, view: View) -> PlayStatus<'_> {
    let ops = if view == View::Planned {
        fleet.operations()
    } else {
        Vec::new()
    };
    match fleet.status(view) {
        Status::Ok(state) => PlayStatus::Ok(PlayState { state, ops }),
        Status::Err { message } => PlayStatus::Err { message },
    }
}

fn numbers(value: &str, name: &str) -> anyhow::Result<Vec<usize>> {
    if value == "-" {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|part| {
            part.parse()
                .with_context(|| format!("invalid {name} value `{part}`"))
        })
        .collect()
}

fn cargo(value: &str) -> anyhow::Result<Vec<Cargo>> {
    if value == "-" {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|item| {
            let fields: Vec<_> = item.split(':').collect();
            if fields.len() != 3 {
                bail!("cargo `{item}` must be LOCATION:DESTINATION:QUANTITY");
            }
            Ok(Cargo {
                dock: fields[0].parse().context("invalid cargo location")?,
                dest: fields[1].parse().context("invalid cargo destination")?,
                quantity: fields[2].parse().context("invalid cargo quantity")?,
            })
        })
        .collect()
}

fn parse(line: &str) -> anyhow::Result<TopCommand> {
    let fields: Vec<_> = line.split_whitespace().collect();
    let Some(command) = fields.first().copied() else {
        bail!("empty command");
    };

    let view = |command| Ok(TopCommand::Fleet(FleetCommand::View(command)));
    let fleet = |command| Ok(TopCommand::Fleet(FleetCommand::Planned(command)));
    match command.to_ascii_lowercase().as_str() {
        "init" => {
            if !(4..=5).contains(&fields.len()) {
                bail!("usage: init ROWS VESSELS THROUGHPUT [CARGO]");
            }
            let travel_times = fields[1]
                .split(';')
                .map(|row| numbers(row, "travel time"))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let vessels = if fields[2] == "-" {
                Vec::new()
            } else {
                fields[2]
                    .split(',')
                    .map(|item| {
                        let (vessel, location) = item
                            .split_once(':')
                            .ok_or_else(|| anyhow!("vessel `{item}` must be ID:LOCATION"))?;
                        Ok(VesselLocation {
                            vessel: vessel.into(),
                            location: location.parse().context("invalid vessel location")?,
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?
            };
            Ok(TopCommand::Init(InitCommand::Init(Init {
                travel_times,
                vessels,
                dock_throughput: numbers(fields[3], "dock throughput")?,
                cargo_manifest: fields.get(4).map_or(Ok(Vec::new()), |value| cargo(value))?,
            })))
        }
        "tick" => {
            if fields.len() > 2 {
                bail!("usage: tick [CARGO]");
            }
            Ok(TopCommand::Fleet(FleetCommand::Advance(Tick::Tick {
                cargo_manifest: fields.get(1).map_or(Ok(Vec::new()), |value| cargo(value))?,
            })))
        }
        "get_state" | "state" => exact(&fields, 1).and_then(|()| view(ViewCommand::GetState {})),
        "get_state_at" | "state_at" => {
            exact(&fields, 2)?;
            view(ViewCommand::GetStateAt {
                tick: fields[1].parse().context("invalid tick")?,
            })
        }
        "set_destination" | "set_dest" => {
            exact(&fields, 3)?;
            fleet(MutCommand::SetDest {
                vessel_id: fields[1].into(),
                dest: fields[2].parse().context("invalid destination")?,
            })
        }
        "dock" | "undock" => {
            exact(&fields, 2)?;
            let vessel_id = fields[1].into();
            fleet(if command.eq_ignore_ascii_case("dock") {
                MutCommand::Dock { vessel_id }
            } else {
                MutCommand::Undock { vessel_id }
            })
        }
        "load" | "unload" => {
            exact(&fields, 3)?;
            let vessel_id = fields[1].into();
            let dest = fields[2].parse().context("invalid destination")?;
            fleet(if command.eq_ignore_ascii_case("load") {
                MutCommand::Load { vessel_id, dest }
            } else {
                MutCommand::Unload { vessel_id, dest }
            })
        }
        "swap" => {
            exact(&fields, 4)?;
            fleet(MutCommand::Swap {
                vessel_id: fields[1].into(),
                load_dest: fields[2].parse().context("invalid load destination")?,
                unload_dest: fields[3].parse().context("invalid unload destination")?,
            })
        }
        _ => bail!("unknown command `{command}`"),
    }
}

fn exact(fields: &[&str], expected: usize) -> anyhow::Result<()> {
    if fields.len() != expected {
        bail!("`{}` expects {} argument(s)", fields[0], expected - 1);
    }
    Ok(())
}

fn execute(command: TopCommand, sim: &mut Option<Fleet>) -> PlayStatus<'_> {
    match command {
        TopCommand::Init(InitCommand::Init(init)) => {
            if sim.is_some() {
                return PlayStatus::Err {
                    message: "already called init".into(),
                };
            }
            match Fleet::init(init) {
                Ok(fleet) => {
                    *sim = Some(fleet);
                    play_status(sim.as_ref().unwrap(), View::LastCommitted)
                }
                Err(error) => PlayStatus::Err {
                    message: error.to_string(),
                },
            }
        }
        TopCommand::Fleet(command) => {
            let Some(fleet) = sim.as_mut() else {
                return PlayStatus::Err {
                    message: "must call init first".into(),
                };
            };
            let view = match View::for_command(&command) {
                Ok(view) => view,
                Err(message) => return PlayStatus::Err { message },
            };
            match fleet.apply(command) {
                Ok(()) => play_status(fleet, view),
                Err(error) => PlayStatus::Err {
                    message: error.to_string(),
                },
            }
        }
    }
}

fn fleet_from_str(contents: &str) -> anyhow::Result<Fleet> {
    let mut commands = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    let line = commands
        .next()
        .ok_or_else(|| anyhow!("fleet file does not contain an init command"))?;
    if commands.next().is_some() {
        bail!("fleet file must contain exactly one init command");
    }

    match parse(line).context("failed to parse fleet file")? {
        TopCommand::Init(InitCommand::Init(init)) => {
            Fleet::init(init).context("fleet file does not describe a valid fleet")
        }
        TopCommand::Fleet(_) => bail!("fleet file must contain an init command"),
    }
}

fn fleet_from_file(path: &Path) -> anyhow::Result<Fleet> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read fleet file `{}`", path.display()))?;
    fleet_from_str(&contents)
        .with_context(|| format!("failed to initialize from `{}`", path.display()))
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut args = env::args_os().skip(1);
    let fleet_file = args.next();
    if args.next().is_some() {
        bail!("usage: play [FLEET_FILE]");
    }

    let mut sim = fleet_file
        .as_deref()
        .map(Path::new)
        .map(fleet_from_file)
        .transpose()?;
    eprintln!("{HELP}");
    if let Some(fleet) = sim.as_ref() {
        println!(
            "{}",
            serde_json::to_string(&play_status(fleet, View::LastCommitted))?
        );
    }
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.eq_ignore_ascii_case("help") {
            eprintln!("{HELP}");
            continue;
        }
        let status = match parse(line) {
            Ok(command) => execute(command, &mut sim),
            Err(error) => PlayStatus::Err {
                message: error.to_string(),
            },
        };
        println!("{}", serde_json::to_string(&status)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        for line in [
            "init 0,4;4,0 v1:0,v2:1 2,3 0:1:5",
            "tick",
            "tick 0:1:2,1:0:3",
            "get_state",
            "get_state_at 2",
            "set_destination v1 1",
            "dock v1",
            "undock v1",
            "load v1 1",
            "unload v1 1",
            "swap v1 1 0",
        ] {
            parse(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        }
    }

    #[test]
    fn rejects_extra_arguments() {
        assert!(parse("dock v1 extra").is_err());
    }

    #[test]
    fn play_output_includes_pending_operations() -> anyhow::Result<()> {
        let mut sim = None;
        execute(parse("init 0 v1:0 1")?, &mut sim);
        let status = execute(parse("dock v1")?, &mut sim);

        let json = serde_json::to_value(status)?;
        assert_eq!(
            json["payload"]["ops"],
            serde_json::json!([{"operation": "dock", "vessel_id": "v1"}]),
        );
        Ok(())
    }

    #[test]
    fn initializes_fleet_from_text_file_contents() -> anyhow::Result<()> {
        let fleet = fleet_from_str("# initial fleet\n\ninit 0,4;4,0 v1:0 2,3 0:1:5\n")?;
        let json = serde_json::to_value(play_status(&fleet, View::LastCommitted))?;

        assert_eq!(json["payload"]["tick"], 0);
        assert_eq!(json["payload"]["vessels"]["v1"]["location"], 0);
        Ok(())
    }

    #[test]
    fn rejects_invalid_fleet_file_contents() {
        assert!(fleet_from_str("tick").is_err());
        assert!(fleet_from_str("init 0 v1:1 1").is_err());
        assert!(fleet_from_str("init 0 v1:0 1\ntick").is_err());
    }
}
