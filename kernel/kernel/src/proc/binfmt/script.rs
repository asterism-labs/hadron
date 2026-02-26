//! Shebang (`#!`) script handler — recognised but not yet implemented.

use super::{BinaryError, BinaryFormat, ExecImage};

/// Singleton handler for `#!` scripts.
pub struct ScriptHandler;

impl BinaryFormat for ScriptHandler {
    fn name(&self) -> &'static str {
        "script"
    }

    fn probe(&self, data: &[u8]) -> bool {
        data.len() >= 2 && data[..2] == *b"#!"
    }

    fn load<'a>(&self, _data: &'a [u8]) -> Result<ExecImage<'a>, BinaryError> {
        Err(BinaryError::Unimplemented("script/shebang (#!)"))
    }
}
