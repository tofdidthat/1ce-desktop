//! Explicit, Windows-only, no-replace renames. No rendering or FLP byte edits.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

#[derive(Default)]
pub struct RenameState(Mutex<HashMap<String, Plan>>);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Entry {
    name: String,
    len: u64,
    modified: u128,
    directory: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    id: String,
    old_folder: PathBuf,
    new_folder: PathBuf,
    old_file: String,
    new_file: String,
    #[serde(skip)]
    entries: Vec<Entry>,
}

// Disk records need the snapshot; IPC deliberately omits it.
#[derive(Serialize, Deserialize)]
struct Record {
    plan: Plan,
    entries: Vec<Entry>,
}

fn id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn regular(path: &Path) -> Result<fs::Metadata, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(
                "Links, junctions and cloud placeholders are not supported for rename".into(),
            );
        }
    }
    if meta.file_type().is_symlink() {
        return Err("Symbolic links are not supported".into());
    }
    Ok(meta)
}

fn checked_folder(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.parent().is_none() {
        return Err("Select an absolute project folder".into());
    }
    for ancestor in path.ancestors() {
        regular(ancestor)?;
    }
    if !regular(path)?.is_dir() {
        return Err("Project folder is not a directory".into());
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}

pub(crate) fn valid_name(name: &str) -> Result<(), String> {
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p).is_some_and(|s| {
                ["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"].contains(&s)
            })
        });
    if name.is_empty()
        || name.encode_utf16().count() > 180
        || name.trim() != name
        || name.ends_with('.')
        || name == "."
        || name == ".."
        || reserved
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err("The proposed name is not a safe Windows filename".into());
    }
    Ok(())
}

fn snapshot(folder: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for item in fs::read_dir(folder).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let meta = regular(&item.path())?;
        entries.push(Entry {
            name: item
                .file_name()
                .into_string()
                .map_err(|_| "Unsupported filename")?,
            len: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta
                .modified()
                .map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos(),
            directory: meta.is_dir(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(format!(
            "Destination already exists or cannot be checked: {}. Nothing will be overwritten.",
            path.display()
        )),
    }
}

#[cfg(test)]
fn prepare(
    folder: &str,
    expected_file: &str,
    proposed: &str,
    template: &str,
) -> Result<Plan, String> {
    prepare_name(folder, expected_file, proposed, template, false)
}

fn prepare_name(
    folder: &str,
    expected_file: &str,
    proposed: &str,
    template: &str,
    manual: bool,
) -> Result<Plan, String> {
    valid_name(proposed)?;
    let old_folder = checked_folder(Path::new(folder))?;
    let scanned = crate::scanner::inspect_folder(&old_folder, template);
    if scanned.project_file.as_deref() != Some(expected_file)
        || (!manual && scanned.name_preview.as_deref() != Some(proposed))
    {
        return Err(
            "The selected FLP or suggested name changed. Scan the folder again before renaming."
                .into(),
        );
    }
    let new_folder = old_folder
        .parent()
        .ok_or("Cannot rename a root folder")?
        .join(proposed);
    let new_file = format!("{proposed}.flp");
    if old_folder == new_folder && expected_file == new_file {
        return Err("Folder and FLP already have this name".into());
    }
    if old_folder != new_folder {
        absent(&new_folder)?;
    }
    if expected_file != new_file {
        absent(&old_folder.join(&new_file))?;
    }
    Ok(Plan {
        id: id(),
        entries: snapshot(&old_folder)?,
        old_folder,
        new_folder,
        old_file: expected_file.into(),
        new_file,
    })
}

#[cfg(windows)]
pub(crate) fn move_no_replace(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileW(old: *const u16, new: *const u16) -> i32;
    }
    let old: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let new: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Unlike std::fs::rename, MoveFileW never replaces an existing file.
    if unsafe { MoveFileW(old.as_ptr(), new.as_ptr()) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
pub(crate) fn move_no_replace(_: &Path, _: &Path) -> Result<(), String> {
    Err("Rename is available on Windows only".into())
}

fn mark(log: &Path, status: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .append(true)
        .open(log)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{status}").map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

fn execute(plan: &Plan, logs: &Path) -> Result<String, String> {
    checked_folder(&plan.old_folder)?;
    if snapshot(&plan.old_folder)? != plan.entries {
        return Err("Folder changed after review. Review it again; no files were renamed.".into());
    }
    if plan.old_folder != plan.new_folder {
        absent(&plan.new_folder)?;
    }
    if plan.old_file != plan.new_file {
        absent(&plan.old_folder.join(&plan.new_file))?;
    }
    fs::create_dir_all(logs).map_err(|e| e.to_string())?;
    let log = logs.join(format!("{}.jsonl", plan.id));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(
        &mut file,
        &Record {
            plan: plan.clone(),
            entries: plan.entries.clone(),
        },
    )
    .map_err(|e| e.to_string())?;
    writeln!(file).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    if plan.old_folder != plan.new_folder {
        if let Err(error) = move_no_replace(&plan.old_folder, &plan.new_folder) {
            let _ = mark(&log, "rolled-back");
            return Err(format!("Folder could not be renamed: {error}. Close FL Studio or other apps using it and review again."));
        }
    }
    if plan.old_file != plan.new_file {
        if let Err(error) = move_no_replace(
            &plan.new_folder.join(&plan.old_file),
            &plan.new_folder.join(&plan.new_file),
        ) {
            let rollback = plan.old_folder == plan.new_folder
                || move_no_replace(&plan.new_folder, &plan.old_folder).is_ok();
            if rollback {
                let _ = mark(&log, "rolled-back");
            }
            return Err(format!(
                "FLP rename failed: {error}. {} Recovery record: {}",
                if rollback {
                    "Original names restored."
                } else {
                    "Folder name changed; use Recover rename before continuing."
                },
                log.display()
            ));
        }
    }
    mark(&log, "completed").map_err(|e| format!("Names changed, but completion could not be recorded ({e}). Use Recover rename before continuing."))?;
    Ok(plan.new_folder.to_string_lossy().into_owned())
}

fn pending(logs: &Path) -> Result<Vec<(PathBuf, Plan)>, String> {
    if !logs.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for item in fs::read_dir(logs).map_err(|e| e.to_string())? {
        let path = item.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if matches!(text.lines().last(), Some("completed" | "rolled-back")) {
            continue;
        }
        let mut record: Record = serde_json::from_str(text.lines().next().unwrap_or("")).map_err(|e| format!("Unreadable recovery record {}: {e}. No new renames allowed; retain this file for recovery.", path.display()))?;
        record.plan.entries = record.entries;
        result.push((path, record.plan));
    }
    Ok(result)
}

// Explicit user recovery restores the original names only when the saved
// snapshot still matches. Changed/missing files require manual review.
fn recover(plan: &Plan, log: &Path) -> Result<String, String> {
    if plan.old_folder != plan.new_folder && plan.old_folder.exists() && plan.new_folder.exists() {
        return Err("Both old and new folders exist. Recovery will not overwrite or guess which folder to move; review them manually.".into());
    }
    let folder = if plan.old_folder.exists() {
        &plan.old_folder
    } else {
        &plan.new_folder
    };
    checked_folder(folder)?;
    let actual = snapshot(folder)?;
    let mut renamed = plan.entries.clone();
    for entry in &mut renamed {
        if entry.name == plan.old_file {
            entry.name = plan.new_file.clone();
        }
    }
    renamed.sort_by(|a, b| a.name.cmp(&b.name));
    if actual == renamed && plan.old_file != plan.new_file {
        absent(&folder.join(&plan.old_file))?;
        move_no_replace(&folder.join(&plan.new_file), &folder.join(&plan.old_file))?;
    } else if actual != plan.entries {
        return Err("Files changed since the interrupted rename. Automatic recovery refused; retain the recovery record and review the folder manually.".into());
    }
    if folder != &plan.old_folder {
        absent(&plan.old_folder)?;
        move_no_replace(folder, &plan.old_folder)?;
    }
    mark(log, "rolled-back")?;
    Ok(plan.old_folder.to_string_lossy().into_owned())
}

fn logs(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("rename-records"))
}

#[tauri::command]
pub fn prepare_smart_rename(
    app: tauri::AppHandle,
    state: tauri::State<RenameState>,
    folder: String,
    expected_file: String,
    proposed: String,
    template: String,
    manual: Option<bool>,
) -> Result<Plan, String> {
    let mut plans = state.0.lock().map_err(|e| e.to_string())?;
    if !pending(&logs(&app)?)?.is_empty() {
        return Err("Recover the interrupted rename before starting another".into());
    }
    let plan = prepare_name(
        &folder,
        &expected_file,
        &proposed,
        &template,
        manual.unwrap_or(false),
    )?;
    plans.clear(); // Only one outstanding review; no unbounded token accumulation.
    plans.insert(plan.id.clone(), plan.clone());
    Ok(plan)
}

#[tauri::command]
pub fn execute_smart_rename(
    app: tauri::AppHandle,
    state: tauri::State<RenameState>,
    id: String,
) -> Result<String, String> {
    let mut plans = state.0.lock().map_err(|e| e.to_string())?;
    let plan = plans
        .remove(&id)
        .ok_or("Review expired. Review the names again.")?;
    let logs = logs(&app)?;
    if !pending(&logs)?.is_empty() {
        return Err("Recover the interrupted rename first".into());
    }
    execute(&plan, &logs)
}

#[tauri::command]
pub fn pending_smart_renames(
    app: tauri::AppHandle,
    state: tauri::State<RenameState>,
) -> Result<Vec<Plan>, String> {
    let _guard = state.0.lock().map_err(|e| e.to_string())?;
    Ok(pending(&logs(&app)?)?.into_iter().map(|(_, p)| p).collect())
}

#[tauri::command]
pub fn recover_smart_rename(
    app: tauri::AppHandle,
    state: tauri::State<RenameState>,
    id: String,
) -> Result<String, String> {
    let _guard = state.0.lock().map_err(|e| e.to_string())?;
    let (log, plan) = pending(&logs(&app)?)?
        .into_iter()
        .find(|(_, p)| p.id == id)
        .ok_or("Recovery record not found")?;
    recover(&plan, &log)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("onece-rename-{}", id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn plan(&self) -> Plan {
            let folder = self.0.join("Beat");
            fs::create_dir(&folder).unwrap();
            fs::write(folder.join("selected.flp"), b"selected FLP bytes").unwrap();
            fs::write(folder.join("older.flp"), b"older FLP bytes").unwrap();
            fs::create_dir(folder.join("stems")).unwrap();
            fs::write(folder.join("stems/audio.wav"), b"audio").unwrap();
            Plan {
                id: id(),
                entries: snapshot(&folder).unwrap(),
                old_folder: folder,
                new_folder: self.0.join("Beat 140"),
                old_file: "selected.flp".into(),
                new_file: "Beat 140.flp".into(),
            }
        }
        fn log(&self, plan: &Plan) -> PathBuf {
            let logs = self.0.join("logs");
            fs::create_dir_all(&logs).unwrap();
            let path = logs.join(format!("{}.jsonl", plan.id));
            let text = serde_json::to_string(&Record {
                plan: plan.clone(),
                entries: plan.entries.clone(),
            })
            .unwrap();
            fs::write(&path, format!("{text}\n")).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn renames_only_selected_flp_and_folder() {
        let f = Fixture::new();
        let p = f.plan();
        execute(&p, &f.0.join("logs")).unwrap();
        assert!(!p.old_folder.exists());
        assert_eq!(
            fs::read(p.new_folder.join(&p.new_file)).unwrap(),
            b"selected FLP bytes"
        );
        assert_eq!(
            fs::read(p.new_folder.join("older.flp")).unwrap(),
            b"older FLP bytes"
        );
        assert_eq!(
            fs::read(p.new_folder.join("stems/audio.wav")).unwrap(),
            b"audio"
        );
        assert!(pending(&f.0.join("logs")).unwrap().is_empty());
    }
    #[test]
    fn refuses_collisions_and_changes_after_review() {
        let f = Fixture::new();
        let p = f.plan();
        fs::create_dir(&p.new_folder).unwrap();
        assert!(execute(&p, &f.0.join("logs")).is_err());
        assert!(p.old_folder.join(&p.old_file).exists());
        fs::remove_dir(&p.new_folder).unwrap();
        fs::write(p.old_folder.join(&p.old_file), b"changed").unwrap();
        assert!(execute(&p, &f.0.join("logs"))
            .unwrap_err()
            .contains("changed"));
    }
    #[test]
    fn move_never_overwrites_existing_file() {
        let f = Fixture::new();
        let a = f.0.join("a");
        let b = f.0.join("b");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        assert!(move_no_replace(&a, &b).is_err());
        assert_eq!(fs::read(&b).unwrap(), b"b");
        assert!(a.exists());
    }
    #[test]
    fn locked_flp_rolls_back_folder() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let p = f.plan();
        let _lock = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(p.old_folder.join(&p.old_file))
            .unwrap();
        assert!(execute(&p, &f.0.join("logs")).is_err());
        assert!(p.old_folder.join(&p.old_file).exists());
        assert!(!p.new_folder.exists());
    }
    #[test]
    fn recovers_each_crash_boundary_and_rejects_changed_files() {
        for stage in 0..3 {
            let f = Fixture::new();
            let p = f.plan();
            let log = f.log(&p);
            if stage > 0 {
                move_no_replace(&p.old_folder, &p.new_folder).unwrap();
            }
            if stage > 1 {
                move_no_replace(
                    &p.new_folder.join(&p.old_file),
                    &p.new_folder.join(&p.new_file),
                )
                .unwrap();
            }
            let loaded = pending(&f.0.join("logs")).unwrap().pop().unwrap().1;
            recover(&loaded, &log).unwrap();
            assert!(p.old_folder.join(&p.old_file).exists());
            assert!(pending(&f.0.join("logs")).unwrap().is_empty());
        }
        let f = Fixture::new();
        let p = f.plan();
        let log = f.log(&p);
        move_no_replace(&p.old_folder, &p.new_folder).unwrap();
        fs::write(p.new_folder.join(&p.old_file), b"edited").unwrap();
        assert!(recover(&p, &log).is_err());
        assert_eq!(fs::read(p.new_folder.join(&p.old_file)).unwrap(), b"edited");
    }
    #[test]
    fn rejects_unsafe_names() {
        for name in [
            "..", "CON", "aux.flp", "COM1", "LPT²", "name.", "name ", "a/b", "a\\b", "", "a:b",
        ] {
            assert!(valid_name(name).is_err(), "{name}");
        }
        assert!(valid_name("Beat 140 F#minor @producer").is_ok());
    }
    #[test]
    fn reviewed_scan_round_trip_uses_newest_flp_and_canonical_paths() {
        let f = Fixture::new();
        let p = f.plan();
        let time = UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        OpenOptions::new()
            .write(true)
            .open(p.old_folder.join("older.flp"))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(time))
            .unwrap();
        let source = p.old_folder.to_str().unwrap();
        assert!(prepare(source, "older.flp", "Beat @test", "{name} @test").is_err());
        let reviewed = prepare(source, "selected.flp", "Beat @test", "{name} @test").unwrap();
        execute(&reviewed, &f.0.join("logs")).unwrap();
        assert!(reviewed.new_folder.join("Beat @test.flp").exists());
        assert_eq!(
            fs::read(reviewed.new_folder.join("older.flp")).unwrap(),
            b"older FLP bytes"
        );
    }
    #[test]
    fn recovery_refuses_recreated_original_folder() {
        let f = Fixture::new();
        let p = f.plan();
        let log = f.log(&p);
        move_no_replace(&p.old_folder, &p.new_folder).unwrap();
        fs::create_dir(&p.old_folder).unwrap();
        assert!(recover(&p, &log).is_err());
        assert!(p.new_folder.join(&p.old_file).exists());
    }
    #[test]
    fn manual_title_uses_same_safe_rename_and_newest_file_checks() {
        let f = Fixture::new();
        let base = f.plan();
        let source = base.old_folder.to_str().unwrap();
        let old_time = UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        OpenOptions::new()
            .write(true)
            .open(base.old_folder.join("older.flp"))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old_time))
            .unwrap();
        assert!(prepare_name(
            source,
            "selected.flp",
            "Custom title 136 @test",
            "{name}",
            false
        )
        .is_err());
        assert!(prepare_name(
            source,
            "older.flp",
            "Custom title 136 @test",
            "{name}",
            true
        )
        .is_err());
        assert!(prepare_name(source, "selected.flp", "../escape", "{name}", true).is_err());
        let plan = prepare_name(
            source,
            "selected.flp",
            "Custom title 136 @test",
            "{name}",
            true,
        )
        .unwrap();
        execute(&plan, &f.0.join("logs")).unwrap();
        assert_eq!(
            fs::read(plan.new_folder.join("Custom title 136 @test.flp")).unwrap(),
            b"selected FLP bytes"
        );
        assert!(plan.new_folder.join("older.flp").exists());
    }
}
