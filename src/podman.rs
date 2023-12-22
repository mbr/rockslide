use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct Podman {
    /// Path to the podman binary.
    podman_path: PathBuf,
}

impl Podman {
    /// Creates a new podman handle.
    pub(crate) fn new<P: AsRef<Path>>(podman_path: P) -> Self {
        Self {
            podman_path: podman_path.as_ref().into(),
        }
    }
}
