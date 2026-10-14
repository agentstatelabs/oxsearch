//! The oxsearch generator and its development tasks. `main.rs` is the
//! command line; this library is what it runs, so each step can be tested.

pub mod graph;
pub mod placement;
pub mod shapes;
pub mod spec;

use std::path::{Path, PathBuf};

/// The error the tasks return: a message for the developer running them.
pub type Error = Box<dyn std::error::Error>;

/// The workspace root, which holds `spec/` and `integration/`.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}
