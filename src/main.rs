use anyhow::{Context, anyhow, bail};
use fleet_sim::square::Square;
use fleet_sim::{Cargo, Command, VesselLocation};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{self, BufRead};

#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
#[serde(tag = "state")]
#[serde(rename_all = "lowercase")]
enum VesselState {
    Idle {
        location: usize,
    },
    Transit {
        from: usize,
        to: usize,
        arrives_at_tick: usize,
    },
    Docked {
        location: usize,
    },
}

struct Fleet {
    tick: usize,
    travel_times: Square,
    cargo_manifest: Square,
    vessels: HashMap<String, VesselState>,
    staged: HashMap<String, VesselState>,
}

fn apply_manifest(square: &mut Square, manifest: Vec<Cargo>) -> anyhow::Result<()> {
    for &Cargo { dock, dest, .. } in &manifest {
        if dock >= square.n() {
            bail!("unknown location {dock}");
        }
        if dest >= square.n() {
            bail!("unknown destination {dest}");
        }
    }
    for Cargo {
        dock,
        dest,
        quantity,
    } in manifest
    {
        square[dock][dest] += quantity;
    }
    Ok(())
}

struct Sim(Option<Fleet>);

fn apply(sim: &mut Sim, command: Command) -> anyhow::Result<()> {
    use Command::*;
    let Some(fleet) = sim.0.as_mut() else {
        let Init {
            travel_times,
            vessels,
            cargo_manifest,
        } = command
        else {
            bail!("must call init first");
        };

        let travel_times = Square::try_from(travel_times)
            .context("failed to convert travel times to square matrix")?;
        let n = travel_times.n();
        let mut locations = HashMap::new();
        for VesselLocation { vessel, location } in vessels {
            if location >= n {
                bail!("invalid location {location} for vessel {vessel}");
            }
            if locations
                .insert(vessel.clone(), VesselState::Idle { location })
                .is_some()
            {
                bail!("duplicate registration of vessel {vessel}");
            }
        }
        let mut manifest = Square::zeros(n);
        apply_manifest(&mut manifest, cargo_manifest)?;
        sim.0 = Some(Fleet {
            tick: 0,
            travel_times,
            cargo_manifest: manifest,
            vessels: locations.clone(),
            staged: locations,
        });

        return Ok(());
    };

    match command {
        Init { .. } => Err(anyhow!("cannot call init again")),
        Tick { cargo_manifest } => {
            for (vessel, state) in &mut fleet.vessels {
                let next = fleet.staged[vessel];
                *state = next;
            }
            fleet.tick += 1;
            for state in fleet.vessels.values_mut() {
                if let &mut VesselState::Transit {
                    arrives_at_tick,
                    to,
                    ..
                } = state
                    && arrives_at_tick == fleet.tick
                {
                    *state = VesselState::Idle { location: to };
                }
            }
            apply_manifest(&mut fleet.cargo_manifest, cargo_manifest)?;
            Ok(())
        }
        GetState => Ok(()),
        SetDest { vessel, dest } => {
            let n = fleet.travel_times.n();
            if dest >= n {
                bail!("destination can only be set to 0..{n}");
            }
            let tick = fleet.tick;
            let (Some(existing), Some(pending)) =
                (fleet.vessels.get(&vessel), fleet.staged.get_mut(&vessel))
            else {
                bail!("unknown vessel {vessel}");
            };
            *pending = match *existing {
                VesselState::Idle { location: from } => {
                    if from == dest {
                        return Ok(());
                    }
                    let distance = fleet.travel_times[from][dest];
                    VesselState::Transit {
                        from,
                        to: dest,
                        arrives_at_tick: tick + distance,
                    }
                }
                VesselState::Transit {
                    arrives_at_tick,
                    to,
                    ..
                } if arrives_at_tick == tick + 1 => {
                    let distance = fleet.travel_times[to][dest];
                    VesselState::Transit {
                        from: to,
                        to: dest,
                        arrives_at_tick: tick + 1 + distance,
                    }
                }
                _ => bail!("vessel {vessel} is not idle"),
            };
            Ok(())
        }
        Dock { vessel_id: vessel } => {
            let (Some(existing), Some(pending)) =
                (fleet.vessels.get(&vessel), fleet.staged.get_mut(&vessel))
            else {
                bail!("unknown vessel {vessel}");
            };
            *pending = match *existing {
                VesselState::Idle { location } | VesselState::Docked { location } => {
                    VesselState::Docked { location }
                }
                _ => bail!("vessel {vessel} is not idle"),
            };
            Ok(())
        }
        Undock { vessel_id: vessel } => {
            let (Some(existing), Some(pending)) =
                (fleet.vessels.get(&vessel), fleet.staged.get_mut(&vessel))
            else {
                bail!("unknown vessel {vessel}");
            };
            *pending = match *existing {
                VesselState::Docked { location } => VesselState::Idle { location },
                _ => bail!("vessel {vessel} is not docked"),
            };
            Ok(())
        }
    }
}

#[derive(Debug, Serialize)]
struct WorldState<'a> {
    tick: usize,
    vessels: &'a HashMap<String, VesselState>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", content = "payload", rename_all = "lowercase")]
enum Status<'a> {
    Ok(WorldState<'a>),
    Err { message: String },
}

fn main() -> anyhow::Result<()> {
    let mut buf = String::new();
    let mut stdin = io::stdin().lock();
    let mut state = Sim(None);
    loop {
        buf.clear();
        if stdin.read_line(&mut buf)? == 0 {
            break;
        }
        let command: Command = serde_json::from_str(&buf[..])?;
        let show_staged = matches!(&command, Command::GetState | Command::SetDest { .. });
        let status = match apply(&mut state, command) {
            Ok(_) => {
                let Some(Fleet {
                    vessels,
                    staged,
                    tick,
                    ..
                }) = state.0.as_ref()
                else {
                    panic!("received Ok when sim has not been initialized");
                };
                Status::Ok(WorldState {
                    vessels: if show_staged { staged } else { vessels },
                    tick: *tick,
                })
            }
            Err(e) => Status::Err {
                message: e.to_string(),
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
    fn application() -> anyhow::Result<()> {
        let vessels = vec![
            VesselLocation {
                vessel: "v1".into(),
                location: 0,
            },
            VesselLocation {
                vessel: "v2".into(),
                location: 2,
            },
        ];

        let mut sim = Sim(None);
        apply(
            &mut sim,
            Command::Init {
                cargo_manifest: vec![],
                travel_times: vec![vec![0, 4, 7], vec![4, 0, 3], vec![7, 3, 0]],
                vessels,
            },
        )?;
        apply(
            &mut sim,
            Command::SetDest {
                vessel: "v1".into(),
                dest: 2,
            },
        )?;
        assert_eq!(
            VesselState::Transit {
                from: 0,
                to: 2,
                arrives_at_tick: 7
            },
            sim.0.as_ref().unwrap().staged["v1"]
        );
        apply(
            &mut sim,
            Command::SetDest {
                vessel: "v1".into(),
                dest: 1,
            },
        )?;
        assert_eq!(
            VesselState::Transit {
                from: 0,
                to: 1,
                arrives_at_tick: 4
            },
            sim.0.as_ref().unwrap().staged["v1"]
        );
        for _ in 0..3 {
            apply(&mut sim, Command::tick())?;
            assert_eq!(
                VesselState::Transit {
                    from: 0,
                    to: 1,
                    arrives_at_tick: 4
                },
                sim.0.as_ref().unwrap().vessels["v1"]
            );
        }
        apply(
            &mut sim,
            Command::SetDest {
                vessel: "v1".into(),
                dest: 2,
            },
        )?;
        apply(&mut sim, Command::tick())?;
        assert_eq!(
            VesselState::Transit {
                from: 1,
                to: 2,
                arrives_at_tick: 4 + 3,
            },
            sim.0.as_ref().unwrap().vessels["v1"]
        );
        assert!(
            apply(
                &mut sim,
                Command::Undock {
                    vessel_id: "v2".into()
                }
            )
            .is_err()
        );
        apply(
            &mut sim,
            Command::Dock {
                vessel_id: "v2".into(),
            },
        )?;
        assert_eq!(
            VesselState::Docked { location: 2 },
            sim.0.as_ref().unwrap().staged["v2"]
        );
        apply(&mut sim, Command::tick())?;
        assert_eq!(
            VesselState::Docked { location: 2 },
            sim.0.as_ref().unwrap().vessels["v2"]
        );
        apply(
            &mut sim,
            Command::Undock {
                vessel_id: "v2".into(),
            },
        )?;
        assert_eq!(
            VesselState::Idle { location: 2 },
            sim.0.as_ref().unwrap().staged["v2"],
        );

        Ok(())
    }
}
