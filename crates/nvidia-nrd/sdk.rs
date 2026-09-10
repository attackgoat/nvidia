use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub fn sdk_inputs(sdk: &Path) -> io::Result<Vec<PathBuf>> {
    let mut pending = [
        "CMakeLists.txt",
        "Include",
        "Integration",
        "Resources",
        "Source",
        "Shaders",
        "deps",
    ]
    .map(|relative| sdk.join(relative))
    .to_vec();
    let mut inputs = Vec::new();

    while let Some(path) = pending.pop() {
        // CMake rewrites this output on every configure, even when unchanged.
        if path == sdk.join("Shaders/NRDConfig.hlsli") {
            continue;
        }

        if path.is_dir() {
            for entry in fs::read_dir(&path)? {
                let entry = entry?;

                if matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "_Bin" | "_Build" | "_Shaders")
                ) {
                    continue;
                }

                pending.push(entry.path());
            }
        } else {
            // Directory watches recursively include generated files and metadata.
            inputs.push(path);
        }
    }

    inputs.sort();

    Ok(inputs)
}
