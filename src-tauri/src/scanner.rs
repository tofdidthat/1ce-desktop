//! Read-only beat folder inspection. This module never creates, moves, or deletes files.
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use crate::{flp_metadata, naming};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatFolder {
    pub folder: String,
    pub folder_name: String,
    pub project_file: Option<String>,
    pub project_count: usize,
    pub state: &'static str,
    pub detail: String,
    pub bpm: Option<f64>,
    pub bpm_source: Option<String>,
    pub bpm_warning: Option<String>,
    pub name_preview: Option<String>,
    pub missing_key: bool,
    pub needs_tempo_review: bool,
}

fn inspect_folder(path: &Path, template: &str) -> BeatFolder {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let mut item = BeatFolder {
        folder: path.to_string_lossy().to_string(),
        folder_name: name,
        project_file: None,
        project_count: 0,
        state: "invalid",
        detail: String::new(),
        bpm: None,
        bpm_source: None,
        bpm_warning: None,
        name_preview: None,
        missing_key: false,
        needs_tempo_review: false,
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
            item.detail = "Project found · read-only BPM and naming preview".into();
            match flp_metadata::read_flp_tempo(&projects[0]) {
                Ok(metadata) => {
                    item.bpm = Some(metadata.bpm);
                    item.bpm_source = Some(metadata.source.into());
                    item.needs_tempo_review = metadata.needs_tempo_review;
                    if metadata.needs_tempo_review {
                        item.bpm_warning = Some("Multiple tempo-setting events; review in FL Studio".into());
                    }
                }
                Err(e) => { item.bpm_warning = Some(format!("Cannot read base BPM: {e}")); }
            }
            match naming::preview_title(&item.folder_name, item.bpm, None, template) {
                Ok(preview) => {
                    item.name_preview = Some(preview.proposed);
                    item.missing_key = preview.missing_key;
                    if preview.bpm_conflict {
                        item.needs_tempo_review = true;
                        item.bpm_warning = Some("BPM in title differs from FLP; manual review needed".into());
                    }
                }
                Err(error) => {
                    item.bpm_warning = Some(format!("Naming preview unavailable: {error}"));
                }
            }
        }
        _ => {
            item.state = "multiple";
            item.detail = "Multiple .flp files; select a folder containing exactly one".into();
        }
    }
    item
}

#[tauri::command]
pub fn scan_beat_folders(paths: Vec<String>, template: Option<String>) -> Vec<BeatFolder> {
    let template = template.as_deref().unwrap_or(naming::DEFAULT_TEMPLATE);
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .take(250)
        .filter(|p| seen.insert(p.to_lowercase()))
        .map(|p| inspect_folder(Path::new(&p), template))
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
        let item = inspect_folder(&folder, naming::DEFAULT_TEMPLATE);
        assert_eq!(item.state, "ready");
        assert_eq!(item.project_count, 1);
        assert!(item.bpm.is_none());
        assert_eq!(fs::read(folder.join("beat.flp")).unwrap(), b"untouched");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn scan_reads_valid_flp_bpm_and_previews_name_without_writes() {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("onece-bpm-{}-{} 145 @producer", std::process::id(), n));
        fs::create_dir(&folder).unwrap();
        let project = folder.join("beat.flp");
        let mut data = b"FLhd".to_vec();
        data.extend(6u32.to_le_bytes());
        data.extend([0,0,1,0,96,0]);
        data.extend(b"FLdt");
        data.extend(5u32.to_le_bytes());
        data.push(156);
        data.extend(145_000u32.to_le_bytes());
        fs::write(&project, &data).unwrap();
        let item = inspect_folder(&folder, naming::DEFAULT_TEMPLATE);
        assert_eq!(item.bpm, Some(145.0));
        assert!(item.name_preview.as_ref().is_some_and(|n| n.contains("145")));
        assert_eq!(fs::read(&project).unwrap(), data);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn scan_reports_missing_folder() {
        let item = inspect_folder(Path::new("this-path-should-not-exist-onece-2a17"), naming::DEFAULT_TEMPLATE);
        assert_eq!(item.state, "invalid");
    }
}
