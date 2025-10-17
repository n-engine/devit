use std::fs;
use std::fs::Metadata;
use std::path::Path;

/// Platform-specific file permission snapshot and operations.
///
/// Note: This is an internal abstraction. Public APIs remain unchanged.
pub struct PlatformPermissions {
    #[cfg(unix)]
    pub mode: u32,

    #[cfg(windows)]
    pub readonly: bool,
    #[cfg(windows)]
    pub executable: bool, // heuristic based on extension
}

impl PlatformPermissions {
    /// Construct from filesystem metadata only.
    /// On Windows this does not infer executability (use `from_fs`).
    pub fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Self {
                mode: metadata.permissions().mode(),
            }
        }

        #[cfg(windows)]
        {
            Self {
                readonly: metadata.permissions().readonly(),
                executable: false, // unknown without path; prefer `from_fs`
            }
        }
    }

    /// Construct using both metadata and path (preferred on Windows).
    pub fn from_fs(path: &Path, metadata: &Metadata) -> Self {
        #[cfg(unix)]
        let _ = path;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Self {
                mode: metadata.permissions().mode(),
            }
        }

        #[cfg(windows)]
        {
            let readonly = metadata.permissions().readonly();
            let executable = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "exe" | "bat" | "cmd" | "com"
                    )
                })
                .unwrap_or(false);
            Self {
                readonly,
                executable,
            }
        }
    }

    /// Apply the stored permissions to a path.
    pub fn apply(&self, path: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perm = fs::Permissions::from_mode(self.mode);
            fs::set_permissions(path, perm)
        }

        #[cfg(windows)]
        {
            let mut perm = fs::metadata(path)?.permissions();
            perm.set_readonly(self.readonly);
            // Executability is determined by extension on Windows; no-op here.
            fs::set_permissions(path, perm)
        }
    }

    /// Compare two platform permission snapshots for meaningful change.
    pub fn has_changed(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            self.mode != other.mode
        }

        #[cfg(windows)]
        {
            self.readonly != other.readonly || self.executable != other.executable
        }
    }

    /// Encode to a u32 for storage in SnapshotFile without changing public API.
    pub fn encode(&self) -> u32 {
        #[cfg(unix)]
        {
            self.mode
        }

        #[cfg(windows)]
        {
            // Tag highest bit to indicate Windows-encoded flags.
            const TAG: u32 = 0x8000_0000;
            let mut bits: u32 = 0;
            if self.readonly {
                bits |= 0x1;
            }
            if self.executable {
                bits |= 0x2;
            }
            TAG | bits
        }
    }

    /// Decode from stored u32. Returns None if not decodable on this platform.
    pub fn decode(encoded: u32) -> Option<Self> {
        #[cfg(unix)]
        {
            Some(Self { mode: encoded })
        }

        #[cfg(windows)]
        {
            const TAG: u32 = 0x8000_0000;
            if encoded & TAG == 0 {
                // Snapshot likely produced on Unix; treat as unknown on Windows.
                return None;
            }
            let readonly = (encoded & 0x1) != 0;
            let executable = (encoded & 0x2) != 0;
            Some(Self {
                readonly,
                executable,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[cfg(windows)]
    #[test]
    fn windows_encode_decode_roundtrip() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "hello").unwrap();
        let path = tmp.path().with_extension("exe");
        fs::rename(tmp.path(), &path).unwrap();
        let meta = fs::metadata(&path).unwrap();
        let pp = PlatformPermissions::from_fs(&path, &meta);
        assert!(pp.executable, "exe heuristic must be true");
        let enc = pp.encode();
        let dec = PlatformPermissions::decode(enc).expect("decodable on Windows");
        assert_eq!(pp.readonly, dec.readonly);
        assert_eq!(pp.executable, dec.executable);
    }

    #[cfg(unix)]
    #[test]
    fn unix_encode_decode_roundtrip() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "hello").unwrap();
        let meta = fs::metadata(tmp.path()).unwrap();
        let pp = PlatformPermissions::from_fs(tmp.path(), &meta);
        let enc = pp.encode();
        let dec = PlatformPermissions::decode(enc).expect("decodable on Unix");
        assert_eq!(pp.mode, dec.mode);
    }
}
