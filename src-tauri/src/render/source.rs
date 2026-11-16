//! Render a neutral-named copy, retaining project-relative assets and originals.
use super::*;
use std::io::{Read, Write};

pub(super) fn working_copy(
    project: &Path,
    attempt: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let root = regular_path(project.parent().ok_or("Project folder missing")?, true)?;
    let project = regular_path(project, false)?;
    let destination = attempt.join("render-input");
    fs::create_dir(&destination).map_err(|e| e.to_string())?;
    let neutral = format!("1ce-input-{}.flp", attempt_id());
    let output = destination.join(&neutral);
    let mut entries = 0;
    fn walk(
        root: &Path,
        dest: &Path,
        project: &Path,
        neutral: &str,
        depth: usize,
        entries: &mut usize,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        if depth > 32 {
            return Err("Project exceeds 32 working-copy levels".into());
        }
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            integrity::check(cancel)?;
            *entries += 1;
            if *entries > 10000 {
                return Err("Project exceeds 10000 working-copy entries".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            let path = regular_path(&entry.path(), metadata.is_dir())?;
            let target = if path == project {
                dest.join(neutral)
            } else {
                dest.join(entry.file_name())
            };
            if metadata.is_dir() {
                fs::create_dir(&target).map_err(|e| e.to_string())?;
                walk(&path, &target, project, neutral, depth + 1, entries, cancel)?;
            } else {
                let expected = integrity::fingerprint(&path, cancel)?;
                let mut input = integrity::read_locked(&path)?;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(|e| e.to_string())?;
                let mut buffer = [0u8; 65536];
                loop {
                    integrity::check(cancel)?;
                    let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                }
                output.sync_all().map_err(|e| e.to_string())?;
                drop(output);
                drop(input);
                integrity::matches(&path, &expected, cancel)?;
                integrity::matches(&target, &expected, cancel)?;
            }
        }
        Ok(())
    }
    walk(
        &root,
        &destination,
        &project,
        &neutral,
        0,
        &mut entries,
        cancel,
    )?;
    let manifest = snapshot(&destination, cancel)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(attempt.join("working-copy.json"))
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(output)
}

// A manifest is emitted only after the complete private working copy verifies.
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    fingerprint: Option<integrity::Fingerprint>,
}
fn snapshot(root: &Path, cancel: &AtomicBool) -> Result<Vec<Entry>, String> {
    fn walk(
        root: &Path,
        rel: &Path,
        out: &mut Vec<Entry>,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        if rel.components().count() > 32 {
            return Err("Working copy too deep".into());
        }
        for entry in fs::read_dir(root.join(rel)).map_err(|e| e.to_string())? {
            integrity::check(cancel)?;
            if out.len() >= 10000 {
                return Err("Working copy too large".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let path = rel.join(entry.file_name());
            let directory = entry.file_type().map_err(|e| e.to_string())?.is_dir();
            regular_path(&entry.path(), directory)?;
            out.push(Entry {
                path: path.clone(),
                fingerprint: if directory {
                    None
                } else {
                    Some(integrity::fingerprint(&entry.path(), cancel)?)
                },
            });
            if directory {
                walk(root, &path, out, cancel)?;
            }
        }
        Ok(())
    }
    regular_path(root, true)?;
    let mut entries = Vec::new();
    walk(root, Path::new(""), &mut entries, cancel)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}
pub(super) fn cleanup_working_copy(attempt: &Path, cancel: &AtomicBool) -> Result<(), String> {
    // Caller first verifies final delivery and source-move completion. Never sweep
    // other attempts, old music folders, diagnostics, or unmanifested copies.
    let root = attempt.join("render-input");
    if !root.exists() {
        return Ok(());
    }
    let file = integrity::read_locked(&attempt.join("working-copy.json"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("Working copy manifest too large".into());
    }
    let expected: Vec<Entry> = serde_json::from_reader(file).map_err(|e| e.to_string())?;
    let actual = snapshot(&root, cancel)?;
    // Missing entries can be an interrupted cleanup; extras/edits preserve everything.
    if actual.iter().any(|e| !expected.contains(e)) {
        return Err("Private working copy changed; retained for inspection".into());
    }
    for entry in actual.iter().filter(|e| e.fingerprint.is_some()) {
        integrity::check(cancel)?;
        integrity::matches(
            &root.join(&entry.path),
            entry.fingerprint.as_ref().unwrap(),
            cancel,
        )?;
        fs::remove_file(root.join(&entry.path)).map_err(|e| e.to_string())?;
    }
    let mut dirs: Vec<_> = actual.iter().filter(|e| e.fingerprint.is_none()).collect();
    dirs.sort_by_key(|e| std::cmp::Reverse(e.path.components().count()));
    for entry in dirs {
        integrity::check(cancel)?;
        regular_path(&root.join(&entry.path), true)?;
        fs::remove_dir(root.join(&entry.path)).map_err(|e| e.to_string())?;
    }
    fs::remove_dir(root).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_preserves_untracked_changed_and_cancelled_copies() {
        for scenario in 0..4 {
            let root = std::env::temp_dir().join(format!("1ce-copy-cleanup-{}", attempt_id()));
            fs::create_dir(&root).unwrap();
            let original = root.join("original");
            fs::create_dir(&original).unwrap();
            let project = original.join("beat.flp");
            fs::write(&project, b"original").unwrap();
            let attempt = root.join("attempt");
            fs::create_dir(&attempt).unwrap();
            let copy = working_copy(&project, &attempt, &AtomicBool::new(false)).unwrap();
            if scenario == 0 {
                fs::write(copy.parent().unwrap().join("new.txt"), b"user data").unwrap();
            }
            if scenario == 1 {
                fs::write(&copy, b"edited").unwrap();
            }
            if scenario == 2 {
                fs::remove_file(attempt.join("working-copy.json")).unwrap();
            }
            assert!(cleanup_working_copy(&attempt, &AtomicBool::new(scenario == 3)).is_err());
            assert!(copy.exists());
            assert_eq!(fs::read(&project).unwrap(), b"original");
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn cleanup_resumes_missing_files_and_keeps_diagnostics() {
        let root = std::env::temp_dir().join(format!("1ce-copy-cleanup-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let original = root.join("original");
        fs::create_dir(&original).unwrap();
        let project = original.join("beat.flp");
        fs::write(&project, b"original").unwrap();
        let attempt = root.join("attempt");
        fs::create_dir(&attempt).unwrap();
        let copy = working_copy(&project, &attempt, &AtomicBool::new(false)).unwrap();
        fs::remove_file(copy).unwrap();
        fs::write(attempt.join("diagnostic.json"), b"keep").unwrap();
        cleanup_working_copy(&attempt, &AtomicBool::new(false)).unwrap();
        assert!(!attempt.join("render-input").exists());
        assert!(attempt.join("diagnostic.json").exists());
        cleanup_working_copy(&attempt, &AtomicBool::new(false)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn neutral_copy_preserves_assets_and_original_names() {
        let root = std::env::temp_dir().join(format!("1ce-source-copy-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let original = root.join("HOOD 182 f# @producer");
        fs::create_dir(&original).unwrap();
        let project = original.join("HOOD 182 f# @producer.flp");
        fs::write(&project, b"original FLP").unwrap();
        fs::create_dir(original.join("samples")).unwrap();
        fs::write(original.join("samples/kick.wav"), b"relative sample").unwrap();
        let attempt = root.join("attempt");
        fs::create_dir(&attempt).unwrap();
        let copy = working_copy(&project, &attempt, &AtomicBool::new(false)).unwrap();
        assert!(!copy
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(['#', ' ']));
        assert_eq!(fs::read(&copy).unwrap(), b"original FLP");
        assert_eq!(
            fs::read(copy.parent().unwrap().join("samples/kick.wav")).unwrap(),
            b"relative sample"
        );
        assert_eq!(fs::read(&project).unwrap(), b"original FLP");
        assert!(working_copy(&project, &attempt, &AtomicBool::new(false)).is_err());
        let cancelled = root.join("cancelled");
        fs::create_dir(&cancelled).unwrap();
        assert!(working_copy(&project, &cancelled, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancel"));
        fs::remove_dir_all(root).unwrap();
    }
}
