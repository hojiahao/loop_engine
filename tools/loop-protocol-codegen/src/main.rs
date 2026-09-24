use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use prost::Message;
use prost_types::FileDescriptorSet;

fn main() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let descriptor_path = arguments
        .next()
        .map(PathBuf::from)
        .context("usage: loop-protocol-codegen <descriptor.binpb> <output-directory>")?;
    let output_directory = arguments
        .next()
        .map(PathBuf::from)
        .context("usage: loop-protocol-codegen <descriptor.binpb> <output-directory>")?;
    if arguments.next().is_some() {
        bail!("usage: loop-protocol-codegen <descriptor.binpb> <output-directory>");
    }

    let descriptor_bytes = fs::read(&descriptor_path)
        .with_context(|| format!("failed to read {}", descriptor_path.display()))?;
    let descriptor = FileDescriptorSet::decode(descriptor_bytes.as_slice())
        .with_context(|| format!("failed to decode {}", descriptor_path.display()))?;
    fs::create_dir_all(&output_directory)
        .with_context(|| format!("failed to create {}", output_directory.display()))?;

    tonic_prost_build::configure()
        .out_dir(&output_directory)
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(false)
        .compile_fds(descriptor)
        .context("failed to generate Rust protocol bindings")?;

    Ok(())
}
