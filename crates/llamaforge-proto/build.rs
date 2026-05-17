fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    // Emit a file descriptor set alongside the generated bindings so
    // `tonic-reflection` can serve discovery in dev builds. The
    // transport crate gates the actual reflection-service wiring
    // behind its `dev-reflection` feature.
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path(out_dir.join("llamaforge_descriptor.bin"))
        .compile_protos(
            &["../../proto/llamaforge/inference.proto", "../../proto/llamaforge/management.proto"],
            &["../../proto"],
        )?;
    Ok(())
}
