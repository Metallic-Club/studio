//! Print the schema fingerprint for comparing protocol peers, without loading a scene.
//! Also the way to get the new value for the `registry_hash_is_pinned` test.

use std::process::ExitCode;

fn main() -> ExitCode {
    match metallic_core::replication::registry_hash() {
        Ok(hash) => {
            for byte in hash {
                print!("{byte:02x}");
            }
            println!();
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("registry has no fingerprint: {error}");
            ExitCode::FAILURE
        }
    }
}
