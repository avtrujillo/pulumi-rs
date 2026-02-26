fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .compile_protos(
            &[
                "proto/pulumi/resource.proto",
                "proto/pulumi/engine.proto",
                "proto/pulumi/provider.proto",
                "proto/pulumi/callback.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
