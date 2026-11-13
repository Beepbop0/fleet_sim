use serde::{Deserialize, Serialize};

pub type CowStr = std::borrow::Cow<'static, str>;

#[derive(Debug, Serialize, Deserialize)]
pub struct VesselLocation {
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
#[serde(tag = "command", content = "parameters", rename_all = "lowercase")]
pub enum Command {
    Tick {
        #[serde(default)]
        cargo_manifest: Vec<Cargo>,
    },
    GetState,
    Init {
        travel_times: Vec<Vec<usize>>,
        vessels: Vec<VesselLocation>,
        #[serde(default)]
        cargo_manifest: Vec<Cargo>,
    },
    #[serde(rename = "set_destination")]
    SetDest {
        vessel: String,
        #[serde(rename = "destination")]
        dest: usize,
    },
    Dock {
        vessel_id: String,
    },
    Undock {
        vessel_id: String,
    },
}
