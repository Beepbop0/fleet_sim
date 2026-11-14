use anyhow::{Context, anyhow, bail};
use fleet_sim::square::Square;
use fleet_sim::{Cargo, Command, VesselLocation};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{self, BufRead};

#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
#[serde(tag = "state")]
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

#[derive(Debug, Clone, PartialEq)]
struct Vessel {
    state: VesselState,
    cargo: Vec<usize>,
}

impl Serialize for Vessel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Debug, Serialize)]
        #[allow(non_camel_case_types)]
        struct cargo {
            quantity: usize,
            destination: usize,
        }
        #[derive(Debug, Serialize)]
        #[allow(non_camel_case_types)]
        struct vessel {
            #[serde(flatten)]
            state: VesselState,
            cargo: Vec<cargo>,
        }
        let mut cargo = vec![];
        for (destination, &quantity) in self.cargo.iter().enumerate() {
            if quantity > 0 {
                cargo.push(cargo {
                    quantity,
                    destination,
                });
            }
        }

        let item = vessel {
            state: self.state,
            cargo,
        };

        item.serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy)]
struct DockThroughput {
    limit: usize,
    ops: usize,
}

#[derive(Debug, Clone)]
struct DockBandwidth(Vec<DockThroughput>);

impl DockBandwidth {
    fn new(throughput: &[usize]) -> Self {
        Self(
            throughput
                .iter()
                .map(|&limit| DockThroughput { limit, ops: 0 })
                .collect(),
        )
    }

    fn reset(&mut self) {
        for DockThroughput { ops, .. } in &mut self.0 {
            *ops = 0;
        }
    }

    fn add_op(&mut self, dock: usize) -> anyhow::Result<()> {
        let Some(DockThroughput { limit, ops }) = self.0.get_mut(dock) else {
            bail!("unknown dock {dock}");
        };
        *ops += 1;
        if ops > limit {
            bail!(
                "transfer of {ops} operations exceed dock throughput of {limit} at location {dock}"
            );
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FleetState {
    vessels: HashMap<String, Vessel>,
    cargo_manifest: Square,
}

struct Fleet {
    tick: usize,
    travel_times: Square,
    dock_bandwidth: DockBandwidth,
    committed: FleetState,
    planned: FleetState,
}

impl Fleet {
    fn get_vessel(&mut self, vessel: &str) -> anyhow::Result<(&Vessel, &mut Vessel)> {
        match (
            self.committed.vessels.get(vessel),
            self.planned.vessels.get_mut(vessel),
        ) {
            (Some(existing), Some(pending)) => Ok((existing, pending)),
            _ => Err(anyhow!("unknown vessel {vessel}")),
        }
    }
}

fn apply_manifest(square: &mut Square, manifest: &[Cargo]) -> anyhow::Result<()> {
    for &Cargo { dock, dest, .. } in manifest {
        if dock >= square.n() {
            bail!("unknown location {dock}");
        }
        if dest >= square.n() {
            bail!("unknown destination {dest}");
        }
    }
    for &Cargo {
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

fn sim_apply(sim: &mut Sim, command: Command) -> anyhow::Result<()> {
    let Some(fleet) = sim.0.as_mut() else {
        let Command::Init {
            travel_times,
            vessels,
            dock_throughput,
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
                .insert(
                    vessel.clone(),
                    Vessel {
                        state: VesselState::Idle { location },
                        cargo: vec![0; n],
                    },
                )
                .is_some()
            {
                bail!("duplicate registration of vessel {vessel}");
            }
        }
        let mut initial_state = FleetState {
            vessels: locations,
            cargo_manifest: Square::zeros(n),
        };
        apply_manifest(&mut initial_state.cargo_manifest, &cargo_manifest)?;
        sim.0 = Some(Fleet {
            tick: 0,
            travel_times,
            committed: initial_state.clone(),
            planned: initial_state,
            dock_bandwidth: DockBandwidth::new(&dock_throughput[..]),
        });

        return Ok(());
    };

    apply(fleet, command)
}

fn apply(fleet: &mut Fleet, command: Command) -> anyhow::Result<()> {
    use Command::*;
    match command {
        GetState => Ok(()),
        Init { .. } => Err(anyhow!("cannot call init again")),
        Tick { cargo_manifest } => {
            fleet.committed = fleet.planned.clone();
            fleet.tick += 1;
            for Vessel { state, .. } in fleet.committed.vessels.values_mut() {
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
            apply_manifest(&mut fleet.planned.cargo_manifest, &cargo_manifest)?;
            fleet.committed.cargo_manifest = fleet.planned.cargo_manifest.clone();
            fleet.dock_bandwidth.reset();
            Ok(())
        }
        SetDest { vessel_id, dest } => {
            let n = fleet.travel_times.n();
            if dest >= n {
                bail!("destination can only be set to 0..{n}");
            }
            let tick = fleet.tick;
            let (
                Some(Vessel {
                    state: existing, ..
                }),
                Some(Vessel { state: pending, .. }),
            ) = (
                fleet.committed.vessels.get(&vessel_id),
                fleet.planned.vessels.get_mut(&vessel_id),
            )
            else {
                bail!("unknown vessel {vessel_id}");
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
                _ => bail!("vessel {vessel_id} is not idle"),
            };
            Ok(())
        }
        Dock { vessel_id } => {
            let (existing, pending) = fleet.get_vessel(&vessel_id)?;
            pending.state = match existing.state {
                VesselState::Idle { location } | VesselState::Docked { location } => {
                    VesselState::Docked { location }
                }
                _ => bail!("vessel {vessel_id} is not idle"),
            };
            Ok(())
        }
        Undock { vessel_id: vessel } => {
            let (existing, pending) = fleet.get_vessel(&vessel)?;
            pending.state = match existing.state {
                VesselState::Docked { location } => VesselState::Idle { location },
                _ => bail!("vessel {vessel} is not docked"),
            };
            Ok(())
        }
        Load {
            vessel_id,
            dest: destination,
        } => {
            if destination >= fleet.planned.cargo_manifest.n() {
                bail!("unknown dock {destination}");
            }
            let pending = fleet
                .planned
                .vessels
                .get_mut(&vessel_id)
                .with_context(|| format!("unknown vessel {vessel_id}"))?;

            let &VesselState::Docked { location: dock } = &pending.state else {
                bail!("vessel {vessel_id} is not docked");
            };
            fleet.dock_bandwidth.add_op(dock)?;
            let items = &mut fleet.planned.cargo_manifest[dock][destination];
            if *items == 0 {
                bail!("no cargo to load from location {dock} destined to {destination}");
            }
            pending.cargo[destination] += 1;
            *items -= 1;

            Ok(())
        }
        Unload {
            vessel_id,
            dest: destination,
        } => {
            if destination >= fleet.planned.cargo_manifest.n() {
                bail!("unknown dock {destination}");
            }
            let pending = fleet
                .planned
                .vessels
                .get_mut(&vessel_id)
                .with_context(|| format!("unknown vessel {vessel_id}"))?;
            let &VesselState::Docked { location: dock } = &pending.state else {
                bail!("vessel {vessel_id} is not docked");
            };
            fleet.dock_bandwidth.add_op(dock)?;
            let items = &mut pending.cargo[destination];
            if *items == 0 {
                bail!("no cargo on vessel {vessel_id} destined to {destination}");
            }

            *items -= 1;
            if destination != dock {
                fleet.planned.cargo_manifest[dock][destination] += 1;
            }

            Ok(())
        }
        Swap {
            vessel_id,
            load_dest,
            unload_dest,
        } => {
            for dest in [load_dest, unload_dest] {
                if load_dest >= fleet.planned.cargo_manifest.n() {
                    bail!("unknown dock {dest}");
                }
            }
            let pending = fleet
                .planned
                .vessels
                .get_mut(&vessel_id)
                .with_context(|| format!("unknown vessel {vessel_id}"))?;
            let &VesselState::Docked { location: dock } = &pending.state else {
                bail!("vessel {vessel_id} is not docked");
            };

            // load/unload pre-conditions
            let load_amount = &mut fleet.planned.cargo_manifest[dock][load_dest];
            if *load_amount == 0 {
                bail!("no cargo on dock {dock} destined to {load_dest}");
            }
            let unload_amount = &mut pending.cargo[unload_dest];
            if *unload_amount == 0 {
                bail!("no cargo on vessel {vessel_id} destined to {unload_dest}");
            }
            fleet.dock_bandwidth.add_op(dock)?;

            *load_amount -= 1;
            *unload_amount -= 1;
            pending.cargo[load_dest] += 1;
            if dock != unload_dest {
                fleet.planned.cargo_manifest[dock][load_dest] += 1;
            }

            Ok(())
        }
    }
}

#[derive(Debug, Serialize)]
struct WorldState<'a> {
    tick: usize,
    vessels: &'a HashMap<String, Vessel>,
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
        let show_planned = matches!(&command, Command::GetState | Command::SetDest { .. });
        let status = match sim_apply(&mut state, command) {
            Ok(_) => {
                let Some(Fleet {
                    committed,
                    planned,
                    tick,
                    ..
                }) = state.0.as_ref()
                else {
                    panic!("received Ok when sim has not been initialized");
                };
                Status::Ok(WorldState {
                    vessels: if show_planned {
                        &planned.vessels
                    } else {
                        &committed.vessels
                    },
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
        sim_apply(
            &mut sim,
            Command::Init {
                cargo_manifest: vec![],
                dock_throughput: vec![2, 4, 1],
                travel_times: vec![vec![0, 4, 7], vec![4, 0, 3], vec![7, 3, 0]],
                vessels,
            },
        )?;
        let mut fleet = sim.0.unwrap();
        apply(
            &mut fleet,
            Command::SetDest {
                vessel_id: "v1".into(),
                dest: 2,
            },
        )?;
        assert_eq!(
            VesselState::Transit {
                from: 0,
                to: 2,
                arrives_at_tick: 7
            },
            fleet.planned.vessels["v1"].state,
        );
        apply(
            &mut fleet,
            Command::SetDest {
                vessel_id: "v1".into(),
                dest: 1,
            },
        )?;
        assert_eq!(
            VesselState::Transit {
                from: 0,
                to: 1,
                arrives_at_tick: 4
            },
            fleet.planned.vessels["v1"].state,
        );
        for _ in 0..3 {
            apply(&mut fleet, Command::tick())?;
            assert_eq!(
                VesselState::Transit {
                    from: 0,
                    to: 1,
                    arrives_at_tick: 4
                },
                fleet.committed.vessels["v1"].state,
            );
            assert!(
                apply(
                    &mut fleet,
                    Command::Dock {
                        vessel_id: "v1".into()
                    }
                )
                .is_err()
            );
            assert!(
                apply(
                    &mut fleet,
                    Command::Undock {
                        vessel_id: "v1".into()
                    }
                )
                .is_err()
            );
        }
        apply(
            &mut fleet,
            Command::SetDest {
                vessel_id: "v1".into(),
                dest: 2,
            },
        )?;

        apply(&mut fleet, Command::tick())?;
        assert_eq!(
            VesselState::Transit {
                from: 1,
                to: 2,
                arrives_at_tick: 4 + 3,
            },
            fleet.committed.vessels["v1"].state,
        );
        assert!(
            apply(
                &mut fleet,
                Command::Undock {
                    vessel_id: "v2".into()
                }
            )
            .is_err()
        );
        apply(
            &mut fleet,
            Command::Dock {
                vessel_id: "v2".into(),
            },
        )?;
        assert_eq!(
            VesselState::Docked { location: 2 },
            fleet.planned.vessels["v2"].state,
        );
        apply(
            &mut fleet,
            Command::Tick {
                cargo_manifest: vec![
                    Cargo {
                        quantity: 1,
                        dock: 0,
                        dest: 2,
                    },
                    Cargo {
                        quantity: 2,
                        dock: 2,
                        dest: 0,
                    },
                ],
            },
        )?;

        // v2 has no cargo to unload
        // dock 2 only has cargo destined to dock 0
        for i in 0..=3 {
            let prev = fleet.planned.clone();
            assert!(
                apply(
                    &mut fleet,
                    Command::Unload {
                        vessel_id: "v2".into(),
                        dest: i,
                    }
                )
                .is_err()
            );
            assert_eq!(prev, fleet.planned);
            apply(&mut fleet, Command::tick())?;
            if i > 0 {
                let prev = fleet.planned.clone();
                assert!(
                    apply(
                        &mut fleet,
                        Command::Load {
                            vessel_id: "v2".into(),
                            dest: i
                        }
                    )
                    .is_err()
                );
                assert_eq!(prev, fleet.planned);
                apply(&mut fleet, Command::tick())?;
            }
        }
        apply(
            &mut fleet,
            Command::Load {
                vessel_id: "v2".into(),
                dest: 0,
            },
        )?;
        assert_eq!(1, fleet.planned.vessels["v2"].cargo[0]);
        // reached max operations throughput
        assert!(
            apply(
                &mut fleet,
                Command::Load {
                    vessel_id: "v2".into(),
                    dest: 0,
                }
            )
            .is_err()
        );
        apply(&mut fleet, Command::tick())?;

        apply(
            &mut fleet,
            Command::Undock {
                vessel_id: "v2".into(),
            },
        )?;
        assert_eq!(
            VesselState::Idle { location: 2 },
            fleet.planned.vessels["v2"].state
        );
        apply(&mut fleet, Command::tick())?;
        apply(
            &mut fleet,
            Command::SetDest {
                vessel_id: "v2".into(),
                dest: 0,
            },
        )?;
        for _ in 0..7 {
            apply(&mut fleet, Command::tick())?;
        }
        apply(
            &mut fleet,
            Command::Dock {
                vessel_id: "v2".into(),
            },
        )?;
        apply(
            &mut fleet,
            Command::Unload {
                vessel_id: "v2".into(),
                dest: 0,
            },
        )?;
        apply(&mut fleet, Command::tick())?;
        assert_eq!(0, fleet.committed.vessels["v2"].cargo[0]);
        assert_eq!(0, fleet.committed.cargo_manifest[0][0]);

        Ok(())
    }
}
