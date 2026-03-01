use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let proto_root = PathBuf::from("../../proto");
    let files = [
        proto_root.join("anki/api/v1/common.proto"),
        proto_root.join("anki/api/v1/health.proto"),
        proto_root.join("anki/api/v1/system.proto"),
        proto_root.join("anki/api/v1/notes.proto"),
        proto_root.join("anki/api/v1/notetypes.proto"),
    ];

    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    let proto_files = files.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    let include_paths = [proto_root.as_path()];
    tonic_prost_build::configure().compile_protos(&proto_files, &include_paths)?;

    Ok(())
}
