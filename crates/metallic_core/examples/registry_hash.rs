//! Print the schema fingerprint for comparing protocol peers, without loading a scene.

fn main() {
    for byte in metallic_core::replication::registry_hash() {
        print!("{byte:02x}");
    }
    println!();
}
