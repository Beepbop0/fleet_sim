use anyhow::{Context, anyhow, bail};
use fleet_sim::{
    Cargo, Command, FleetCommand, Init, InitCommand, Tick, TopCommand, VesselLocation,
    sim::{Fleet, Status, View},
};
use std::io::{self, BufRead};

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
        "get_state" | "state" => exact(&fields, 1).and_then(|()| fleet(Command::GetState {})),
        "get_state_at" | "state_at" => {
            exact(&fields, 2)?;
            fleet(Command::GetStateAt {
                tick: fields[1].parse().context("invalid tick")?,
            })
        }
        "set_destination" | "set_dest" => {
            exact(&fields, 3)?;
            fleet(Command::SetDest {
                vessel_id: fields[1].into(),
                dest: fields[2].parse().context("invalid destination")?,
            })
        }
        "dock" | "undock" => {
            exact(&fields, 2)?;
            let vessel_id = fields[1].into();
            fleet(if command.eq_ignore_ascii_case("dock") {
                Command::Dock { vessel_id }
            } else {
                Command::Undock { vessel_id }
            })
        }
        "load" | "unload" => {
            exact(&fields, 3)?;
            let vessel_id = fields[1].into();
            let dest = fields[2].parse().context("invalid destination")?;
            fleet(if command.eq_ignore_ascii_case("load") {
                Command::Load { vessel_id, dest }
            } else {
                Command::Unload { vessel_id, dest }
            })
        }
        "swap" => {
            exact(&fields, 4)?;
            fleet(Command::Swap {
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

fn execute(command: TopCommand, sim: &mut Option<Fleet>) -> Status<'_> {
    match command {
        TopCommand::Init(InitCommand::Init(init)) => {
            if sim.is_some() {
                return Status::Err {
                    message: "already called init".into(),
                };
            }
            match Fleet::init(init) {
                Ok(fleet) => {
                    *sim = Some(fleet);
                    sim.as_ref().unwrap().status(View::LastCommitted)
                }
                Err(error) => Status::Err {
                    message: error.to_string(),
                },
            }
        }
        TopCommand::Fleet(command) => {
            let Some(fleet) = sim.as_mut() else {
                return Status::Err {
                    message: "must call init first".into(),
                };
            };
            let view = match &command {
                FleetCommand::Planned(Command::GetState {} | Command::SetDest { .. }) => {
                    View::Planned
                }
                FleetCommand::Planned(Command::GetStateAt { tick }) => View::CommittedAt(*tick),
                _ => View::LastCommitted,
            };
            match fleet.apply(command) {
                Ok(()) => fleet.status(view),
                Err(error) => Status::Err {
                    message: error.to_string(),
                },
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut sim = None;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let status = match parse(&line) {
            Ok(command) => execute(command, &mut sim),
            Err(error) => Status::Err {
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
}
