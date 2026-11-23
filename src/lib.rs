pub mod nonempty;
pub mod sim;
pub mod square;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct VesselLocation {
    #[serde(rename = "id")]
    pub vessel: String,
    pub location: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Cargo {
    pub quantity: usize,
    #[serde(rename = "location")]
    pub dock: usize,
    #[serde(rename = "destination")]
    pub dest: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Init {
    pub travel_times: Vec<Vec<usize>>,
    pub vessels: Vec<VesselLocation>,
    pub dock_throughput: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cargo_manifest: Vec<Cargo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", content = "parameters", rename_all = "snake_case")]
pub enum InitCommand {
    Init(Init),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", content = "parameters", rename_all = "snake_case")]
pub enum Tick {
    Tick {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cargo_manifest: Vec<Cargo>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", content = "parameters", rename_all = "snake_case")]
pub enum ViewCommand {
    GetState {},
    GetStateAt { tick: isize },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", content = "parameters", rename_all = "snake_case")]
pub enum MutCommand {
    #[serde(rename = "set_destination")]
    SetDest {
        vessel_id: String,
        #[serde(rename = "destination")]
        dest: usize,
    },
    Dock {
        vessel_id: String,
    },
    Undock {
        vessel_id: String,
    },
    Load {
        vessel_id: String,
        #[serde(rename = "destination")]
        dest: usize,
    },
    Unload {
        vessel_id: String,
        #[serde(rename = "destination")]
        dest: usize,
    },
    Swap {
        vessel_id: String,
        #[serde(rename = "load_destination")]
        load_dest: usize,
        #[serde(rename = "unload_destination")]
        unload_dest: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FleetCommand {
    Advance(Tick),
    View(ViewCommand),
    Planned(MutCommand),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TopCommand {
    Init(InitCommand),
    Fleet(FleetCommand),
}

impl TopCommand {
    pub fn tick() -> Self {
        TopCommand::Fleet(FleetCommand::Advance(Tick::Tick {
            cargo_manifest: vec![],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state_commands_from_specification() {
        let get_state: TopCommand =
            serde_json::from_str(r#"{"command":"get_state","parameters":{}}"#).unwrap();
        assert!(matches!(
            get_state,
            TopCommand::Fleet(FleetCommand::View(ViewCommand::GetState {}))
        ));

        let get_state_at: TopCommand =
            serde_json::from_str(r#"{"command":"get_state_at","parameters":{"tick":0}}"#).unwrap();
        assert!(matches!(
            get_state_at,
            TopCommand::Fleet(FleetCommand::View(ViewCommand::GetStateAt { tick: 0 }))
        ));
    }
}
