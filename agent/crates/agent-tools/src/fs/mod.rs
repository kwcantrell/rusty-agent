pub mod paths;
pub mod read;
pub mod search;
pub mod write;
pub use paths::resolve_in_workspace;
pub use read::{ListDirectory, ReadFile};
pub use search::GrepTool;
pub use write::{EditFile, WriteFile};

use crate::backend::FsError;
use crate::ToolError;

/// FsError → ToolError in one place (spec §5.1). `read_file` intercepts
/// `NotUtf8` itself before reaching this mapping (Task 6: honest binary-file
/// error alongside the paging contract) — the fallback arm below still maps
/// it to `NotFound` for any other caller that hasn't made that switch yet.
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
