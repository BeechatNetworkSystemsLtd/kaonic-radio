use std::io::Result;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=proto/");

    // Generate proto files for Kaonic
    tonic_build::configure().compile_protos(
        &[
            "proto/kaonic/kaonic.proto",
            "proto/kaonic/kaonic_factory.proto",
        ],
        &["proto/kaonic"],
    )?;
    Ok(())
}
