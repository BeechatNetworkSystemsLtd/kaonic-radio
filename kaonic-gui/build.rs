fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &["../kaonic-commd/proto/kaonic/kaonic.proto"],
            &["../kaonic-commd/proto"],
        )?;

    Ok(())
}
