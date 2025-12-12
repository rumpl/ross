//! Build script for ross-shim.
//!
//! This ensures the crate is rebuilt when the guest binary changes.

fn main() {
    // Tell cargo to rerun this build script if the guest binary changes
    println!("cargo::rerun-if-changed=../guest/target/release/ross-init");
}
