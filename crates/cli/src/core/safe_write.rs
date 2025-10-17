use std::path::{Path, PathBuf};

use anyhow::Error;
use devit_common::fs::SafeFileWriter as CommonSafeFileWriter;
pub use devit_common::fs::WriteMode;
use uuid::Uuid;

use crate::core::errors::{DevItError, DevItResult};

pub struct SafeFileWriter {
    inner: CommonSafeFileWriter,
}

impl SafeFileWriter {
    pub fn new() -> DevItResult<Self> {
        let inner = CommonSafeFileWriter::new().map_err(convert_error)?;
        Ok(Self { inner })
    }

    pub fn with_allowed_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.inner = self.inner.with_allowed_dirs(dirs);
        self
    }

    pub fn with_max_size(mut self, max_size: Option<usize>) -> Self {
        self.inner = self.inner.with_max_size(max_size);
        self
    }

    pub fn write(&self, path: &Path, content: &[u8], mode: WriteMode) -> DevItResult<()> {
        self.inner.write(path, content, mode).map_err(convert_error)
    }

    pub fn write_text(&self, path: &Path, content: &str, mode: WriteMode) -> DevItResult<()> {
        self.inner
            .write_text(path, content, mode)
            .map_err(convert_error)
    }
}

#[inline]
fn convert_error(err: Error) -> DevItError {
    match err.downcast::<std::io::Error>() {
        Ok(io_err) => DevItError::io(None, "safe_file_writer", io_err),
        Err(err) => DevItError::Internal {
            component: "safe_file_writer".to_string(),
            message: err.to_string(),
            cause: None,
            correlation_id: Uuid::new_v4().to_string(),
        },
    }
}
