//! Read-only beat folder inspection. This module never creates, moves, or deletes files.
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatFolder {
    pub folder: String,
    pub folder_name: String,
    pub project_file: Option<String>,
    pub project_count: usize,
    pub state: &'static str,
    pub detail: String,
}

fn inspect_folder(path: &Path) -> BeatFolder {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let mut item = BeatFolder {
        folder: path.to_string_lossy().to_string(),
        folder_name: name,
        project_file: None,
        project_count: 0,
        state: "invalid",
        detail: String::new(),
    };

    // Do not follow symbolic links / junctions in the initial scanner.
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            item.detail = "Select a regular beat folder, not a shortcut or file".into();
            return item;
        }
        Err(_) => {
            item.detail = "Folder not found or access denied".into();
            return item;
        }
        _ => {}
    }

    let mut projects = Vec::<PathBuf>::new();
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => {
            item.detail = "Unable to read this folder".into();
            return item;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            item.detail = "Unable to read all folder entries".into();
            return item;
        };
        let p = entry.path();
        if !p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("flp")) {
            continue;
        }
        if let Ok(meta) = fs::symlink_metadata(&p) {
            if meta.is_file() && !meta.file_type().is_symlink() {
                projects.push(p);
            }
        }
    }
    projects.sort();
    item.project_count = projects.len();
    match projects.len() {
        0 => {
            item.state = "missing";
            item.detail = "No .flp in this folder's top level".into();
        }
        1 => {
            item.state = "ready";
            item.project_file = projects[0].file_name().map(|x| x.to_string_lossy().to_string());
            item.detail = "Project found · BPM/Key analysis coming next".into();
        }
        _ => {
            item.state = "multiple";
            item.detail = "Multiple .flp files; select a folder containing exactly one".into();
        }
    }
    item
}

#[tauri::command]
pub fn scan_beat_folders(paths: Vec<String>) -> Vec<BeatFolder> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .take(250)
        .filter(|p| seen.insert(p.to_lowercase()))
        .map(|p| inspect_folder(Path::new(&p)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn scan_detects_one_flp_without_touching_it() {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("onece-scanner-{}-{}", std::process::id(), n));
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("beat.flp"), b"untouched").unwrap();
        let item = inspect_folder(&folder);
        assert_eq!(item.state, "ready");
        assert_eq!(item.project_count, 1);
        assert_eq!(fs::read(folder.join("beat.flp")).unwrap(), b"untouched");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn scan_reports_missing_folder() {
        let item = inspect_folder(Path::new("this-path-should-not-exist-onece-2a17"));
        assert_eq!(item.state, "invalid");
    }
}
