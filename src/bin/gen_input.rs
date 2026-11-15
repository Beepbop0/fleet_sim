use fleet_sim::{Cargo, Command, Init, InitCommand, VesselLocation};

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

    let init = InitCommand::Init(Init {
        travel_times: vec![vec![0, 4, 7], vec![4, 0, 3], vec![7, 3, 0]],
        dock_throughput: vec![2, 4, 1],
        vessels,
        cargo_manifest: vec![],
    });
    println!("{}", serde_json::to_string(&init)?);

    let commands = vec![
        Command::Tick {
            cargo_manifest: vec![
                Cargo {
                    dock: 0,
                    dest: 2,
                    quantity: 1,
                },
                Cargo {
                    dock: 0,
                    dest: 1,
                    quantity: 1,
                },
            ],
        },
        Command::Dock {
            vessel_id: "v1".into(),
        },
        Command::tick(),
        Command::Load {
            vessel_id: "v1".into(),
            dest: 2,
        },
        Command::Load {
            vessel_id: "v1".into(),
            dest: 1,
        },
        Command::tick(),
        Command::Undock {
            vessel_id: "v1".into(),
        },
        Command::tick(),
        Command::SetDest {
            vessel_id: "v1".into(),
            dest: 2,
        },
        Command::tick(),
        Command::tick(),
        Command::tick(),
        Command::tick(),
        Command::tick(),
        Command::tick(),
    ];
    for cmd in commands {
        println!("{}", serde_json::to_string(&cmd)?);
    }

    Ok(())
}
