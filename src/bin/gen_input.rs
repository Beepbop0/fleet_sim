use fleet_sim::{Command, VesselLocation};

fn main() -> serde_json::Result<()> {
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

    let commands = vec![
        Command::Init {
            travel_times: vec![vec![0, 4, 7], vec![4, 0, 3], vec![7, 3, 0]],
            vessels,
        },
        Command::SetDest {
            vessel: "v1".into(),
            dest: 2,
        },
        Command::SetDest {
            vessel: "v1".into(),
            dest: 1,
        },
        Command::Tick,
        Command::Tick,
        Command::Tick,
        Command::SetDest {
            vessel: "v1".into(),
            dest: 2,
        },
    ];
    for cmd in commands {
        println!("{}", serde_json::to_string(&cmd)?);
    }

    Ok(())
}
