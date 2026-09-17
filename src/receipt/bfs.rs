use std::collections::VecDeque;
use std::fs::FileType;
use std::path::PathBuf;

/// Directory entry.
#[derive(Debug)]
pub struct DirEntry {
    /// File type.
    pub kind: FileType,
    /// Full file path.
    pub path: PathBuf,
}

/// Traverse file system tree under `path` in breadth-first order.
///
/// Additionally entries within each directory are sorted by file name.
pub fn bfs(path: PathBuf) -> std::io::Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    let root_kind = std::fs::metadata(&path)?.file_type();
    entries.push(DirEntry {
        kind: root_kind,
        path: path.clone(),
    });
    if !root_kind.is_dir() {
        return Ok(entries);
    }
    let mut queue = VecDeque::new();
    queue.push_back(path);
    while let Some(path) = queue.pop_front() {
        let offset = entries.len();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            entries.push(DirEntry {
                kind: entry.file_type()?,
                path: entry.path(),
            });
        }
        entries[offset..].sort_unstable_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
        for entry in entries.iter().skip(offset) {
            if entry.kind.is_dir() {
                queue.push_back(entry.path.clone());
            }
        }
    }
    Ok(entries)
}
