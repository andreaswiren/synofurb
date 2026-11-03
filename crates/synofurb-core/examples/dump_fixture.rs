//! Prints the built-in fixture of the owner's DS918+ (raw inspection output, `KV|...` lines) so it can be fed to the CLI:
//!
//! ```text
//! cargo run -q -p synofurb-core --example dump_fixture > tests/cli/ds918-real.inspect.txt
//! synofurb-cli plan --facts tests/cli/ds918-real.inspect.txt --raid shr1 --copies 4 --kernel stable
//! ```
fn main() {
    print!("{}", synofurb_core::inspect::fixtures::ds918_two_volumes());
}
