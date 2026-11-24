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
                let vessel = fleet_state
                    .vessels
                    .get_mut(id)
                    .expect("planned vessel event references a registered vessel");
                vessel.state = *state;
            }
            Event::Cargo(CargoEvent {
                vessel,
                dock,
                action,
            }) => {
                let vessel_cargo = &mut fleet_state
                    .vessels
                    .get_mut(vessel)
                    .expect("planned cargo event references a registered vessel")
                    .cargo;
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
    use proptest::prelude::*;

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

    struct TestStateBuilder {
        state: FleetState,
    }

    impl TestStateBuilder {
        fn new(locations: usize) -> Self {
            Self {
                state: FleetState {
                    vessels: HashMap::new(),
                    cargo_manifest: Square::zeros(locations),
                },
            }
        }

        fn vessel(mut self, id: &str, state: VesselState, cargo: Vec<usize>) -> Self {
            assert_eq!(cargo.len(), self.state.cargo_manifest.n());
            assert!(
                self.state
                    .vessels
                    .insert(id.into(), Vessel { state, cargo })
                    .is_none()
            );
            self
        }

        fn docked_vessel(self, id: &str, cargo: Vec<usize>) -> Self {
            self.vessel(id, VesselState::Docked { location: 0 }, cargo)
        }

        fn idle_vessel(self, id: &str, cargo: Vec<usize>) -> Self {
            self.vessel(id, VesselState::Idle { location: 0 }, cargo)
        }

        fn dock_cargo(mut self, destination: usize, quantity: usize) -> Self {
            self.state.cargo_manifest[0][destination] = quantity;
            self
        }

        fn build(self) -> FleetState {
            self.state
        }

        fn build_fleet(self, throughput: usize) -> Fleet {
            let locations = self.state.cargo_manifest.n();
            Fleet {
                travel_times: Square::try_from(travel_times(locations))
                    .expect("test travel times form a square matrix"),
                dock_throughput: vec![throughput; locations],
                committed: NonEmpty::new(self.build()),
                planned: vec![],
            }
        }
    }

    fn cargo_event(vessel: &str, action: CargoAction) -> Event {
        Event::Cargo(CargoEvent {
            vessel: vessel.into(),
            dock: 0,
            action,
        })
    }

    fn cargo_action_strategy() -> impl Strategy<Value = CargoAction> {
        prop_oneof![
            (1usize..4).prop_map(CargoAction::Load),
            (0usize..4).prop_map(CargoAction::Unload),
            (1usize..4, 0usize..4).prop_map(|(load_dest, unload_dest)| CargoAction::Swap {
                load_dest,
                unload_dest,
            }),
        ]
    }

    fn planned_command_strategy() -> impl Strategy<Value = MutCommand> {
        (0u8..6, any::<bool>(), 1usize..4, 1usize..4).prop_map(
            |(kind, second_vessel, first_dest, second_dest)| {
                let vessel_id = if second_vessel { "v2" } else { "v1" }.into();
                match kind {
                    0 => MutCommand::SetDest {
                        vessel_id,
                        dest: first_dest,
                    },
                    1 => MutCommand::Dock { vessel_id },
                    2 => MutCommand::Undock { vessel_id },
                    3 => MutCommand::Load {
                        vessel_id,
                        dest: first_dest,
                    },
                    4 => MutCommand::Unload {
                        vessel_id,
                        dest: first_dest,
                    },
                    _ => MutCommand::Swap {
                        vessel_id,
                        load_dest: first_dest,
                        unload_dest: second_dest,
                    },
                }
            },
        )
    }

    fn integration_test_fleet() -> Fleet {
        TestStateBuilder::new(4)
            .docked_vessel("v1", vec![0; 4])
            .idle_vessel("v2", vec![0; 4])
            .dock_cargo(1, 32)
            .dock_cargo(2, 32)
            .dock_cargo(3, 32)
            .build_fleet(8)
    }

    fn travel_times(locations: usize) -> Vec<Vec<usize>> {
        (0..locations)
            .map(|from| (0..locations).map(|to| usize::from(from != to)).collect())
            .collect()
    }

    fn cargo_property_state() -> FleetState {
        TestStateBuilder::new(4)
            .docked_vessel("v1", vec![32; 4])
            .dock_cargo(1, 32)
            .dock_cargo(2, 32)
            .dock_cargo(3, 32)
            .build()
    }

    proptest! {
        #[test]
        fn returning_to_current_location_undoes_a_planned_destination(
            (locations, current, destination, duration) in (2usize..8).prop_flat_map(|locations| {
                (Just(locations), 0..locations, 0..locations - 1, 1usize..20)
            }).prop_map(|(locations, current, offset, duration)| {
                let destination = (current + offset + 1) % locations;
                (locations, current, destination, duration)
            }),
        ) {
            let travel_times = (0..locations)
                .map(|from| {
                    (0..locations)
                        .map(|to| {
                            if from == to {
                                0
                            } else if (from, to) == (current, destination) {
                                duration
                            } else {
                                1
                            }
                        })
                        .collect()
                })
                .collect();
            let mut fleet = init_fleet(Init {
                cargo_manifest: vec![],
                dock_throughput: vec![1; locations],
                travel_times,
                vessels: vec![VesselLocation {
                    vessel: "v1".into(),
                    location: current,
                }],
            }).unwrap();
            let committed = fleet.committed.last().clone();

            fleet.apply(FleetCommand::Planned(MutCommand::SetDest {
                vessel_id: "v1".into(),
                dest: destination,
            })).unwrap();
            let transit_is_planned = matches!(
                fleet.planned.as_slice(),
                [Event::Vessel {
                    id,
                    state: VesselState::Transit {
                        from,
                        to,
                        arrives_at_tick,
                    },
                }] if id == "v1"
                    && *from == current
                    && *to == destination
                    && *arrives_at_tick == duration
            );
            prop_assert!(transit_is_planned);

            fleet.apply(FleetCommand::Planned(MutCommand::SetDest {
                vessel_id: "v1".into(),
                dest: current,
            })).unwrap();

            prop_assert!(fleet.planned.is_empty());
            prop_assert_eq!(fleet.committed.last(), &committed);
            let Status::Ok(planned) = fleet.status(View::Planned) else {
                prop_assert!(false, "planned status should be valid");
                return Ok(());
            };
            prop_assert_eq!(&*planned.vessels, &committed.vessels);
        }

        #[test]
        fn vessels_arrive_exactly_on_their_scheduled_ticks(
            durations in prop::collection::vec(1usize..12, 1..7),
        ) {
            let locations = durations.len() + 1;
            let travel_times = (0..locations)
                .map(|from| {
                    (0..locations)
                        .map(|to| match (from, to) {
                            (from, to) if from == to => 0,
                            (0, to) => durations[to - 1],
                            (from, 0) => durations[from - 1],
                            _ => 1,
                        })
                        .collect()
                })
                .collect();
            let vessels = durations
                .iter()
                .enumerate()
                .map(|(index, _)| VesselLocation {
                    vessel: format!("v{index}"),
                    location: 0,
                })
                .collect();
            let mut fleet = init_fleet(Init {
                cargo_manifest: vec![],
                dock_throughput: vec![1; locations],
                travel_times,
                vessels,
            }).unwrap();

            for (index, _) in durations.iter().enumerate() {
                fleet.apply(FleetCommand::Planned(MutCommand::SetDest {
                    vessel_id: format!("v{index}"),
                    dest: index + 1,
                })).unwrap();
            }

            for tick in 1..=*durations.iter().max().unwrap() {
                fleet.apply(FleetCommand::Advance(Tick::Tick {
                    cargo_manifest: vec![],
                })).unwrap();

                for (index, &arrives_at_tick) in durations.iter().enumerate() {
                    let destination = index + 1;
                    let expected = if tick < arrives_at_tick {
                        VesselState::Transit {
                            from: 0,
                            to: destination,
                            arrives_at_tick,
                        }
                    } else {
                        VesselState::Idle {
                            location: destination,
                        }
                    };
                    prop_assert_eq!(
                        fleet.committed.last().vessels[&format!("v{index}")].state,
                        expected
                    );
                }
            }
        }

        #[test]
        fn cargo_reduction_preserves_effect_and_recalculates_ops(
            actions in prop::collection::vec(cargo_action_strategy(), 0..7),
            new_action in cargo_action_strategy(),
        ) {
            let state = cargo_property_state();
            let mut events: Vec<_> = actions
                .into_iter()
                .map(|action| cargo_event("v1", action))
                .collect();
            let unreduced_op_count = events.len() + 1;

            let mut expected = state.clone();
            apply_events(&mut expected, &events).unwrap();
            apply_events(&mut expected, &[cargo_event("v1", new_action)]).unwrap();

            validate_action(
                "v1".into(),
                new_action,
                &mut events,
                &[unreduced_op_count; 4],
                &state.vessels,
                &state.cargo_manifest,
            ).unwrap();

            let mut actual = state.clone();
            apply_events(&mut actual, &events).unwrap();
            prop_assert_eq!(actual.cargo_manifest[0][0], 0);
            prop_assert_eq!(actual, expected);
            prop_assert!(events.len() <= unreduced_op_count);
        }

        #[test]
        fn inverse_cargo_operation_releases_its_throughput_slot(
            destination in 1usize..4,
            load_first in any::<bool>(),
        ) {
            let state = cargo_property_state();
            let (old, new) = if load_first {
                (CargoAction::Load(destination), CargoAction::Unload(destination))
            } else {
                (CargoAction::Unload(destination), CargoAction::Load(destination))
            };
            let mut events = vec![cargo_event("v1", old)];

            validate_action(
                "v1".into(),
                new,
                &mut events,
                &[1; 4],
                &state.vessels,
                &state.cargo_manifest,
            ).unwrap();

            prop_assert!(events.is_empty());
        }

        #[test]
        fn non_reducible_operation_at_throughput_limit_is_atomic(
            destination in 1usize..4,
            load in any::<bool>(),
        ) {
            let state = cargo_property_state();
            let action = if load {
                CargoAction::Load(destination)
            } else {
                CargoAction::Unload(destination)
            };
            let mut events = vec![cargo_event("v1", action)];

            let result = validate_action(
                "v1".into(),
                action,
                &mut events,
                &[1; 4],
                &state.vessels,
                &state.cargo_manifest,
            );

            prop_assert!(result.is_err());
            let unchanged = matches!(
                events.as_slice(),
                [Event::Cargo(CargoEvent { action: unchanged, .. })] if *unchanged == action
            );
            prop_assert!(unchanged);
        }

        #[test]
        fn planned_command_sequences_are_atomic_replayable_and_committable(
            commands in prop::collection::vec(planned_command_strategy(), 0..20),
        ) {
            let mut fleet = integration_test_fleet();

            for command in commands {
                let committed = fleet.committed.last().clone();
                let planned = format!("{:?}", fleet.planned);
                let result = fleet.apply(FleetCommand::Planned(command));

                prop_assert_eq!(fleet.committed.last(), &committed);
                if result.is_err() {
                    prop_assert_eq!(format!("{:?}", fleet.planned), planned);
                }

                let mut projected = committed;
                apply_events(&mut projected, &fleet.planned).unwrap();
                for dock in 0..fleet.dock_throughput.len() {
                    let operations = fleet.planned.iter().filter(|event| {
                        matches!(event, Event::Cargo(cargo) if cargo.dock == dock)
                    }).count();
                    prop_assert!(operations <= fleet.dock_throughput[dock]);
                }
            }

            let mut expected = fleet.committed.last().clone();
            apply_events(&mut expected, &fleet.planned).unwrap();
            let next_tick = fleet.committed.rest_len() + 1;
            for vessel in expected.vessels.values_mut() {
                if let VesselState::Transit { to, arrives_at_tick, .. } = vessel.state
                    && arrives_at_tick == next_tick
                {
                    vessel.state = VesselState::Idle { location: to };
                }
            }

            fleet.apply(FleetCommand::Advance(Tick::Tick {
                cargo_manifest: vec![],
            })).unwrap();
            prop_assert_eq!(fleet.committed.last(), &expected);
            prop_assert!(fleet.planned.is_empty());
        }

        #[test]
        fn undocking_removes_only_that_vessels_operations_and_releases_throughput(
            v1_operations in 1usize..5,
            v2_operations in 1usize..5,
            undock_v2 in any::<bool>(),
            v1_destination in 1usize..4,
            v2_destination in 1usize..4,
        ) {
            let throughput = v1_operations + v2_operations;
            let mut fleet = TestStateBuilder::new(4)
                .docked_vessel("v1", vec![0, 32, 32, 32])
                .docked_vessel("v2", vec![0; 4])
                .dock_cargo(1, 32)
                .dock_cargo(2, 32)
                .dock_cargo(3, 32)
                .build_fleet(throughput);

            for (vessel, destination, operations) in [
                ("v1", v1_destination, v1_operations),
                ("v2", v2_destination, v2_operations),
            ] {
                for _ in 0..operations {
                    fleet.apply(FleetCommand::Planned(MutCommand::Load {
                        vessel_id: vessel.into(),
                        dest: destination,
                    })).unwrap();
                }
            }

            let (removed, retained, retained_destination) = if undock_v2 {
                ("v2", "v1", v1_destination)
            } else {
                ("v1", "v2", v2_destination)
            };
            let removed_operations = if undock_v2 { v2_operations } else { v1_operations };
            let retained_before = fleet.planned.iter().filter(|event| {
                matches!(event, Event::Cargo(cargo) if cargo.vessel == retained)
            }).count();

            fleet.apply(FleetCommand::Planned(MutCommand::Undock {
                vessel_id: removed.into(),
            })).unwrap();

            let removed_remaining = fleet.planned.iter().any(|event| {
                matches!(event, Event::Cargo(cargo) if cargo.vessel == removed)
            });
            let retained_after = fleet.planned.iter().filter(|event| {
                matches!(event, Event::Cargo(cargo) if cargo.vessel == retained)
            }).count();
            prop_assert!(!removed_remaining);
            prop_assert_eq!(retained_after, retained_before);

            for _ in 0..removed_operations {
                fleet.apply(FleetCommand::Planned(MutCommand::Load {
                    vessel_id: retained.into(),
                    dest: retained_destination,
                })).unwrap();
            }
            let planned = format!("{:?}", fleet.planned);
            let extra = fleet.apply(FleetCommand::Planned(MutCommand::Load {
                vessel_id: retained.into(),
                dest: retained_destination,
            }));
            prop_assert!(extra.is_err());
            prop_assert_eq!(format!("{:?}", fleet.planned), planned);

            let mut projected = fleet.committed.last().clone();
            apply_events(&mut projected, &fleet.planned).unwrap();
            prop_assert_eq!(
                projected.vessels[removed].state,
                VesselState::Idle { location: 0 }
            );
        }

        #[test]
        fn dependency_ordering_makes_invalid_reductions_atomic(
            destination in 1usize..4,
            swap_dependency in any::<bool>(),
            prefix_operations in 0usize..5,
            suffix_operations in 0usize..5,
        ) {
            let other_destination = destination % 3 + 1;
            let vessel_cargo = (0..4)
                .map(|dest| usize::from(dest == destination || dest == other_destination))
                .collect();
            let state = (1..4)
                .filter(|&dest| dest != destination)
                .fold(
                    TestStateBuilder::new(4)
                        .docked_vessel("v1", vessel_cargo)
                        .docked_vessel("v2", vec![0; 4]),
                    |state, dest| state.dock_cargo(dest, 32),
                )
                .build();

            let unrelated = || cargo_event("v2", CargoAction::Load(other_destination));
            let mut events: Vec<_> = (0..prefix_operations).map(|_| unrelated()).collect();
            events.push(cargo_event("v1", CargoAction::Unload(destination)));
            events.push(if swap_dependency {
                cargo_event(
                    "v1",
                    CargoAction::Swap {
                        load_dest: destination,
                        unload_dest: other_destination,
                    },
                )
            } else {
                cargo_event("v2", CargoAction::Load(destination))
            });
            events.extend((0..suffix_operations).map(|_| unrelated()));

            let mut projected = state.clone();
            apply_events(&mut projected, &events).unwrap();
            let original = format!("{:?}", events);
            let result = validate_action(
                "v1".into(),
                CargoAction::Load(destination),
                &mut events,
                &[prefix_operations + suffix_operations + 3; 4],
                &state.vessels,
                &state.cargo_manifest,
            );

            prop_assert!(result.is_err());
            prop_assert_eq!(format!("{:?}", events), original);
        }

        #[test]
        fn destination_bearing_commands_reject_unknown_locations_atomically(
            (locations, valid, invalid, command_kind) in (1usize..6).prop_flat_map(|locations| {
                (
                    Just(locations),
                    0..locations,
                    locations..locations + 10,
                    0u8..5,
                )
            }),
        ) {
            let mut fleet = TestStateBuilder::new(locations)
                .docked_vessel("v1", vec![0; locations])
                .build_fleet(4);
            let command = match command_kind {
                0 => MutCommand::SetDest {
                    vessel_id: "v1".into(),
                    dest: invalid,
                },
                1 => MutCommand::Load {
                    vessel_id: "v1".into(),
                    dest: invalid,
                },
                2 => MutCommand::Unload {
                    vessel_id: "v1".into(),
                    dest: invalid,
                },
                3 => MutCommand::Swap {
                    vessel_id: "v1".into(),
                    load_dest: invalid,
                    unload_dest: valid,
                },
                _ => MutCommand::Swap {
                    vessel_id: "v1".into(),
                    load_dest: valid,
                    unload_dest: invalid,
                },
            };
            let committed = fleet.committed.last().clone();
            let planned = format!("{:?}", fleet.planned);

            let error = fleet.apply(FleetCommand::Planned(command)).unwrap_err();

            prop_assert_eq!(error.to_string(), format!("unknown location {invalid}"));
            prop_assert_eq!(fleet.committed.last(), &committed);
            prop_assert_eq!(format!("{:?}", fleet.planned), planned);
        }

        #[test]
        fn valid_manifests_change_only_the_requested_off_diagonal_cell(
            (locations, dock, destination, quantity) in (2usize..7).prop_flat_map(|locations| {
                (Just(locations), 0..locations, 0..locations - 1, 1usize..100)
            }).prop_map(|(locations, dock, offset, quantity)| {
                let destination = (dock + offset + 1) % locations;
                (locations, dock, destination, quantity)
            }),
        ) {
            let mut fleet = init_fleet(Init {
                cargo_manifest: vec![],
                dock_throughput: vec![1; locations],
                travel_times: travel_times(locations),
                vessels: vec![],
            }).unwrap();

            apply_tick(&mut fleet, &[Cargo { dock, dest: destination, quantity }]).unwrap();

            let manifest = &fleet.committed.last().cargo_manifest;
            for from in 0..locations {
                for to in 0..locations {
                    let expected = if (from, to) == (dock, destination) {
                        quantity
                    } else {
                        0
                    };
                    prop_assert_eq!(manifest[from][to], expected);
                }
            }
            prop_assert_eq!(fleet.committed.rest_len(), 1);
        }

        #[test]
        fn invalid_manifests_do_not_advance_the_tick(
            (locations, dock, destination, invalid_kind) in (2usize..7).prop_flat_map(|locations| {
                (Just(locations), 0..locations, 0..locations, 0u8..4)
            }),
        ) {
            let mut fleet = init_fleet(Init {
                cargo_manifest: vec![],
                dock_throughput: vec![1; locations],
                travel_times: travel_times(locations),
                vessels: vec![],
            }).unwrap();
            let valid_destination = (dock + 1) % locations;
            let cargo = match invalid_kind {
                0 => Cargo { dock, dest: dock, quantity: 1 },
                1 => Cargo { dock, dest: valid_destination, quantity: 0 },
                2 => Cargo { dock: locations, dest: destination, quantity: 1 },
                _ => Cargo { dock, dest: locations, quantity: 1 },
            };
            let committed = fleet.committed.last().clone();

            let result = apply_tick(&mut fleet, &[cargo]);

            prop_assert!(result.is_err());
            prop_assert_eq!(fleet.committed.rest_len(), 0);
            prop_assert_eq!(fleet.committed.last(), &committed);
            prop_assert!(fleet.planned.is_empty());
        }
    }
}
