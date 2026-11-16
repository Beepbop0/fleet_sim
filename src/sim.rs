use crate::nonempty::NonEmpty;
use crate::square::Square;
use crate::{Cargo, Command, FleetCommand, Init, Tick, VesselLocation};
use anyhow::{Context, anyhow, bail};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum VesselState {
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
pub struct Vessel {
    pub state: VesselState,
    pub cargo: Vec<usize>,
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
pub struct DockThroughput {
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
        if ops >= limit {
            let excess = *ops + 1;
            bail!(
                "transfer of {excess} operations exceed dock throughput of {limit} at location {dock}"
            );
        }
        *ops += 1;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FleetState {
    vessels: HashMap<String, Vessel>,
    cargo_manifest: Square,
}

pub struct Fleet {
    travel_times: Square,
    dock_bandwidth: DockBandwidth,
    committed: NonEmpty<FleetState>,
    planned: FleetState,
}

pub struct FleetView<'a> {
    travel_times: &'a Square,
    dock_bandwidth: &'a mut DockBandwidth,
    committed: &'a FleetState,
    planned: &'a mut FleetState,
    tick: usize,
}

impl<'a> From<&'a mut Fleet> for FleetView<'a> {
    fn from(value: &'a mut Fleet) -> Self {
        Self {
            travel_times: &value.travel_times,
            dock_bandwidth: &mut value.dock_bandwidth,
            committed: value.committed.last(),
            planned: &mut value.planned,
            tick: value.committed.rest_len(),
        }
    }
}

fn get_vessel<'a>(
    committed: &'a HashMap<String, Vessel>,
    planned: &'a mut HashMap<String, Vessel>,
    vessel: &str,
) -> anyhow::Result<(&'a Vessel, &'a mut Vessel)> {
    match (committed.get(vessel), planned.get_mut(vessel)) {
        (Some(existing), Some(pending)) => Ok((existing, pending)),
        _ => Err(anyhow!("unknown vessel {vessel}")),
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

fn init_fleet(init: Init) -> anyhow::Result<Fleet> {
    let Init {
        travel_times,
        vessels,
        dock_throughput,
        cargo_manifest,
    } = init;
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

    Ok(Fleet {
        travel_times,
        committed: NonEmpty::new(initial_state.clone()),
        planned: initial_state,
        dock_bandwidth: DockBandwidth::new(&dock_throughput[..]),
    })
}

fn apply_fleet_cmd(fleet: &mut Fleet, command: FleetCommand) -> anyhow::Result<()> {
    match command {
        FleetCommand::Advance(Tick::Tick { cargo_manifest }) => apply_tick(fleet, cargo_manifest),
        FleetCommand::Planned(command) => apply(FleetView::from(fleet), command),
    }
}

fn apply_tick(
    Fleet {
        committed,
        planned,
        dock_bandwidth,
        ..
    }: &mut Fleet,
    cargo_manifest: Vec<Cargo>,
) -> anyhow::Result<()> {
    apply_manifest(&mut planned.cargo_manifest, &cargo_manifest)?;
    let tick = committed.rest_len() + 1;
    for Vessel { state, .. } in planned.vessels.values_mut() {
        if let &mut VesselState::Transit {
            arrives_at_tick,
            to,
            ..
        } = state
            && arrives_at_tick == tick
        {
            *state = VesselState::Idle { location: to };
        }
    }
    committed.push(planned.clone());
    dock_bandwidth.reset();
    Ok(())
}

fn apply<'a, I: Into<FleetView<'a>>>(view: I, command: Command) -> anyhow::Result<()> {
    use Command::*;
    let FleetView {
        committed: FleetState {
            vessels: committed, ..
        },
        planned:
            FleetState {
                vessels: planned,
                cargo_manifest: planned_cargo,
            },
        travel_times,
        dock_bandwidth,
        tick,
    } = view.into();
    match command {
        GetState {} | GetStateAt { .. } => Ok(()),
        SetDest { vessel_id, dest } => {
            let n = travel_times.n();
            if dest >= n {
                bail!("destination can only be set to 0..{n}");
            }
            let (
                Some(Vessel {
                    state: existing, ..
                }),
                Some(Vessel { state: pending, .. }),
            ) = (committed.get(&vessel_id), planned.get_mut(&vessel_id))
            else {
                bail!("unknown vessel {vessel_id}");
            };
            *pending = match *existing {
                VesselState::Idle { location: from } => {
                    if from == dest {
                        return Ok(());
                    }
                    let distance = travel_times[from][dest];
                    VesselState::Transit {
                        from,
                        to: dest,
                        arrives_at_tick: tick + distance,
                    }
                }
                _ => bail!("vessel {vessel_id} is not idle"),
            };
            Ok(())
        }
        Dock { vessel_id } => {
            let (existing, pending) = get_vessel(committed, planned, &vessel_id)?;
            pending.state = match existing.state {
                VesselState::Idle { location } | VesselState::Docked { location } => {
                    VesselState::Docked { location }
                }
                _ => bail!("vessel {vessel_id} is not idle"),
            };
            Ok(())
        }
        Undock { vessel_id } => {
            let (existing, pending) = get_vessel(committed, planned, &vessel_id)?;
            pending.state = match existing.state {
                VesselState::Docked { location } => VesselState::Idle { location },
                _ => bail!("vessel {vessel_id} is not docked"),
            };
            Ok(())
        }
        Load {
            vessel_id,
            dest: destination,
        } => {
            if destination >= planned_cargo.n() {
                bail!("unknown dock {destination}");
            }
            let (existing, pending) = get_vessel(committed, planned, &vessel_id)?;
            let &VesselState::Docked { location: dock } = &existing.state else {
                bail!("vessel {vessel_id} is not docked");
            };
            let items = &mut planned_cargo[dock][destination];
            if *items == 0 {
                bail!("no cargo to load from location {dock} destined to {destination}");
            }
            dock_bandwidth.add_op(dock)?;
            pending.cargo[destination] += 1;
            *items -= 1;

            Ok(())
        }
        Unload {
            vessel_id,
            dest: destination,
        } => {
            if destination >= planned_cargo.n() {
                bail!("unknown dock {destination}");
            }
            let (existing, pending) = get_vessel(committed, planned, &vessel_id)?;
            let &VesselState::Docked { location: dock } = &existing.state else {
                bail!("vessel {vessel_id} is not docked");
            };
            let items = &mut pending.cargo[destination];
            if *items == 0 {
                bail!("no cargo on vessel {vessel_id} destined to {destination}");
            }
            dock_bandwidth.add_op(dock)?;

            *items -= 1;
            if destination != dock {
                planned_cargo[dock][destination] += 1;
            }

            Ok(())
        }
        Swap {
            vessel_id,
            load_dest,
            unload_dest,
        } => {
            for dest in [load_dest, unload_dest] {
                if dest >= planned_cargo.n() {
                    bail!("unknown dock {dest}");
                }
            }
            let (existing, pending) = get_vessel(committed, planned, &vessel_id)?;
            let &VesselState::Docked { location: dock } = &existing.state else {
                bail!("vessel {vessel_id} is not docked");
            };

            // load/unload pre-conditions
            let load_amount = &mut planned_cargo[dock][load_dest];
            if *load_amount == 0 {
                bail!("no cargo on dock {dock} destined to {load_dest}");
            }
            let unload_amount = &mut pending.cargo[unload_dest];
            if *unload_amount == 0 {
                bail!("no cargo on vessel {vessel_id} destined to {unload_dest}");
            }
            dock_bandwidth.add_op(dock)?;

            *load_amount -= 1;
            *unload_amount -= 1;
            pending.cargo[load_dest] += 1;
            if dock != unload_dest {
                planned_cargo[dock][unload_dest] += 1;
            }

            Ok(())
        }
    }
}

impl Fleet {
    pub fn init(init: Init) -> anyhow::Result<Fleet> {
        init_fleet(init)
    }

    pub fn apply(&mut self, command: FleetCommand) -> anyhow::Result<()> {
        apply_fleet_cmd(self, command)
    }
}

#[derive(Debug, Serialize)]
pub struct WorldState<'a> {
    pub tick: usize,
    pub vessels: &'a HashMap<String, Vessel>,
    pub cargo: Vec<Cargo>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", content = "payload", rename_all = "lowercase")]
pub enum Status<'a> {
    Ok(WorldState<'a>),
    #[serde(rename = "error")]
    Err {
        message: String,
    },
}

pub enum View {
    Planned,
    LastCommitted,
    CommittedAt(usize),
}

fn status(
    Fleet {
        planned, committed, ..
    }: &Fleet,
    view: View,
) -> Status<'_> {
    let state = match view {
        View::Planned => planned,
        View::LastCommitted => committed.last(),
        View::CommittedAt(tick) => {
            if tick == 0 {
                &committed.head
            } else {
                let i = tick - 1;
                if i >= committed.rest_len() {
                    return Status::Err {
                        message: format!(
                            "tick {} is in the future (current tick: {})",
                            tick,
                            committed.rest_len(),
                        ),
                    };
                }
                &committed.rest[i]
            }
        }
    };

    let mut cargo = vec![];
    let n = state.cargo_manifest.n();
    for dock in 0..n {
        for (dest, &quantity) in state.cargo_manifest[dock].iter().enumerate() {
            if quantity > 0 {
                cargo.push(Cargo {
                    dock,
                    dest,
                    quantity,
                });
            }
        }
    }

    Status::Ok(WorldState {
        vessels: &state.vessels,
        tick: committed.rest_len(),
        cargo,
    })
}

impl Fleet {
    pub fn status(&self, view: View) -> Status<'_> {
        status(self, view)
    }
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

        let mut fleet = init_fleet(Init {
            cargo_manifest: vec![],
            dock_throughput: vec![2, 4, 1],
            travel_times: vec![vec![0, 4, 7], vec![4, 0, 3], vec![7, 3, 0]],
            vessels,
        })?;
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
            apply_tick(&mut fleet, vec![])?;
            assert_eq!(
                VesselState::Transit {
                    from: 0,
                    to: 1,
                    arrives_at_tick: 4
                },
                fleet.committed.last().vessels["v1"].state,
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

        apply_tick(&mut fleet, vec![])?;
        assert_eq!(
            VesselState::Transit {
                from: 1,
                to: 2,
                arrives_at_tick: 4 + 3,
            },
            fleet.committed.last().vessels["v1"].state,
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
        apply_tick(
            &mut fleet,
            vec![
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
            apply_tick(&mut fleet, vec![])?;
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
                apply_tick(&mut fleet, vec![])?;
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
        apply_tick(&mut fleet, vec![])?;

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
        apply_tick(&mut fleet, vec![])?;
        apply(
            &mut fleet,
            Command::SetDest {
                vessel_id: "v2".into(),
                dest: 0,
            },
        )?;
        for _ in 0..7 {
            apply_tick(&mut fleet, vec![])?;
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
        apply_tick(&mut fleet, vec![])?;
        assert_eq!(0, fleet.committed.last().vessels["v2"].cargo[0]);
        assert_eq!(0, fleet.committed.last().cargo_manifest[0][0]);

        Ok(())
    }
}
