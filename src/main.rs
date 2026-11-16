use fleet_sim::{
    Command, InitCommand, TopCommand,
    sim::{Fleet, Status, View},
};
use std::io::{self, BufRead};

fn main() -> anyhow::Result<()> {
    let mut buf = String::new();
    let mut stdin = io::stdin().lock();
    let mut sim = None;
    loop {
        buf.clear();
        if stdin.read_line(&mut buf)? == 0 {
            break;
        }
        let command: TopCommand = serde_json::from_str(&buf[..])?;
        let status = match (command, sim.as_mut()) {
            (TopCommand::Fleet(_), None) => Status::Err {
                message: "must call init first".into(),
            },
            (TopCommand::Init(_), Some(_)) => Status::Err {
                message: "already called init".into(),
            },
            (TopCommand::Init(InitCommand::Init(init)), None) => match Fleet::init(init) {
                Ok(fleet) => {
                    sim = Some(fleet);
                    sim.as_ref().unwrap().status(View::LastCommitted)
                }
                Err(err) => Status::Err {
                    message: err.to_string(),
                },
            },
            (TopCommand::Fleet(command), Some(fleet)) => {
                let view = match &command {
                    Command::GetState {} | Command::SetDest { .. } => View::Planned,
                    Command::GetStateAt { tick } => View::CommittedAt(*tick),
                    _ => View::LastCommitted,
                };
                match fleet.apply(command) {
                    Ok(_) => fleet.status(view),
                    Err(err) => Status::Err {
                        message: err.to_string(),
                    },
                }
            }
        };

        println!("{}", serde_json::to_string(&status)?);
    }
    Ok(())
}
