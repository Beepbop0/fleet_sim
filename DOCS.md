# Project Structure

src/*.rs files that do not include src/bin/*.rs are all related to the fleet sim

src/bin/gen_input is a utility to easily convert commands into their JSON representation for input into the fleet sim.
Modify main to include the set of commands you want to issue to the fleet sim.
```
cargo run --bin gen_input > <input_file>
cargo run --bin fleet_sim < <input_file>
```

## Interactive text input

The `play` binary accepts one whitespace-separated command per line and prints the
same JSON status responses as the main binary:

```text
cargo run --bin play
init 0,4,7;4,0,3;7,3,0 v1:0,v2:2 2,4,1
tick 0:2:1,0:1:1
dock v1
tick
load v1 2
set_destination v1 2
get_state
get_state_at 1
```

To start with an initialized fleet, put one `init` command (using the same
grammar) in a file and pass its path to `play`:

```text
cargo run --bin play -- fleet.txt
```

The file may contain blank lines and comments beginning with `#`, but exactly
one non-comment command is required and it must be `init`. `play` exits with a
non-zero status before reading interactive commands if the file cannot be read,
parsed, or converted into a valid fleet. The initial fleet state is printed to
stdout on success.

The grammar and value formats are printed to stderr when `play` starts. Enter
`help` at any time to print them again; command results remain JSON on stdout.
The `play` binary enriches each successful state payload with an `ops` array
containing the normalized operations pending for the current tick. Committed
states returned by `tick` or `get_state_at` have an empty `ops` array. This
additional field is not emitted by the regular `fleet_sim` binary.

The full command grammar is:

```text
init ROWS VESSELS THROUGHPUT [CARGO]
tick [CARGO]
get_state
get_state_at TICK
set_destination VESSEL DESTINATION
dock VESSEL
undock VESSEL
load VESSEL DESTINATION
unload VESSEL DESTINATION
swap VESSEL LOAD_DESTINATION UNLOAD_DESTINATION
help
```

Rows are comma-separated and joined with semicolons. Vessels use
`ID:LOCATION`, and cargo uses `LOCATION:DESTINATION:QUANTITY`; multiple vessels
or cargo entries are comma-separated. Use `-` for an empty list. Blank lines
and lines beginning with `#` are ignored. `state`, `state_at`, and `set_dest`
are accepted as shorter aliases.
