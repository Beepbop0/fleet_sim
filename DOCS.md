# Project Structure

src/*.rs files that do not include src/bin/*.rs are all related to the fleet sim

src/bin/gen_input is a utility to easily convert commands into their JSON representation for input into the fleet sim.
Modify main to include the set of commands you want to issue to the fleet sim.
```
cargo run --bin gen_input > <input_file>
cargo run --bin fleet_sim < <input_file>
```
