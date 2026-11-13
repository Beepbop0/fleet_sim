use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct VesselLocation {
    pub vessel: String,
    pub location: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", content = "parameters", rename_all = "lowercase")]
pub enum Command {
    Tick,
    GetState,
    Init {
        travel_times: Vec<Vec<usize>>,
        vessels: Vec<VesselLocation>,
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
