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
#[serde(tag = "command", content = "parameters", rename_all = "lowercase")]
pub enum InitCommand {
    Init(Init),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", content = "parameters", rename_all = "snake_case")]
pub enum Command {
    Tick {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cargo_manifest: Vec<Cargo>,
    },
    GetState {},
    GetStateAt {
        tick: usize,
    },
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
pub enum TopCommand {
    Init(InitCommand),
    Fleet(Command),
}

impl Command {
    pub fn tick() -> Self {
        Self::Tick {
            cargo_manifest: vec![],
        }
    }
}
