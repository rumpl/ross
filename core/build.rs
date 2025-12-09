fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile_protos(
        &[
            "../proto/ross.proto",
            "../proto/image.proto",
            "../proto/container.proto",
        ],
        &["../proto"],
    )?;
    Ok(())
}
