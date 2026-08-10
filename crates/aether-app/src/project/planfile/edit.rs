//! The match/replace/create/delete edit engine. Pure text operations only —
//! no knowledge of the journal or the graph. `apply_edit` turns a validated
//! [`crate::project::planfile::schema::Edit`] into a `ProjectWrite` by
//! reading from `base_dir` (either the disposable copy or, for the initial
//! per-step read, the real project root); it never writes anything itself.

use crate::project::planfile::schema::Edit;
use crate::project::source::ProjectWrite;
use std::path::Path;

/// Count non-overlapping, byte-exact occurrences of `needle` in `haystack`.
/// Empty needles never occur (avoids an infinite/degenerate count).
pub(crate) fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(offset) = haystack[start..].find(needle) {
        count += 1;
        start += offset + needle.len();
    }
    count
}

/// Turn one edit into a `ProjectWrite`, re-checking its exact-occurrence
/// contract at apply time (not just precondition time) — fails closed, no
/// fuzzy retry, if the file changed between precondition checking and
/// application.
pub(crate) fn apply_edit(base_dir: &Path, edit: &Edit) -> std::io::Result<ProjectWrite> {
    match edit {
        Edit::Substitute {
            path,
            match_text,
            replace,
            occurrences,
        } => {
            let target = base_dir.join(path);
            let contents = std::fs::read_to_string(&target).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("could not read {path} to apply edit: {error}"),
                )
            })?;
            let found = count_occurrences(&contents, match_text);
            let expected = *occurrences as usize;
            if found != expected {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "{path} expects {expected} occurrence(s) of the given match text, \
                         found {found}; refusing to guess"
                    ),
                ));
            }
            let new_contents = contents.replace(match_text.as_str(), replace);
            Ok(ProjectWrite::text(
                path.clone(),
                Some(contents.into_bytes()),
                new_contents,
            ))
        }
        Edit::Create { path, create } => {
            let target = base_dir.join(path);
            if target.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{path} already exists; create edits never overwrite"),
                ));
            }
            Ok(ProjectWrite::text(path.clone(), None, create.clone()))
        }
        Edit::Delete { path, delete } => {
            if !*delete {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{path}: a delete edit must set \"delete\": true"),
                ));
            }
            let target = base_dir.join(path);
            let contents = std::fs::read(&target).map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("could not read {path} to delete it: {error}"),
                )
            })?;
            Ok(ProjectWrite::delete(path.clone(), contents))
        }
    }
}

/// Physically write a `ProjectWrite` into `dir` (the disposable copy).
/// Real-tree writes always go through `commit_project_writes` instead —
/// this is only for making a copy's on-disk state match what the step
/// would commit, so checks can run against it.
pub(crate) fn write_into(dir: &Path, write: &ProjectWrite) -> std::io::Result<()> {
    let target = dir.join(write.relative());
    if let Some(contents) = write.contents() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, contents)
    } else {
        std::fs::remove_file(&target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_non_overlapping_matches() {
        assert_eq!(count_occurrences("aaaa", "aa"), 2);
        assert_eq!(count_occurrences("abcabcabc", "abc"), 3);
    }

    #[test]
    fn zero_when_absent() {
        assert_eq!(count_occurrences("hello world", "goodbye"), 0);
    }

    #[test]
    fn empty_needle_counts_as_zero_not_infinite() {
        assert_eq!(count_occurrences("hello", ""), 0);
    }

    #[test]
    fn respects_utf8_boundaries() {
        // "café" repeated; needle "é" must match exactly twice, not corrupt
        // byte offsets across the multi-byte character.
        assert_eq!(count_occurrences("café café", "é"), 2);
        assert_eq!(count_occurrences("café café", "caf"), 2);
    }

    #[test]
    fn counts_across_multiline_text() {
        let haystack = "line one\nline two\nline one\n";
        assert_eq!(count_occurrences(haystack, "line one\n"), 2);
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "bitcode-planfile-edit-{name}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn substitute_edit_produces_the_expected_project_write() {
        let dir = TempDir::new("substitute");
        std::fs::write(dir.0.join("a.rs"), "fn old() {}\n").unwrap();
        let edit = Edit::Substitute {
            path: "a.rs".into(),
            match_text: "old".into(),
            replace: "new".into(),
            occurrences: 1,
        };
        let write = apply_edit(&dir.0, &edit).unwrap();
        assert_eq!(write.expected(), Some(b"fn old() {}\n".as_slice()));
        assert_eq!(write.contents(), Some(b"fn new() {}\n".as_slice()));
    }

    #[test]
    fn substitute_edit_fails_closed_on_wrong_occurrence_count_and_does_not_retry() {
        let dir = TempDir::new("substitute-mismatch");
        std::fs::write(dir.0.join("a.rs"), "fn a() {} fn a() {}\n").unwrap();
        let edit = Edit::Substitute {
            path: "a.rs".into(),
            match_text: "fn a()".into(),
            replace: "fn b()".into(),
            occurrences: 1,
        };
        let error = apply_edit(&dir.0, &edit).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("expects 1"), "{error}");
        assert!(error.to_string().contains("found 2"), "{error}");
        // The source file itself is never touched by a failed apply_edit call.
        assert_eq!(
            std::fs::read_to_string(dir.0.join("a.rs")).unwrap(),
            "fn a() {} fn a() {}\n"
        );
    }

    #[test]
    fn create_edit_fails_if_the_path_already_exists() {
        let dir = TempDir::new("create-exists");
        std::fs::write(dir.0.join("a.rs"), "present\n").unwrap();
        let edit = Edit::Create {
            path: "a.rs".into(),
            create: "fn new() {}\n".into(),
        };
        let error = apply_edit(&dir.0, &edit).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn create_edit_produces_a_write_with_no_expected_bytes() {
        let dir = TempDir::new("create-new");
        let edit = Edit::Create {
            path: "new.rs".into(),
            create: "fn new() {}\n".into(),
        };
        let write = apply_edit(&dir.0, &edit).unwrap();
        assert_eq!(write.expected(), None);
        assert_eq!(write.contents(), Some(b"fn new() {}\n".as_slice()));
    }

    #[test]
    fn delete_edit_captures_the_old_bytes() {
        let dir = TempDir::new("delete");
        std::fs::write(dir.0.join("gone.rs"), "bye\n").unwrap();
        let edit = Edit::Delete {
            path: "gone.rs".into(),
            delete: true,
        };
        let write = apply_edit(&dir.0, &edit).unwrap();
        assert_eq!(write.expected(), Some(b"bye\n".as_slice()));
        assert_eq!(write.contents(), None);
    }

    #[test]
    fn write_into_applies_all_three_kinds_to_disk() {
        let dir = TempDir::new("write-into");
        std::fs::write(dir.0.join("keep.rs"), "old\n").unwrap();
        std::fs::write(dir.0.join("gone.rs"), "bye\n").unwrap();

        let substitute = apply_edit(
            &dir.0,
            &Edit::Substitute {
                path: "keep.rs".into(),
                match_text: "old".into(),
                replace: "new".into(),
                occurrences: 1,
            },
        )
        .unwrap();
        let create = apply_edit(
            &dir.0,
            &Edit::Create {
                path: "created.rs".into(),
                create: "fresh\n".into(),
            },
        )
        .unwrap();
        let delete = apply_edit(
            &dir.0,
            &Edit::Delete {
                path: "gone.rs".into(),
                delete: true,
            },
        )
        .unwrap();

        write_into(&dir.0, &substitute).unwrap();
        write_into(&dir.0, &create).unwrap();
        write_into(&dir.0, &delete).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.0.join("keep.rs")).unwrap(),
            "new\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.0.join("created.rs")).unwrap(),
            "fresh\n"
        );
        assert!(!dir.0.join("gone.rs").exists());
    }
}
