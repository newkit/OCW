//! Compiles `proto/` with protox (pure Rust), so no `protoc` binary is needed.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = "../../proto";
    let files = ["ocw/v1/messages.proto"];
    for f in &files {
        println!("cargo:rerun-if-changed={proto_root}/{f}");
    }
    let descriptors = protox::compile(files, [proto_root])?;
    prost_build::Config::new().compile_fds(descriptors)?;
    Ok(())
}
