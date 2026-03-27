use std::io::Result;

fn main() -> Result<()> {
    tonic_build::compile_protos("../kaonic-commd/proto/kaonic/kaonic.proto")?;
    Ok(())
}
