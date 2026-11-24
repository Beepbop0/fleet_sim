use crate::nonempty::NonEmpty;
use crate::square::Square;
use crate::{Cargo, FleetCommand, Init, MutCommand, Tick, VesselLocation, ViewCommand};
use anyhow::{Context, anyhow, bail};
use serde::Serialize;
use std::borrow::Cow;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CargoAction {
    Unload(usize),
    Load(usize),
    Swap {
        load_dest: usize,
        unload_dest: usize,
    },
}

#[derive(Clone, Copy)]
enum CargoOp {
    Load { destination: usize },
    Unload { destination: usize },
}

impl CargoOp {
    fn destination(self) -> usize {
        match self {
            Self::Load { destination } | Self::Unload { destination } => destination,
        }
    }
}

impl From<CargoOp> for CargoAction {
    fn from(operation: CargoOp) -> Self {
        match operation {
            CargoOp::Load { destination } => Self::Load(destination),
            CargoOp::Unload { destination } => Self::Unload(destination),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct CargoOps {
    load: Option<CargoOp>,
    unload: Option<CargoOp>,
}

impl From<CargoAction> for CargoOps {
    fn from(action: CargoAction) -> Self {
        match action {
            CargoAction::Load(dest) => Self {
                load: Some(CargoOp::Load { destination: dest }),
                ..Self::default()
            },
            CargoAction::Unload(dest) => Self {
                unload: Some(CargoOp::Unload { destination: dest }),
                ..Self::default()
            },
            CargoAction::Swap {
                load_dest,
                unload_dest,
            } => Self {
                load: Some(CargoOp::Load {
                    destination: load_dest,
                }),
                unload: Some(CargoOp::Unload {
                    destination: unload_dest,
                }),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CargoEvent {
    vessel: String,
    dock: usize,
    action: CargoAction,
}

#[derive(Debug, Clone)]
pub enum Event {
    Vessel { id: String, state: VesselState },
    Cargo(CargoEvent),
}

#[derive(Debug, Clone, PartialEq)]
struct FleetState {
    vessels: HashMap<String, Vessel>,
    cargo_manifest: Square,
}

pub struct Fleet {
    travel_times: Square,
    dock_throughput: Vec<usize>,
    committed: NonEmpty<FleetState>,
    planned: Vec<Event>,
}

pub struct FleetView<'a> {
    travel_times: &'a Square,
    dock_throughput: &'a [usize],
    committed: &'a FleetState,
    planned: &'a mut Vec<Event>,
    tick: usize,
}

impl<'a> From<&'a mut Fleet> for FleetView<'a> {
    fn from(value: &'a mut Fleet) -> Self {
        Self {
            travel_times: &value.travel_times,
            dock_throughput: &mut value.dock_throughput,
            committed: value.committed.last(),
            planned: &mut value.planned,
            tick: value.committed.rest_len(),
        }
    }
}

fn apply_manifest(square: &mut Square, manifest: &[Cargo]) -> anyhow::Result<()> {
    for &Cargo {
        dock,
        dest,
        quantity,
    } in manifest
    {
        if dock >= square.n() {
            bail!("unknown location {dock}");
        }
        if dest >= square.n() {
            bail!("unknown destination {dest}");
        }
        if dock == dest {
            bail!("cargo at location {dock} is already at its destination");
        }
        if quantity == 0 {
            bail!("expected positive quantity of cargo");
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
    for i in 0..n {
        for j in 0..n {
            if i != j && travel_times[i][j] == 0 {
                bail!("expected non-zero travel time between locations");
            }
        }
    }
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

    if dock_throughput.len() != n {
        bail!("expected {} entries: got {}", n, dock_throughput.len());
    }
    if let Some((location, _)) = dock_throughput.iter().enumerate().find(|(_, t)| **t == 0) {
        bail!("dock {location} needs positive throughput");
    }

    let mut initial_state = FleetState {
        vessels: locations,
        cargo_manifest: Square::zeros(n),
    };
    apply_manifest(&mut initial_state.cargo_manifest, &cargo_manifest)?;

    Ok(Fleet {
        travel_times,
        dock_throughput,
        committed: NonEmpty::new(initial_state.clone()),
        planned: vec![],
    })
}

fn get_vessel<'a>(
    committed: &'a HashMap<String, Vessel>,
    vessel: &str,
) -> anyhow::Result<&'a Vessel> {
    match committed.get(vessel) {
        Some(existing) => Ok(existing),
        _ => Err(anyhow!("unknown vessel {vessel}")),
    }
}

fn apply_fleet_cmd(fleet: &mut Fleet, command: FleetCommand) -> anyhow::Result<()> {
    match command {
        FleetCommand::View(_) => Ok(()),
        FleetCommand::Advance(Tick::Tick { cargo_manifest }) => apply_tick(fleet, &cargo_manifest),
        FleetCommand::Planned(command) => {
            if let Some(event) = event(fleet.into(), command)? {
                fleet.planned.push(event);
            }
            Ok(())
        }
    }
}

fn apply_tick(fleet: &mut Fleet, manifest: &[Cargo]) -> anyhow::Result<()> {
    let mut next = fleet.committed.last().clone();
    apply_events(&mut next, &fleet.planned)?;
    apply_manifest(&mut next.cargo_manifest, manifest)?;
    let tick = fleet.committed.rest_len() + 1;
    for Vessel { state, .. } in next.vessels.values_mut() {
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
    fleet.planned.clear();
    fleet.committed.push(next);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum EventLogChange {
    NoOp,
    Add,
    Edit { at: usize, kind: EventLogEdit },
}

#[derive(Debug, Clone, Copy)]
enum EventLogEdit {
    Remove,
    ReplaceWith(CargoAction),
    RemoveAndAdd(CargoAction),
}

fn reduce_actions(old: CargoAction, new: CargoAction, dock: usize) -> Option<EventLogEdit> {
    use EventLogEdit::*;
    let (old, new) = (CargoOps::from(old), CargoOps::from(new));
    let old_load = old.load.map(CargoOp::destination);
    let old_unload = old.unload.map(CargoOp::destination);
    let new_load = new.load.map(CargoOp::destination);
    let new_unload = new.unload.map(CargoOp::destination);
    let cancel_load = old_load == new_unload && new_unload.is_some_and(|dest| dest != dock);
    let cancel_unload = old_unload == new_load && old_unload.is_some_and(|dest| dest != dock);

    match (cancel_load, cancel_unload) {
        (false, false) => None,
        (true, true) => Some(Remove),
        (true, false) => Some(match (old.unload, new.load) {
            (None, None) => Remove,
            (Some(old), None) => ReplaceWith(old.into()),
            (None, Some(new)) => RemoveAndAdd(new.into()),
            (Some(old), Some(new)) => ReplaceWith(CargoAction::Swap {
                load_dest: new.destination(),
                unload_dest: old.destination(),
            }),
        }),
        (false, true) => Some(match (old.load, new.unload) {
            (None, None) => Remove,
            (Some(old), None) => ReplaceWith(old.into()),
            (None, Some(new)) => RemoveAndAdd(new.into()),
            (Some(old), Some(new)) => ReplaceWith(CargoAction::Swap {
                load_dest: old.destination(),
                unload_dest: new.destination(),
            }),
        }),
    }
}

fn reduce_cargo_event(event: &CargoEvent, events: &[Event]) -> EventLogChange {
    if let CargoAction::Swap {
        load_dest,
        unload_dest,
    } = event.action
        && load_dest == unload_dest
    {
        return EventLogChange::NoOp;
    }

    events
        .iter()
        .enumerate()
        .rev()
        .find_map(|(at, candidate)| {
            let Event::Cargo(candidate) = candidate else {
                return None;
            };
            if candidate.vessel != event.vessel || candidate.dock != event.dock {
                return None;
            }

            reduce_actions(candidate.action, event.action, event.dock)
                .map(|kind| EventLogChange::Edit { at, kind })
        })
        .unwrap_or(EventLogChange::Add)
}

fn validate_action(
    vessel: String,
    action: CargoAction,
    events: &mut Vec<Event>,
    throughput: &[usize],
    vessels: &HashMap<String, Vessel>,
    cargo_manifest: &Square,
) -> anyhow::Result<()> {
    let VesselState::Docked { .. } = get_vessel(vessels, &vessel)?.state else {
        bail!("vessel {vessel} is not committed as docked");
    };
    let mut pending = FleetState {
        vessels: vessels.clone(),
        cargo_manifest: cargo_manifest.clone(),
    };
    apply_events(&mut pending, events)?;
    let pending_vessel = get_vessel(&pending.vessels, &vessel)?;
    let VesselState::Docked { location: dock } = pending_vessel.state else {
        bail!("vessel {vessel} is not docked");
    };

    // see if we can consolidate our event log with this new cargo op
    let event = CargoEvent {
        vessel: vessel.clone(),
        dock,
        action,
    };
    let edit = reduce_cargo_event(&event, events);
    let mut new_events = events.clone();
    match edit {
        EventLogChange::NoOp => (),
        EventLogChange::Add => new_events.push(Event::Cargo(event)),
        EventLogChange::Edit { at, kind } => match kind {
            EventLogEdit::Remove => {
                new_events.remove(at);
            }
            EventLogEdit::ReplaceWith(replace) => {
                new_events[at] = Event::Cargo(CargoEvent {
                    vessel: event.vessel,
                    dock: event.dock,
                    action: replace,
                });
            }
            EventLogEdit::RemoveAndAdd(add) => {
                new_events.remove(at);
                new_events.push(Event::Cargo(CargoEvent {
                    vessel: event.vessel,
                    dock: event.dock,
                    action: add,
                }));
            }
        },
    }

    let mut pending = FleetState {
        vessels: vessels.clone(),
        cargo_manifest: cargo_manifest.clone(),
    };
    // replay new events and see if the cargo accounting still makes sense
    apply_events(&mut pending, &new_events)?;
    // see if ops accounting checks out with new events
    let op_count = new_events
        .iter()
        .filter(
            |e| matches!(e, Event::Cargo(CargoEvent { dock: this_dock, .. }) if *this_dock == dock),
        )
        .count();
    if let limit = throughput[dock]
        && op_count > limit
    {
        bail!(
            "transfer of {op_count} operations exceed dock throughput of {limit} at location {dock}"
        );
    }
    *events = new_events;

    Ok(())
}

fn apply_events(fleet_state: &mut FleetState, events: &[Event]) -> anyhow::Result<()> {
    for event in events {
        match event {
            Event::Vessel { id, state } => {
                let vessel = fleet_state.vessels.get_mut(id).unwrap();
                vessel.state = *state;
            }
            Event::Cargo(CargoEvent {
                vessel,
                dock,
                action,
            }) => {
                let vessel_cargo = &mut fleet_state.vessels.get_mut(vessel).unwrap().cargo;
                let dock_cargo = &mut fleet_state.cargo_manifest[*dock];
                match *action {
                    CargoAction::Load(dest) => {
                        if dock_cargo[dest] == 0 {
                            bail!("no cargo on dock {dock} destined to {dest}");
                        }
                        vessel_cargo[dest] += 1;
                        dock_cargo[dest] -= 1;
                    }
                    CargoAction::Unload(dest) => {
                        if vessel_cargo[dest] == 0 {
                            bail!("no cargo on vessel {vessel} destined to {dest}");
                        }
                        vessel_cargo[dest] -= 1;
                        if dest != *dock {
                            dock_cargo[dest] += 1;
                        }
                    }
                    CargoAction::Swap {
                        load_dest,
                        unload_dest,
                    } => {
                        if vessel_cargo[unload_dest] == 0 {
                            bail!("no cargo on vessel {vessel} destined to {unload_dest}");
                        }
                        if dock_cargo[load_dest] == 0 {
                            bail!("no cargo on dock {dock} destined to {load_dest}");
                        }
                        vessel_cargo[unload_dest] -= 1;
                        if unload_dest != *dock {
                            dock_cargo[unload_dest] += 1;
                        }
                        vessel_cargo[load_dest] += 1;
                        dock_cargo[load_dest] -= 1;
                    }
                }
            }
        }
    }
    Ok(())
}

fn event<'a>(
    FleetView {
        committed:
            FleetState {
                vessels: committed,
                cargo_manifest,
            },
        planned,
        travel_times,
        dock_throughput,
        tick,
    }: FleetView<'a>,
    command: MutCommand,
) -> anyhow::Result<Option<Event>> {
    use MutCommand::*;
    let n = travel_times.n();
    let destinations = match &command {
        SetDest { dest, .. } | Load { dest, .. } | Unload { dest, .. } => &[*dest][..],
        &Swap {
            load_dest,
            unload_dest,
            ..
        } => &[load_dest, unload_dest][..],
        Dock { .. } | Undock { .. } => &[][..],
    };
    if let Some(dest) = destinations.iter().find(|&&dest| dest >= n) {
        bail!("unknown location {dest}");
    }

    match command {
        SetDest { vessel_id, dest } => {
            let existing = get_vessel(committed, &vessel_id)?;
            let VesselState::Idle { location: from } = existing.state else {
                bail!("vessel {vessel_id} is not idle");
            };
            if from == dest {
                planned
                    .retain(|event| !matches!(event, Event::Vessel { id, .. } if id == &vessel_id));
                return Ok(None);
            }
            let distance = travel_times[from][dest];
            Ok(Some(Event::Vessel {
                id: vessel_id,
                state: VesselState::Transit {
                    from,
                    to: dest,
                    arrives_at_tick: tick + distance,
                },
            }))
        }
        Dock { vessel_id } => {
            let existing = get_vessel(committed, &vessel_id)?;
            match existing.state {
                VesselState::Idle { location } | VesselState::Docked { location } => {
                    Ok(Some(Event::Vessel {
                        id: vessel_id,
                        state: VesselState::Docked { location },
                    }))
                }
                _ => Err(anyhow!("vessel {vessel_id} is not idle")),
            }
        }
        Undock { vessel_id } => {
            let existing = get_vessel(committed, &vessel_id)?;
            match existing.state {
                VesselState::Docked { location } => {
                    planned.retain(|event| {
                        !matches!(
                            event,
                            Event::Cargo(CargoEvent { vessel, .. }) if vessel == &vessel_id
                        )
                    });
                    Ok(Some(Event::Vessel {
                        id: vessel_id,
                        state: VesselState::Idle { location },
                    }))
                }
                _ => Err(anyhow!("vessel {vessel_id} is not docked")),
            }
        }
        Load { vessel_id, dest } => {
            validate_action(
                vessel_id,
                CargoAction::Load(dest),
                planned,
                dock_throughput,
                committed,
                cargo_manifest,
            )?;
            Ok(None)
        }
        Unload { vessel_id, dest } => {
            validate_action(
                vessel_id,
                CargoAction::Unload(dest),
                planned,
                dock_throughput,
                committed,
                cargo_manifest,
            )?;
            Ok(None)
        }
        Swap {
            vessel_id,
            load_dest,
            unload_dest,
        } => {
            validate_action(
                vessel_id,
                CargoAction::Swap {
                    load_dest,
                    unload_dest,
                },
                planned,
                dock_throughput,
                committed,
                cargo_manifest,
            )?;
            Ok(None)
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
    pub vessels: Cow<'a, HashMap<String, Vessel>>,
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

#[derive(Debug, PartialEq)]
pub enum View {
    Planned,
    LastCommitted,
    CommittedAt(usize),
}

impl View {
    pub fn for_command(command: &FleetCommand) -> Result<Self, String> {
        match command {
            FleetCommand::Advance(_) => Ok(Self::LastCommitted),
            FleetCommand::View(ViewCommand::GetStateAt { tick }) => match usize::try_from(*tick) {
                Ok(tick) => Ok(Self::CommittedAt(tick)),
                Err(_) => Err("tick must be non-negative".to_owned()),
            },
            FleetCommand::View(ViewCommand::GetState {}) | FleetCommand::Planned(_) => {
                Ok(Self::Planned)
            }
        }
    }
}

fn status(
    Fleet {
        planned, committed, ..
    }: &Fleet,
    view: View,
) -> Status<'_> {
    let (tick, state) = match view {
        View::Planned => {
            let mut staged = committed.last().clone();
            apply_events(&mut staged, planned).expect("persisted an invalid event");
            (committed.rest_len(), Cow::Owned(staged))
        }
        View::LastCommitted => (committed.rest_len(), Cow::Borrowed(committed.last())),
        View::CommittedAt(tick) => (
            tick,
            if tick == 0 {
                Cow::Borrowed(&committed.head)
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
                Cow::Borrowed(&committed.rest[i])
            },
        ),
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
        vessels: match state {
            Cow::Borrowed(state) => Cow::Borrowed(&state.vessels),
            Cow::Owned(state) => Cow::Owned(state.vessels),
        },
        tick,
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
    fn command_views_follow_snapshot_semantics() {
        assert_eq!(
            View::for_command(&FleetCommand::Advance(Tick::Tick {
                cargo_manifest: vec![],
            })),
            Ok(View::LastCommitted),
        );
        assert_eq!(
            View::for_command(&FleetCommand::Planned(MutCommand::Dock {
                vessel_id: "v1".into(),
            })),
            Ok(View::Planned),
        );
        assert_eq!(
            View::for_command(&FleetCommand::View(ViewCommand::GetStateAt { tick: 3 })),
            Ok(View::CommittedAt(3)),
        );
        assert!(
            View::for_command(&FleetCommand::View(ViewCommand::GetStateAt { tick: -1 })).is_err()
        );
    }

    #[test]
    fn historical_status_uses_requested_tick() -> anyhow::Result<()> {
        let mut fleet = init_fleet(Init {
            cargo_manifest: vec![],
            dock_throughput: vec![1],
            travel_times: vec![vec![0]],
            vessels: vec![],
        })?;
        apply_tick(&mut fleet, &[])?;

        let json = serde_json::to_value(fleet.status(View::CommittedAt(0)))?;
        assert_eq!(json["payload"]["tick"], 0);
        Ok(())
    }

    #[test]
    fn vessel_arrives_on_scheduled_tick() -> anyhow::Result<()> {
        let mut fleet = init_fleet(Init {
            cargo_manifest: vec![],
            dock_throughput: vec![1, 1],
            travel_times: vec![vec![0, 2], vec![2, 0]],
            vessels: vec![VesselLocation {
                vessel: "v1".into(),
                location: 0,
            }],
        })?;
        fleet.apply(FleetCommand::Planned(MutCommand::SetDest {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;

        fleet.apply(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![],
        }))?;
        assert_eq!(
            fleet.committed.last().vessels["v1"].state,
            VesselState::Transit {
                from: 0,
                to: 1,
                arrives_at_tick: 2,
            },
        );

        fleet.apply(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![],
        }))?;
        assert_eq!(
            fleet.committed.last().vessels["v1"].state,
            VesselState::Idle { location: 1 },
        );
        Ok(())
    }

    #[test]
    fn cargo_commands_reject_unknown_destinations() -> anyhow::Result<()> {
        let mut fleet = init_fleet(Init {
            cargo_manifest: vec![],
            dock_throughput: vec![4],
            travel_times: vec![vec![0]],
            vessels: vec![VesselLocation {
                vessel: "v1".into(),
                location: 0,
            }],
        })?;
        fleet.apply(FleetCommand::Planned(MutCommand::Dock {
            vessel_id: "v1".into(),
        }))?;
        fleet.apply(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![],
        }))?;

        let commands = [
            MutCommand::Load {
                vessel_id: "v1".into(),
                dest: 1,
            },
            MutCommand::Unload {
                vessel_id: "v1".into(),
                dest: 1,
            },
            MutCommand::Swap {
                vessel_id: "v1".into(),
                load_dest: 1,
                unload_dest: 0,
            },
            MutCommand::Swap {
                vessel_id: "v1".into(),
                load_dest: 0,
                unload_dest: 1,
            },
        ];
        for command in commands {
            let error = fleet
                .apply(FleetCommand::Planned(command))
                .expect_err("unknown destination should be rejected");
            assert_eq!(error.to_string(), "unknown location 1");
            assert!(fleet.planned.is_empty());
        }
        Ok(())
    }

    #[test]
    fn undock_culls_vessels_planned_cargo_operations() -> anyhow::Result<()> {
        let mut fleet = init_fleet(Init {
            cargo_manifest: vec![],
            dock_throughput: vec![2, 1],
            travel_times: vec![vec![0, 1], vec![1, 0]],
            vessels: vec![
                VesselLocation {
                    vessel: "v1".into(),
                    location: 0,
                },
                VesselLocation {
                    vessel: "v2".into(),
                    location: 0,
                },
            ],
        })?;
        for vessel_id in ["v1", "v2"] {
            fleet.apply(FleetCommand::Planned(MutCommand::Dock {
                vessel_id: vessel_id.into(),
            }))?;
        }
        fleet.apply(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![Cargo {
                dock: 0,
                dest: 1,
                quantity: 3,
            }],
        }))?;

        for vessel_id in ["v1", "v2"] {
            fleet.apply(FleetCommand::Planned(MutCommand::Load {
                vessel_id: vessel_id.into(),
                dest: 1,
            }))?;
        }
        fleet.apply(FleetCommand::Planned(MutCommand::Undock {
            vessel_id: "v1".into(),
        }))?;

        let mut projected = fleet.committed.last().clone();
        apply_events(&mut projected, &fleet.planned)?;
        assert_eq!(
            projected.vessels["v1"].state,
            VesselState::Idle { location: 0 }
        );
        assert_eq!(projected.vessels["v1"].cargo[1], 0);
        assert_eq!(projected.vessels["v2"].cargo[1], 1);
        assert_eq!(projected.cargo_manifest[0][1], 2);

        // Removing v1's load also releases its share of dock throughput.
        fleet.apply(FleetCommand::Planned(MutCommand::Load {
            vessel_id: "v2".into(),
            dest: 1,
        }))?;
        Ok(())
    }

    #[test]
    fn ambiguous_command_sequences_rewrite_the_plan() -> anyhow::Result<()> {
        let mut fleet = init_fleet(Init {
            cargo_manifest: vec![Cargo {
                dock: 0,
                dest: 1,
                quantity: 2,
            }],
            dock_throughput: vec![1, 1],
            travel_times: vec![vec![0, 1], vec![1, 0]],
            vessels: vec![VesselLocation {
                vessel: "v1".into(),
                location: 0,
            }],
        })?;
        fleet.apply(FleetCommand::Planned(MutCommand::Dock {
            vessel_id: "v1".into(),
        }))?;
        fleet.apply(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![],
        }))?;
        fleet.apply(FleetCommand::Planned(MutCommand::Load {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;
        fleet.apply(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![],
        }))?;

        // Loading and then unloading the same cargo cancels both operations.
        fleet.apply(FleetCommand::Planned(MutCommand::Load {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;
        fleet.apply(FleetCommand::Planned(MutCommand::Unload {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;
        assert!(fleet.planned.is_empty());

        // The inverse ordering cancels too and releases the throughput slot.
        fleet.apply(FleetCommand::Planned(MutCommand::Unload {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;
        fleet.apply(FleetCommand::Planned(MutCommand::Load {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;
        assert!(fleet.planned.is_empty());
        fleet.apply(FleetCommand::Planned(MutCommand::Unload {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;
        fleet.planned.clear();

        fleet.apply(FleetCommand::Planned(MutCommand::Undock {
            vessel_id: "v1".into(),
        }))?;
        for command in [
            MutCommand::Load {
                vessel_id: "v1".into(),
                dest: 1,
            },
            MutCommand::Unload {
                vessel_id: "v1".into(),
                dest: 1,
            },
            MutCommand::Swap {
                vessel_id: "v1".into(),
                load_dest: 1,
                unload_dest: 1,
            },
        ] {
            assert_eq!(
                fleet
                    .apply(FleetCommand::Planned(command))
                    .unwrap_err()
                    .to_string(),
                "vessel v1 is not docked",
            );
        }

        fleet.apply(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![],
        }))?;
        fleet.apply(FleetCommand::Planned(MutCommand::SetDest {
            vessel_id: "v1".into(),
            dest: 1,
        }))?;
        fleet.apply(FleetCommand::Planned(MutCommand::SetDest {
            vessel_id: "v1".into(),
            dest: 0,
        }))?;
        assert!(fleet.planned.is_empty());
        assert_eq!(
            fleet.committed.last().vessels["v1"].state,
            VesselState::Idle { location: 0 },
        );
        Ok(())
    }

    fn cargo_test_state() -> FleetState {
        let mut vessels = HashMap::new();
        vessels.insert(
            "v1".into(),
            Vessel {
                state: VesselState::Docked { location: 0 },
                cargo: vec![0, 2, 2],
            },
        );
        vessels.insert(
            "v2".into(),
            Vessel {
                state: VesselState::Docked { location: 0 },
                cargo: vec![0, 0, 0],
            },
        );
        let mut cargo_manifest = Square::zeros(3);
        cargo_manifest[0][1] = 2;
        cargo_manifest[0][2] = 2;
        FleetState {
            vessels,
            cargo_manifest,
        }
    }

    fn cargo_event(vessel: &str, action: CargoAction) -> Event {
        Event::Cargo(CargoEvent {
            vessel: vessel.into(),
            dock: 0,
            action,
        })
    }

    #[test]
    fn cargo_reduction_undoes_the_latest_matching_command() {
        let events = vec![
            cargo_event("v1", CargoAction::Load(1)),
            cargo_event("v2", CargoAction::Load(1)),
            cargo_event("v1", CargoAction::Load(1)),
        ];
        let added = CargoEvent {
            vessel: "v1".into(),
            dock: 0,
            action: CargoAction::Unload(1),
        };

        assert!(matches!(
            reduce_cargo_event(&added, &events),
            EventLogChange::Edit {
                at: 2,
                kind: EventLogEdit::Remove
            }
        ));
    }

    #[test]
    fn inverse_cargo_operations_cancel_at_throughput_limit() -> anyhow::Result<()> {
        let state = cargo_test_state();
        let mut events = vec![cargo_event("v1", CargoAction::Load(1))];

        validate_action(
            "v1".into(),
            CargoAction::Unload(1),
            &mut events,
            &[1, 1, 1],
            &state.vessels,
            &state.cargo_manifest,
        )?;

        assert!(events.is_empty());
        Ok(())
    }

    #[test]
    fn rewrite_is_atomic_when_another_vessel_depends_on_cargo() {
        let mut state = cargo_test_state();
        state.cargo_manifest[0][1] = 0;
        state.vessels.get_mut("v1").unwrap().cargo[1] = 1;
        let mut events = vec![
            cargo_event("v1", CargoAction::Unload(1)),
            cargo_event("v2", CargoAction::Load(1)),
        ];

        let error = validate_action(
            "v1".into(),
            CargoAction::Load(1),
            &mut events,
            &[4, 4, 4],
            &state.vessels,
            &state.cargo_manifest,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "no cargo on dock 0 destined to 1");
        assert_eq!(events.len(), 2, "a rejected rewrite must be atomic");
    }

    #[test]
    fn rewrite_is_atomic_when_a_swap_depends_on_cargo() {
        let mut state = cargo_test_state();
        state.cargo_manifest[0][1] = 0;
        state.vessels.get_mut("v1").unwrap().cargo[1] = 1;
        state.vessels.get_mut("v1").unwrap().cargo[2] = 1;
        let mut events = vec![
            cargo_event("v1", CargoAction::Unload(1)),
            cargo_event(
                "v1",
                CargoAction::Swap {
                    load_dest: 1,
                    unload_dest: 2,
                },
            ),
        ];

        let error = validate_action(
            "v1".into(),
            CargoAction::Load(1),
            &mut events,
            &[4, 4, 4],
            &state.vessels,
            &state.cargo_manifest,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "no cargo on dock 0 destined to 1");
        assert_eq!(events.len(), 2, "a rejected rewrite must be atomic");
    }

    #[test]
    fn cancelling_one_side_of_a_swap_preserves_the_other_side() -> anyhow::Result<()> {
        let state = cargo_test_state();
        let mut events = vec![cargo_event(
            "v1",
            CargoAction::Swap {
                load_dest: 1,
                unload_dest: 2,
            },
        )];

        validate_action(
            "v1".into(),
            CargoAction::Unload(1),
            &mut events,
            &[4, 4, 4],
            &state.vessels,
            &state.cargo_manifest,
        )?;

        assert!(matches!(
            events.as_slice(),
            [Event::Cargo(CargoEvent {
                vessel,
                dock: 0,
                action: CargoAction::Unload(2),
            })] if vessel == "v1"
        ));
        Ok(())
    }

    #[test]
    fn partial_swap_reductions_preserve_the_unmatched_transfer() -> anyhow::Result<()> {
        use CargoAction::*;

        let cases = [
            (
                Load(1),
                Swap {
                    load_dest: 2,
                    unload_dest: 1,
                },
                Load(2),
            ),
            (
                Unload(1),
                Swap {
                    load_dest: 1,
                    unload_dest: 2,
                },
                Unload(2),
            ),
            (
                Swap {
                    load_dest: 1,
                    unload_dest: 2,
                },
                Load(2),
                Load(1),
            ),
            (
                Swap {
                    load_dest: 1,
                    unload_dest: 2,
                },
                Unload(1),
                Unload(2),
            ),
        ];

        for (existing, added, expected) in cases {
            let state = cargo_test_state();
            let mut events = vec![cargo_event("v1", existing)];
            validate_action(
                "v1".into(),
                added,
                &mut events,
                &[4, 4, 4],
                &state.vessels,
                &state.cargo_manifest,
            )?;
            assert!(matches!(
                events.as_slice(),
                [Event::Cargo(CargoEvent { action, .. })] if *action == expected
            ));
        }
        Ok(())
    }

    #[test]
    fn chained_swaps_preserve_both_unmatched_transfers() -> anyhow::Result<()> {
        let mut vessels = HashMap::new();
        vessels.insert(
            "v1".into(),
            Vessel {
                state: VesselState::Docked { location: 0 },
                cargo: vec![0, 2, 2, 2],
            },
        );
        let mut cargo_manifest = Square::zeros(4);
        cargo_manifest[0][1] = 2;
        cargo_manifest[0][2] = 2;
        cargo_manifest[0][3] = 2;
        let state = FleetState {
            vessels,
            cargo_manifest,
        };
        let mut events = vec![cargo_event(
            "v1",
            CargoAction::Swap {
                load_dest: 1,
                unload_dest: 2,
            },
        )];

        validate_action(
            "v1".into(),
            CargoAction::Swap {
                load_dest: 2,
                unload_dest: 1,
            },
            &mut events,
            &[4, 4, 4, 4],
            &state.vessels,
            &state.cargo_manifest,
        )?;
        assert!(events.is_empty(), "exact reverse swaps should cancel");

        let mut events = vec![cargo_event(
            "v1",
            CargoAction::Swap {
                load_dest: 1,
                unload_dest: 2,
            },
        )];
        validate_action(
            "v1".into(),
            CargoAction::Swap {
                load_dest: 3,
                unload_dest: 1,
            },
            &mut events,
            &[4, 4, 4, 4],
            &state.vessels,
            &state.cargo_manifest,
        )?;
        assert!(matches!(
            events.as_slice(),
            [Event::Cargo(CargoEvent {
                action: CargoAction::Swap {
                    load_dest: 3,
                    unload_dest: 2,
                },
                ..
            })]
        ));
        Ok(())
    }

    #[test]
    fn chained_swaps_preserve_an_unmatched_delivery() -> anyhow::Result<()> {
        let mut state = cargo_test_state();
        state.vessels.get_mut("v1").unwrap().cargo[0] = 1;
        state.cargo_manifest[0][1] = 1;
        state.cargo_manifest[0][2] = 1;
        let mut events = vec![cargo_event(
            "v1",
            CargoAction::Swap {
                load_dest: 1,
                unload_dest: 0,
            },
        )];

        validate_action(
            "v1".into(),
            CargoAction::Swap {
                load_dest: 2,
                unload_dest: 1,
            },
            &mut events,
            &[4, 4, 4],
            &state.vessels,
            &state.cargo_manifest,
        )?;

        assert!(
            matches!(
                events.as_slice(),
                [Event::Cargo(CargoEvent {
                    action: CargoAction::Swap {
                        load_dest: 2,
                        unload_dest: 0,
                    },
                    ..
                })]
            ),
            "{:?}",
            events.as_slice()
        );
        Ok(())
    }

    #[test]
    fn cargo_manifest_at_its_destination_is_rejected() -> anyhow::Result<()> {
        let init = Init {
            cargo_manifest: vec![Cargo {
                dock: 0,
                dest: 0,
                quantity: 3,
            }],
            dock_throughput: vec![1, 1],
            travel_times: vec![vec![0, 1], vec![1, 0]],
            vessels: vec![],
        };
        assert_eq!(
            init_fleet(init).err().unwrap().to_string(),
            "cargo at location 0 is already at its destination",
        );

        let mut fleet = init_fleet(Init {
            cargo_manifest: vec![],
            dock_throughput: vec![1, 1],
            travel_times: vec![vec![0, 1], vec![1, 0]],
            vessels: vec![],
        })?;

        let error = apply_tick(
            &mut fleet,
            &[Cargo {
                dock: 1,
                dest: 1,
                quantity: 4,
            }],
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "cargo at location 1 is already at its destination",
        );
        assert_eq!(fleet.committed.rest_len(), 0);
        Ok(())
    }

    #[test]
    fn application() -> anyhow::Result<()> {
        use MutCommand::*;

        fn vessel_state(fleet: &Fleet, view: View, vessel: &str) -> VesselState {
            let Status::Ok(state) = fleet.status(view) else {
                panic!("expected fleet status to be ok");
            };
            state.vessels[vessel].state
        }

        fn vessel_cargo(fleet: &Fleet, view: View, vessel: &str, dest: usize) -> usize {
            let Status::Ok(state) = fleet.status(view) else {
                panic!("expected fleet status to be ok");
            };
            state.vessels[vessel].cargo[dest]
        }

        fn cargo_at(fleet: &Fleet, view: View, dock: usize, dest: usize) -> usize {
            let Status::Ok(state) = fleet.status(view) else {
                panic!("expected fleet status to be ok");
            };
            state
                .cargo
                .iter()
                .find(|cargo| cargo.dock == dock && cargo.dest == dest)
                .map_or(0, |cargo| cargo.quantity)
        }

        fn plan(fleet: &mut Fleet, command: MutCommand) -> anyhow::Result<()> {
            fleet.apply(FleetCommand::Planned(command))
        }

        fn tick(fleet: &mut Fleet, cargo_manifest: Vec<Cargo>) -> anyhow::Result<()> {
            fleet.apply(FleetCommand::Advance(Tick::Tick { cargo_manifest }))
        }

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
        plan(
            &mut fleet,
            SetDest {
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
            vessel_state(&fleet, View::Planned, "v1"),
        );
        assert_eq!(
            VesselState::Idle { location: 0 },
            vessel_state(&fleet, View::LastCommitted, "v1"),
        );

        plan(
            &mut fleet,
            SetDest {
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
            vessel_state(&fleet, View::Planned, "v1"),
        );
        tick(&mut fleet, vec![])?;
        assert_eq!(
            VesselState::Transit {
                from: 0,
                to: 1,
                arrives_at_tick: 4
            },
            vessel_state(&fleet, View::LastCommitted, "v1"),
        );
        assert!(fleet.planned.is_empty());

        assert!(
            plan(
                &mut fleet,
                Undock {
                    vessel_id: "v2".into()
                }
            )
            .is_err()
        );
        assert!(fleet.planned.is_empty());

        plan(
            &mut fleet,
            Dock {
                vessel_id: "v2".into(),
            },
        )?;
        assert_eq!(
            VesselState::Docked { location: 2 },
            vessel_state(&fleet, View::Planned, "v2"),
        );
        assert_eq!(
            VesselState::Idle { location: 2 },
            vessel_state(&fleet, View::LastCommitted, "v2"),
        );
        tick(
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
        assert_eq!(
            VesselState::Docked { location: 2 },
            vessel_state(&fleet, View::LastCommitted, "v2"),
        );
        assert_eq!(2, cargo_at(&fleet, View::LastCommitted, 2, 0));

        // v2 has no cargo to unload
        // dock 2 only has cargo destined to dock 0
        for i in 0..3 {
            let planned_len = fleet.planned.len();
            assert!(
                plan(
                    &mut fleet,
                    Unload {
                        vessel_id: "v2".into(),
                        dest: i,
                    }
                )
                .is_err()
            );
            assert_eq!(planned_len, fleet.planned.len());
            if i > 0 {
                let planned_len = fleet.planned.len();
                assert!(
                    plan(
                        &mut fleet,
                        Load {
                            vessel_id: "v2".into(),
                            dest: i
                        }
                    )
                    .is_err()
                );
                assert_eq!(planned_len, fleet.planned.len());
            }
        }
        plan(
            &mut fleet,
            Load {
                vessel_id: "v2".into(),
                dest: 0,
            },
        )?;
        assert_eq!(1, vessel_cargo(&fleet, View::Planned, "v2", 0));
        assert_eq!(0, vessel_cargo(&fleet, View::LastCommitted, "v2", 0));
        assert_eq!(1, cargo_at(&fleet, View::Planned, 2, 0));
        assert_eq!(2, cargo_at(&fleet, View::LastCommitted, 2, 0));

        // reached max operations throughput
        assert!(
            plan(
                &mut fleet,
                Load {
                    vessel_id: "v2".into(),
                    dest: 0,
                }
            )
            .is_err()
        );
        tick(&mut fleet, vec![])?;
        assert_eq!(1, vessel_cargo(&fleet, View::LastCommitted, "v2", 0));
        assert_eq!(1, cargo_at(&fleet, View::LastCommitted, 2, 0));

        plan(
            &mut fleet,
            Undock {
                vessel_id: "v2".into(),
            },
        )?;
        assert_eq!(
            VesselState::Idle { location: 2 },
            vessel_state(&fleet, View::Planned, "v2"),
        );
        assert_eq!(
            VesselState::Docked { location: 2 },
            vessel_state(&fleet, View::LastCommitted, "v2"),
        );
        tick(&mut fleet, vec![])?;
        assert_eq!(
            VesselState::Idle { location: 2 },
            vessel_state(&fleet, View::LastCommitted, "v2"),
        );

        Ok(())
    }
}
