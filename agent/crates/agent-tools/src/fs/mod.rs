pub mod paths;
pub mod read;
pub mod write;
pub use paths::resolve_in_workspace;
pub use read::{ListDirectory, ReadFile};
pub use write::{EditFile, WriteFile};

use crate::backend::FsError;
use crate::ToolError;

/// FsError → ToolError in one place (spec §5.1). Wave-1 parity: NotUtf8 maps
/// to NotFound (same message today's read_to_string path produced); Task 6
/// flips it to the honest error alongside the paging contract.
pub(crate) fn fs_err(e: FsError) -> ToolError {
    match e {
        FsError::NotFound(m) => ToolError::NotFound(m),
        FsError::Denied(m) => ToolError::Denied(m),
        FsError::NotUtf8(m) => ToolError::NotFound(m),
        FsError::EditConflict(m) | FsError::Io(m) => ToolError::Failed {
            message: m,
            stderr: None,
        },
        FsError::InvalidPath(m) => ToolError::InvalidArgs(m),
    }
}
