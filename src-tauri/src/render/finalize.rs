//! Verified delivery with no-replace publication, journaled source move and recovery.
use super::integrity::{self, Artifact, Fingerprint};
use super::*;
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    sync::Mutex,
};

const OWNER: &str = ".1ce-owner.json";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewedKey {
    pub root: String,
    pub mode: String,
    pub confirmed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub attempt: PathBuf,
    pub title: String,
    pub template: String,
    pub key: ReviewedKey,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Transfer {
    source: PathBuf,
    relative: PathBuf,
    fingerprint: Fingerprint,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Target {
    kind: String,
    final_path: PathBuf,
    stage: PathBuf,
    directory: bool,
    dirs: Vec<PathBuf>,
    files: Vec<Transfer>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Plan {
    version: u32,
    id: String,
    request: Request,
    name: String,
    project: PathBuf,
    targets: Vec<Target>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: String,
    pub final_name: String,
    pub source_project: PathBuf,
    pub key: ReviewedKey,
    pub destinations: Vec<PathBuf>,
    pub file_count: usize,
    pub bytes: u64,
    pub resumed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub id: String,
    pub final_name: String,
    pub key: ReviewedKey,
    pub destinations: Vec<PathBuf>,
    pub manifest: PathBuf,
    pub upload_ready: bool,
    pub source_action: String,
}
#[derive(Default)]
pub struct DeliveryState(Mutex<HashMap<String, Plan>>);
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioRecord {
    project: PathBuf,
    state: String,
    format: String,
    stems: bool,
    #[serde(default)]
    source_fingerprint: Option<Fingerprint>,
    #[serde(default)]
    artifacts: Vec<Artifact>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = integrity::read_locked(path)?;
    if file.metadata().map_err(err)?.len() > 16 * 1024 * 1024 {
        return Err("Delivery record is too large".into());
    }
    serde_json::from_reader(file).map_err(err)
}
fn exact_record(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = integrity::read_locked(path)?;
    if file.metadata().map_err(err)?.len() != bytes.len() as u64 {
        return Err("Delivery record size differs".into());
    }
    let mut actual = Vec::with_capacity(bytes.len());
    file.read_to_end(&mut actual).map_err(err)?;
    if actual != bytes {
        return Err("Delivery record differs".into());
    }
    Ok(())
}
fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(format!(
            "Destination already exists; nothing overwritten: {}",
            path.display()
        )),
    }
}
fn id_valid(id: &str) -> bool {
    !id.is_empty() && id.len() < 100 && id.bytes().all(|c| c.is_ascii_digit() || c == b'-')
}
fn key_valid(key: &ReviewedKey) -> Result<(), String> {
    if !key.confirmed
        || ![
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ]
        .contains(&key.root.as_str())
        || !["major", "minor"].contains(&key.mode.as_str())
    {
        return Err("Review/correct the key and select major or minor before delivery".into());
    }
    Ok(())
}
fn audio_names(attempt: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(attempt).map_err(err)? {
        let p = entry.map_err(err)?.path();
        if p.extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("wav") || s.eq_ignore_ascii_case("mp3"))
        {
            regular_path(&p, false)?;
            names.push(
                p.file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("Unsupported filename")?
                    .into(),
            );
        }
    }
    names.sort();
    Ok(names)
}

fn tree(root: &Path, cancel: &AtomicBool) -> Result<(Vec<PathBuf>, Vec<Transfer>), String> {
    fn walk(
        root: &Path,
        relative: &Path,
        depth: usize,
        dirs: &mut Vec<PathBuf>,
        files: &mut Vec<Transfer>,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        integrity::check(cancel)?;
        if depth > 32 || dirs.len() + files.len() > 10000 {
            return Err("Project tree exceeds delivery limits (32 levels / 10000 entries)".into());
        }
        for entry in fs::read_dir(root.join(relative)).map_err(err)? {
            integrity::check(cancel)?;
            if dirs.len() + files.len() >= 10000 {
                return Err("Project exceeds 10000 delivery entries".into());
            }
            let entry = entry.map_err(err)?;
            let filename = entry.file_name();
            let name = filename.to_str().ok_or("Unsupported project filename")?;
            crate::rename::valid_name(name)?;
            let rel = relative.join(name);
            let meta = fs::symlink_metadata(entry.path()).map_err(err)?;
            if meta.is_dir() {
                regular_path(&entry.path(), true)?;
                dirs.push(rel.clone());
                walk(root, &rel, depth + 1, dirs, files, cancel)?;
            } else {
                regular_path(&entry.path(), false)?;
                files.push(Transfer {
                    source: root.join(&rel),
                    relative: rel,
                    fingerprint: integrity::fingerprint(&entry.path(), cancel)?,
                });
            }
        }
        Ok(())
    }
    regular_path(root, true)?;
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    walk(root, Path::new(""), 0, &mut dirs, &mut files, cancel)?;
    dirs.sort();
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok((dirs, files))
}
fn build(
    config: &ExportConfig,
    mut request: Request,
    id: String,
    resume: bool,
    cancel: &AtomicBool,
) -> Result<Plan, String> {
    key_valid(&request.key)?;
    if !id_valid(&id) {
        return Err("Invalid delivery identifier".into());
    }
    let layout = output_layout(config)?;
    request.attempt = regular_path(&request.attempt, true)?;
    if (request.attempt.parent() != Some(layout.stems_and_master_wav.as_path())
        && request.attempt.parent() != Some(exports_private_root()?.as_path()))
        || !request
            .attempt
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("1ce-attempt-"))
    {
        return Err("Choose an export attempt inside the configured Stems destination".into());
    }
    let record: AudioRecord = read_json(&request.attempt.join("audio-validated.json"))?;
    if record.state != "audio_exported" || !["wav", "mp3", "both"].contains(&record.format.as_str())
    {
        return Err("Audio export is not validated".into());
    }
    let expected = record
        .source_fingerprint
        .ok_or("This older export has no integrity record. Re-export before delivery")?;
    let project = regular_path(&record.project, false)?;
    integrity::matches(&project, &expected, cancel)?;
    let source = project.parent().ok_or("Missing source folder")?;
    let roots = [&layout.mp3, &layout.stems_and_master_wav, &layout.projects];
    let newest =
        crate::scanner::inspect_folder(source, crate::naming::DEFAULT_TEMPLATE).project_file;
    if newest.as_deref() != project.file_name().and_then(|n| n.to_str()) {
        return Err("A different/newer FLP is selected now; re-export before delivery".into());
    }
    for (i, root) in roots.iter().enumerate() {
        if root.starts_with(source) || source.starts_with(root.as_path()) {
            return Err("Destinations must be separate from the original project".into());
        }
        for other in roots.iter().skip(i + 1) {
            if root.starts_with(other) || other.starts_with(root) {
                return Err("Choose three separate, non-nested destination folders".into());
            }
        }
    }
    let bpm = crate::flp_metadata::read_flp_tempo(&project)
        .map_err(err)?
        .bpm;
    let key_token = format!(
        "{}{}",
        request.key.root,
        if request.key.mode == "minor" {
            "m"
        } else {
            "maj"
        }
    );
    let named =
        crate::naming::confirmed_title(&request.title, Some(bpm), &key_token, &request.template)?;
    if named.bpm_conflict {
        return Err(
            "Title BPM differs from the FLP; correct the queue title before delivery".into(),
        );
    }
    let name = named.proposed;
    crate::rename::valid_name(&name)?;
    let mut artifacts = record.artifacts;
    artifacts.sort_by(|a, b| a.file.cmp(&b.file));
    if artifacts.is_empty()
        || audio_names(&request.attempt)?
            != artifacts.iter().map(|a| a.file.clone()).collect::<Vec<_>>()
    {
        return Err("Audio artifact set changed or lacks checksums; re-export".into());
    }
    for artifact in &artifacts {
        crate::rename::valid_name(&artifact.file)?;
        integrity::matches(
            &request.attempt.join(&artifact.file),
            &artifact.fingerprint,
            cancel,
        )?;
    }
    let master = if record.stems {
        "master_Master.wav"
    } else {
        "master.wav"
    };
    let master_art = artifacts
        .iter()
        .find(|a| a.file == master)
        .ok_or("Validated master WAV missing")?;
    super::wave::validate(&request.attempt.join(master))?;
    let (mut dirs, mut files) = tree(source, cancel)?;
    omit_empty_export_stems(&mut dirs, &files);
    if files.iter().any(|f| f.relative == Path::new(OWNER)) {
        return Err("Project uses the reserved delivery marker filename".into());
    }
    let selected = project.file_name().ok_or("Missing FLP filename")?;
    for file in &mut files {
        if file.relative == Path::new(selected) {
            file.relative = PathBuf::from(format!("{name}.flp"));
            if file.fingerprint != expected {
                return Err("Selected FLP changed during preparation".into());
            }
        }
    }
    let mut targets = vec![Target {
        kind: "projects".into(),
        final_path: layout.projects.join(&name),
        stage: layout.projects.join(format!(".1ce-{id}-projects")),
        directory: true,
        dirs,
        files,
    }];
    if record.format != "mp3" || record.stems {
        let mut files = vec![Transfer {
            source: request.attempt.join(master),
            relative: PathBuf::from(format!("{name}.wav")),
            fingerprint: master_art.fingerprint.clone(),
        }];
        if record.stems {
            for a in &artifacts {
                if a.file.ends_with(".wav") && a.file != master && a.file != "master_Current.wav" {
                    super::wave::validate(&request.attempt.join(&a.file))?;
                    let label = a
                        .file
                        .strip_prefix("master_")
                        .ok_or("Unrecognized stem filename")?;
                    files.push(Transfer {
                        source: request.attempt.join(&a.file),
                        relative: PathBuf::from(label),
                        fingerprint: a.fingerprint.clone(),
                    });
                }
            }
            if files.len() < 2 {
                return Err("Separate stems missing".into());
            }
        }
        targets.push(Target {
            kind: "stems".into(),
            final_path: layout.stems_and_master_wav.join(&name),
            stage: layout.stems_and_master_wav.join(format!(".1ce-{id}-stems")),
            directory: true,
            dirs: vec![],
            files,
        });
    }
    if record.format != "wav" {
        let mp3 = artifacts
            .iter()
            .find(|a| a.file == "master.mp3")
            .ok_or("MP3 missing")?;
        super::jobs::validate_mp3(&request.attempt.join("master.mp3"))?;
        targets.push(Target {
            kind: "mp3".into(),
            final_path: layout.mp3.join(format!("{name}.mp3")),
            stage: layout.mp3.join(format!(".1ce-{id}-mp3")),
            directory: false,
            dirs: vec![],
            files: vec![Transfer {
                source: request.attempt.join("master.mp3"),
                relative: PathBuf::new(),
                fingerprint: mp3.fingerprint.clone(),
            }],
        });
    }
    for target in &targets {
        if target.directory && target.dirs.len() + target.files.len() + 1 > 10000 {
            return Err("Project has too many entries for delivery metadata".into());
        }
        let mut used = HashSet::new();
        for d in &target.dirs {
            used.insert(d.to_string_lossy().to_lowercase());
        }
        for f in &target.files {
            for component in f.relative.components() {
                if let std::path::Component::Normal(n) = component {
                    crate::rename::valid_name(n.to_str().ok_or("Unsupported name")?)?;
                } else {
                    return Err("Unsafe delivery path".into());
                }
            }
            if !used.insert(f.relative.to_string_lossy().to_lowercase()) {
                return Err("Final filenames collide inside the project or stems folder".into());
            }
        }
        if !resume {
            absent(&target.final_path)?;
            absent(&target.stage)?;
        }
    }
    Ok(Plan {
        version: 1,
        id,
        request,
        name,
        project,
        targets,
    })
}
fn preview(plan: &Plan, resumed: bool) -> Preview {
    Preview {
        id: plan.id.clone(),
        final_name: plan.name.clone(),
        source_project: plan.project.clone(),
        key: plan.request.key.clone(),
        destinations: plan.targets.iter().map(|t| t.final_path.clone()).collect(),
        file_count: plan.targets.iter().map(|t| t.files.len()).sum(),
        bytes: plan
            .targets
            .iter()
            .flat_map(|t| &t.files)
            .map(|f| f.fingerprint.bytes)
            .sum(),
        resumed,
    }
}
pub(super) fn automatic(
    config: &ExportConfig,
    request: Request,
    cancel: &AtomicBool,
) -> Result<Receipt, String> {
    let plan = prepare(config, request, cancel)?;
    finish_delivery(&plan, config, cancel, |_, _| {})
}

// Destructive step is restricted to automatic Export, after all final copies
// have been published and verified. A source changed meanwhile is never removed.
fn finish_source_move(plan: &Plan, cancel: &AtomicBool) -> Result<(), String> {
    integrity::check(cancel)?;
    for target in &plan.targets {
        verify(target, &target.final_path, &plan.id, cancel)?;
    }
    let source = plan.project.parent().ok_or("Missing source parent")?;
    let project_target = plan
        .targets
        .iter()
        .find(|t| t.kind == "projects")
        .ok_or("Project destination missing")?;
    let selected = plan.project.file_name().ok_or("Missing FLP name")?;
    let mut expected = project_target.files.clone();
    for f in &mut expected {
        if f.relative == PathBuf::from(format!("{}.flp", plan.name)) {
            f.relative = PathBuf::from(selected);
        }
        f.source = source.join(&f.relative);
    }
    expected.sort_by(|a, b| a.relative.cmp(&b.relative));
    let (dirs, files) = tree(source, cancel)?;
    let mut compared_dirs = dirs.clone();
    omit_empty_export_stems(&mut compared_dirs, &files);
    if compared_dirs != project_target.dirs || files != expected {
        return Err("Source changed after delivery; original folder preserved".into());
    }
    let holding = source
        .parent()
        .ok_or("Missing source parent")?
        .join(format!(".1ce-moving-{}", plan.id));
    absent(&holding)?;
    atomic_new(&plan.request.attempt.join("source-move.json"),&serde_json::to_vec_pretty(&serde_json::json!({"source":source,"holding":holding,"verifiedProject":project_target.final_path})).map_err(err)?,&plan.request.attempt)?;
    crate::rename::move_no_replace(source, &holding)?;
    let stable = tree(&holding, &AtomicBool::new(false));
    let matches = stable.as_ref().is_ok_and(|(d, f)| {
        *d == dirs
            && f.iter()
                .map(|x| (&x.relative, &x.fingerprint))
                .eq(expected.iter().map(|x| (&x.relative, &x.fingerprint)))
    });
    if !matches {
        crate::rename::move_no_replace(&holding, source)?;
        return Err("Source changed during move; restored original folder".into());
    }
    // No recursive delete: remove exactly the verified snapshot. Any unexpected
    // new file keeps the holding folder instead of being deleted.
    for f in &expected {
        integrity::matches(
            &holding.join(&f.relative),
            &f.fingerprint,
            &AtomicBool::new(false),
        )?;
        fs::remove_file(holding.join(&f.relative)).map_err(err)?;
    }
    let mut ordered = dirs;
    ordered.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in ordered {
        fs::remove_dir(holding.join(d)).map_err(err)?;
    }
    fs::remove_dir(&holding).map_err(err)?;
    for target in &plan.targets {
        // Only empty, owned staging containers are removed. Unexpected contents
        // are retained, never recursively erased.
        if target.stage.exists()
            && stage_owned(&target.stage, &plan.id, &plan.request.attempt).is_ok()
        {
            let _ = fs::remove_dir(target.stage.join("parts"));
            let _ = fs::remove_dir(&target.stage);
        }
    }
    atomic_new(
        &plan.request.attempt.join("source-move-complete.json"),
        b"complete",
        &plan.request.attempt,
    )?;
    Ok(())
}

fn completed_receipt(
    plan: &Plan,
    config: &ExportConfig,
    cancel: &AtomicBool,
) -> Result<Receipt, String> {
    key_valid(&plan.request.key)?;
    crate::rename::valid_name(&plan.name)?;
    if !id_valid(&plan.id) {
        return Err("Invalid delivery identifier".into());
    }
    let layout = output_layout(config)?;
    let mut seen = HashSet::new();
    for target in &plan.targets {
        if target.files.is_empty()
            || target.files.len() + target.dirs.len() > 10000
            || target.directory != (target.kind != "mp3")
        {
            return Err("Invalid recovery target shape".into());
        }
        for relative in target
            .dirs
            .iter()
            .chain(target.files.iter().map(|f| &f.relative))
        {
            if relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("Unsafe recovery path".into());
            }
        }
        let root = match target.kind.as_str() {
            "projects" => &layout.projects,
            "stems" => &layout.stems_and_master_wav,
            "mp3" => &layout.mp3,
            _ => return Err("Invalid delivery kind".into()),
        };
        let expected = root.join(if target.kind == "mp3" {
            format!("{}.mp3", plan.name)
        } else {
            plan.name.clone()
        });
        if target.final_path != expected || !seen.insert(target.kind.as_str()) {
            return Err("Recovery destinations changed".into());
        }
        exact_record(
            &plan
                .request
                .attempt
                .join(format!("delivery-intent-{}.json", target.kind)),
            &serde_json::to_vec(&serde_json::json!({"id":plan.id,"destination":target.final_path}))
                .map_err(err)?,
        )?;
        verify(target, &target.final_path, &plan.id, cancel)?;
    }
    if !seen.contains("projects") {
        return Err("Project destination missing".into());
    }
    let mut receipt: Receipt = read_json(&plan.request.attempt.join("delivery-complete.json"))?;
    if receipt.id != plan.id
        || receipt.final_name != plan.name
        || receipt.key != plan.request.key
        || receipt.destinations
            != plan
                .targets
                .iter()
                .map(|t| t.final_path.clone())
                .collect::<Vec<_>>()
        || !receipt.upload_ready
    {
        return Err("Completion receipt differs".into());
    }
    exact_record(
        &plan.request.attempt.join("source-move-complete.json"),
        b"complete",
    )?;
    receipt.source_action = "moved to verified destinations; source removed".into();
    Ok(receipt)
}
fn finish_delivery(
    plan: &Plan,
    config: &ExportConfig,
    cancel: &AtomicBool,
    progress: impl Fn(u64, u64),
) -> Result<Receipt, String> {
    let receipt = if plan
        .request
        .attempt
        .join("source-move-complete.json")
        .exists()
    {
        completed_receipt(plan, config, cancel)?
    } else {
        let mut receipt = execute(plan, config, cancel, progress)?;
        finish_source_move(plan, cancel)?;
        receipt.source_action = "moved to verified destinations; source removed".into();
        receipt
    };
    // Cleanup failure must not turn a verified delivered beat into a failed export.
    // Keep the manifest and retry on explicit recovery; no broad recursive deletion.
    if let Err(error) = super::source::cleanup_working_copy(&plan.request.attempt, cancel) {
        let _=atomic_new(&plan.request.attempt.join("working-copy-cleanup-warning.json"),&serde_json::to_vec(&serde_json::json!({"message":error,"action":"Retained private copy; retry completed delivery recovery"})).map_err(err)?,&plan.request.attempt);
    }
    Ok(receipt)
}

fn prepare(config: &ExportConfig, request: Request, cancel: &AtomicBool) -> Result<Plan, String> {
    let attempt = regular_path(&request.attempt, true)?;
    let layout = output_layout(config)?;
    if (attempt.parent() != Some(layout.stems_and_master_wav.as_path())
        && attempt.parent() != Some(exports_private_root()?.as_path()))
        || !attempt
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("1ce-attempt-"))
    {
        return Err(
            "Choose a private export attempt (or a compatible legacy attempt in Stems)".into(),
        );
    }
    let journal = attempt.join("delivery-plan.json");
    if journal.exists() {
        let old: Plan = read_json(&journal)?;
        if old.request != request {
            return Err("Recovery request differs from saved plan".into());
        }
        if attempt.join("source-move-complete.json").exists() {
            completed_receipt(&old, config, cancel)?;
            return Ok(old);
        }
        let source = old.project.parent().ok_or("Missing project parent")?;
        let holding = source
            .parent()
            .ok_or("Missing source parent")?
            .join(format!(".1ce-moving-{}", old.id));
        if holding.exists() {
            return Err(format!("Interrupted source move: files preserved at {}. Keep this folder and inspect {} before manual recovery; do not re-export or delete it.",holding.display(),attempt.join("source-move.json").display()));
        }
        let rebuilt = build(config, request, old.id.clone(), true, cancel)?;
        if old != rebuilt {
            return Err("Pending delivery inputs/configuration changed; originals and staged files retained".into());
        }
        Ok(rebuilt)
    } else {
        build(config, request, attempt_id(), false, cancel)
    }
}
fn atomic_new(path: &Path, bytes: &[u8], scratch: &Path) -> Result<(), String> {
    if path.exists() {
        return exact_record(path, bytes);
    }
    let temp = scratch.join(format!(".1ce-atomic-{}.part", attempt_id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_all().map_err(err)?;
    drop(file);
    crate::rename::move_no_replace(&temp, path)
}
fn owner_record(path: &Path, scratch: &Path) -> PathBuf {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(path.to_string_lossy().as_bytes());
    scratch.join(format!("stage-{:x}.json", hash))
}
fn stage_owned(path: &Path, id: &str, scratch: &Path) -> Result<(), String> {
    exact_record(
        &owner_record(path, scratch),
        &serde_json::to_vec(&serde_json::json!({"id":id,"path":path})).map_err(err)?,
    )
}
fn owner(path: &Path, id: &str, scratch: &Path) -> Result<(), String> {
    if !path.exists() {
        fs::create_dir(path).map_err(err)?;
    }
    regular_path(path, true)?;
    let record = owner_record(path, scratch);
    if !record.exists() && fs::read_dir(path).map_err(err)?.next().is_some() {
        return Err("Unowned staging directory is not empty".into());
    }
    atomic_new(
        &record,
        &serde_json::to_vec(&serde_json::json!({"id":id,"path":path})).map_err(err)?,
        scratch,
    )
}
fn omit_empty_export_stems(dirs: &mut Vec<PathBuf>, files: &[Transfer]) {
    let empty = dirs
        .iter()
        .find(|d| {
            d.components().count() == 1
                && d.to_string_lossy().eq_ignore_ascii_case("stems")
                && !dirs.iter().any(|child| child != *d && child.starts_with(d))
                && !files.iter().any(|f| f.relative.starts_with(d))
        })
        .cloned();
    if let Some(empty) = empty {
        dirs.retain(|d| d != &empty);
    }
}
fn mkdirs(base: &Path, relative: &Path) -> Result<(), String> {
    let mut current = base.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err("Unsafe relative directory".into());
        }
        current.push(part.as_os_str());
        if !current.exists() {
            fs::create_dir(&current).map_err(err)?;
        }
        regular_path(&current, true)?;
    }
    Ok(())
}
fn copy_checked(
    from: &Path,
    to: &Path,
    expected: &Fingerprint,
    parts: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    integrity::check(cancel)?;
    if to.exists() {
        return integrity::matches(to, expected, cancel);
    }
    let mut source = integrity::read_locked(from)?;
    let part = parts.join(format!("{}.part", attempt_id()));
    let mut dest = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(err)?;
    let mut buffer = [0u8; 65536];
    loop {
        integrity::check(cancel)?;
        let n = source.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        dest.write_all(&buffer[..n]).map_err(err)?;
    }
    dest.sync_all().map_err(err)?;
    drop(dest);
    drop(source);
    integrity::matches(&part, expected, cancel)?;
    integrity::matches(from, expected, cancel)?;
    crate::rename::move_no_replace(&part, to)
}
fn verify(target: &Target, path: &Path, id: &str, cancel: &AtomicBool) -> Result<(), String> {
    if !target.directory {
        return integrity::matches(path, &target.files[0].fingerprint, cancel);
    }
    regular_path(path, true)?;
    let _ = id;
    let (dirs, files) = tree(path, cancel)?;
    let actual: Vec<_> = files
        .iter()
        .map(|f| (f.relative.clone(), f.fingerprint.clone()))
        .collect();
    let mut expected: Vec<_> = target
        .files
        .iter()
        .map(|f| (f.relative.clone(), f.fingerprint.clone()))
        .collect();
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    if dirs != target.dirs || actual != expected {
        return Err(
            "Delivered directory changed or contains unexpected files; nothing overwritten".into(),
        );
    }
    Ok(())
}
fn execute(
    plan: &Plan,
    config: &ExportConfig,
    cancel: &AtomicBool,
    progress: impl Fn(u64, u64),
) -> Result<Receipt, String> {
    execute_hook(plan, config, cancel, progress, |_| {})
}
fn execute_hook(
    plan: &Plan,
    config: &ExportConfig,
    cancel: &AtomicBool,
    progress: impl Fn(u64, u64),
    published: impl Fn(usize),
) -> Result<Receipt, String> {
    let exists = plan.request.attempt.join("delivery-plan.json").exists();
    let rebuilt = build(
        config,
        plan.request.clone(),
        plan.id.clone(),
        exists,
        cancel,
    )?;
    if rebuilt != *plan {
        return Err("Delivery plan changed; prepare again".into());
    }
    let scratch = &plan.request.attempt;
    let plan_bytes = serde_json::to_vec_pretty(plan).map_err(err)?;
    if plan_bytes.len() > 16 * 1024 * 1024 {
        return Err("Delivery plan exceeds recovery record size limit".into());
    }
    atomic_new(&scratch.join("delivery-plan.json"), &plan_bytes, scratch)?;
    let total = preview(plan, false).bytes;
    let mut done = 0;
    // Stage and verify EVERY destination before publishing any of them.
    for target in &plan.targets {
        integrity::check(cancel)?;
        let payload = target.stage.join("payload");
        let intent = scratch.join(format!("delivery-intent-{}.json", target.kind));
        if target.final_path.exists() {
            if payload.exists() || !intent.exists() {
                return Err(
                    "Destination collision; staged files retained, nothing overwritten".into(),
                );
            }
            verify(target, &target.final_path, &plan.id, cancel)?;
            done += target
                .files
                .iter()
                .map(|f| f.fingerprint.bytes)
                .sum::<u64>();
            progress(done, total);
            continue;
        }
        owner(&target.stage, &plan.id, scratch)?;
        let parts = target.stage.join("parts");
        if !parts.exists() {
            fs::create_dir(&parts).map_err(err)?;
        }
        regular_path(&parts, true)?;
        if target.directory {
            owner(&payload, &plan.id, scratch)?;
            for dir in &target.dirs {
                mkdirs(&payload, dir)?;
            }
        }
        for file in &target.files {
            integrity::check(cancel)?;
            let to = if target.directory {
                payload.join(&file.relative)
            } else {
                payload.clone()
            };
            if target.directory {
                if let Some(parent) = file.relative.parent() {
                    mkdirs(&payload, parent)?;
                }
            }
            copy_checked(&file.source, &to, &file.fingerprint, &parts, cancel)?;
            done += file.fingerprint.bytes;
            progress(done, total);
        }
        verify(target, &payload, &plan.id, cancel)?;
    }
    for (index, target) in plan.targets.iter().enumerate() {
        integrity::check(cancel)?;
        let payload = target.stage.join("payload");
        let intent = scratch.join(format!("delivery-intent-{}.json", target.kind));
        let intent_bytes =
            serde_json::to_vec(&serde_json::json!({"id":plan.id,"destination":target.final_path}))
                .map_err(err)?;
        if target.final_path.exists() {
            if payload.exists() {
                return Err("Destination collision; nothing overwritten".into());
            }
            exact_record(&intent, &intent_bytes)?;
        } else {
            atomic_new(&intent, &intent_bytes, scratch)?;
            crate::rename::move_no_replace(&payload, &target.final_path)?;
        }
        verify(target, &target.final_path, &plan.id, cancel)?;
        published(index);
    }
    integrity::check(cancel)?;
    let receipt = Receipt {
        id: plan.id.clone(),
        final_name: plan.name.clone(),
        key: plan.request.key.clone(),
        destinations: plan.targets.iter().map(|t| t.final_path.clone()).collect(),
        manifest: scratch.join("delivery-complete.json"),
        upload_ready: true,
        source_action: "copied; originals not moved or renamed".into(),
    };
    atomic_new(
        &receipt.manifest,
        &serde_json::to_vec_pretty(&receipt).map_err(err)?,
        scratch,
    )?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        config: ExportConfig,
        request: Request,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn wav() -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend(40u32.to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(44100u32.to_le_bytes());
        b.extend(88200u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(4u32.to_le_bytes());
        b.extend([1, 0, 2, 0]);
        b
    }
    fn flp() -> Vec<u8> {
        let mut b = b"FLhd".to_vec();
        b.extend(6u32.to_le_bytes());
        b.extend([0, 0, 1, 0, 96, 0]);
        b.extend(b"FLdt");
        b.extend(5u32.to_le_bytes());
        b.push(156);
        b.extend(140000u32.to_le_bytes());
        b
    }
    impl Fixture {
        fn new(stems: bool) -> Self {
            let root = std::env::temp_dir().join(format!("onece-delivery-test-{}", attempt_id()));
            fs::create_dir(&root).unwrap();
            let source = root.join("original");
            fs::create_dir(&source).unwrap();
            fs::write(source.join("old.flp"), flp()).unwrap();
            fs::write(source.join("older.flp"), b"other project").unwrap();
            fs::create_dir(source.join("assets")).unwrap();
            fs::write(source.join("assets/sample.bin"), b"sample data").unwrap();
            fs::create_dir(source.join("empty")).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(source.join("older.flp"))
                .unwrap()
                .set_times(
                    fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1600000000)),
                )
                .unwrap();
            let paths: Vec<_> = ["mp3", "stems", "projects"]
                .iter()
                .map(|n| {
                    let p = root.join(n);
                    fs::create_dir(&p).unwrap();
                    p.canonicalize().unwrap()
                })
                .collect();
            let config = ExportConfig {
                mp3_folder: paths[0].to_string_lossy().into(),
                stems_folder: paths[1].to_string_lossy().into(),
                projects_folder: paths[2].to_string_lossy().into(),
                ..Default::default()
            };
            let attempt = paths[1].join("1ce-attempt-fixture");
            fs::create_dir(&attempt).unwrap();
            let master = if stems {
                "master_Master.wav"
            } else {
                "master.wav"
            };
            fs::write(attempt.join(master), wav()).unwrap();
            if stems {
                fs::write(attempt.join("master_Kick.wav"), wav()).unwrap();
                fs::write(attempt.join("master_Current.wav"), wav()).unwrap();
            }
            let request = Request {
                attempt,
                title: "Edited title 140 E @tof".into(),
                template: "{name} {bpm} {key} {producer}".into(),
                key: ReviewedKey {
                    root: "F#".into(),
                    mode: "minor".into(),
                    confirmed: true,
                },
            };
            let f = Self {
                root,
                config,
                request,
            };
            f.record("wav", stems);
            f
        }
        fn record(&self, format: &str, stems: bool) {
            let source = self.root.join("original/old.flp").canonicalize().unwrap();
            let cancel = AtomicBool::new(false);
            let artifacts: Vec<_> = audio_names(&self.request.attempt)
                .unwrap()
                .iter()
                .map(|name| Artifact {
                    file: name.clone(),
                    fingerprint: integrity::fingerprint(&self.request.attempt.join(name), &cancel)
                        .unwrap(),
                })
                .collect();
            let record = serde_json::json!({"project":source,"state":"audio_exported","format":format,"stems":stems,"sourceFingerprint":integrity::fingerprint(&source,&cancel).unwrap(),"artifacts":artifacts});
            fs::write(
                self.request.attempt.join("audio-validated.json"),
                serde_json::to_vec_pretty(&record).unwrap(),
            )
            .unwrap();
        }
        fn plan(&self) -> Plan {
            prepare(&self.config, self.request.clone(), &AtomicBool::new(false)).unwrap()
        }
        fn assert_sources(&self) {
            assert_eq!(fs::read(self.root.join("original/old.flp")).unwrap(), flp());
            assert_eq!(
                fs::read(self.root.join("original/older.flp")).unwrap(),
                b"other project"
            );
            assert_eq!(
                fs::read(self.root.join("original/assets/sample.bin")).unwrap(),
                b"sample data"
            );
        }
    }
    #[test]
    fn completed_recovery_verifies_targets_without_requiring_source() {
        let f = Fixture::new(false);
        let plan = f.plan();
        finish_delivery(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert!(!f.root.join("original").exists());
        let recovered = prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).unwrap();
        // A new folder at the old location belongs to the user; never delete it.
        fs::create_dir(f.root.join("original")).unwrap();
        fs::write(f.root.join("original/new.txt"), b"keep").unwrap();
        assert!(
            finish_delivery(&recovered, &f.config, &AtomicBool::new(false), |_, _| {})
                .unwrap()
                .upload_ready
        );
        assert_eq!(fs::read(f.root.join("original/new.txt")).unwrap(), b"keep");
        let mut changed = f.config.clone();
        changed.projects_folder = changed.mp3_folder.clone();
        assert!(prepare(&changed, f.request.clone(), &AtomicBool::new(false)).is_err());
        fs::write(
            plan.targets[0]
                .final_path
                .join(format!("{}.flp", plan.name)),
            b"edited",
        )
        .unwrap();
        assert!(prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn interrupted_source_move_reports_holding_without_deleting_it() {
        let f = Fixture::new(false);
        let plan = f.plan();
        execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap();
        let holding = f.root.join(format!(".1ce-moving-{}", plan.id));
        fs::rename(f.root.join("original"), &holding).unwrap();
        assert!(
            prepare(&f.config, f.request.clone(), &AtomicBool::new(false))
                .unwrap_err()
                .contains("Interrupted source move")
        );
        assert_eq!(fs::read(holding.join("old.flp")).unwrap(), flp());
    }
    #[test]
    fn successful_delivery_cleans_only_manifested_internal_copy() {
        let f = Fixture::new(false);
        super::super::source::working_copy(
            &f.root.join("original/old.flp"),
            &f.request.attempt,
            &AtomicBool::new(false),
        )
        .unwrap();
        automatic(&f.config, f.request.clone(), &AtomicBool::new(false)).unwrap();
        assert!(!f.request.attempt.join("render-input").exists());
        assert!(f.request.attempt.join("master.wav").exists());
        assert!(f.request.attempt.join("delivery-plan.json").exists());
    }
    #[test]
    fn automatic_delivery_moves_verified_project_and_cleans_staging() {
        let f = Fixture::new(false);
        let source = f.root.join("original");
        let before = tree(&source, &AtomicBool::new(false)).unwrap();
        let receipt = automatic(&f.config, f.request.clone(), &AtomicBool::new(false)).unwrap();
        assert_eq!(receipt.final_name, "Edited title 140 F#m @tof");
        assert!(receipt.upload_ready);
        assert!(!source.exists());
        let project = Path::new(&f.config.projects_folder).join(&receipt.final_name);
        assert_eq!(
            fs::read(project.join(format!("{}.flp", receipt.final_name))).unwrap(),
            flp()
        );
        assert!(!before.1.is_empty());
        assert!(receipt.source_action.starts_with("moved"));
    }
    #[test]
    fn move_refuses_source_changes_cancellation_and_modified_destinations() {
        for scenario in 0..3 {
            let f = Fixture::new(false);
            let plan = f.plan();
            execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap();
            if scenario == 0 {
                fs::write(f.root.join("original/new.wav"), b"new data").unwrap();
            }
            if scenario == 1 {
                fs::write(
                    plan.targets[0]
                        .final_path
                        .join(format!("{}.flp", plan.name)),
                    b"changed destination",
                )
                .unwrap();
            }
            assert!(finish_source_move(&plan, &AtomicBool::new(scenario == 2)).is_err());
            assert_eq!(fs::read(f.root.join("original/old.flp")).unwrap(), flp());
        }
    }
    #[test]
    fn final_folders_have_no_owner_markers_or_empty_export_stems() {
        for populated in [false, true] {
            let f = Fixture::new(true);
            let stems = f.root.join("original/stems");
            fs::create_dir(&stems).unwrap();
            if populated {
                fs::write(stems.join("keep.wav"), b"existing stem").unwrap();
            }
            let receipt = automatic(&f.config, f.request.clone(), &AtomicBool::new(false)).unwrap();
            let project = Path::new(&f.config.projects_folder).join(&receipt.final_name);
            assert_eq!(project.join("stems").exists(), populated);
            for dest in &receipt.destinations {
                if dest.is_dir() {
                    assert!(!dest.join(OWNER).exists());
                }
            }
            assert!(!f.root.join("original").exists());
            if populated {
                assert_eq!(
                    fs::read(project.join("stems/keep.wav")).unwrap(),
                    b"existing stem"
                );
            }
        }
    }
    #[test]
    fn reviewed_names_copy_project_assets_and_wav_stems_without_touching_sources() {
        for stems in [false, true] {
            let f = Fixture::new(stems);
            let plan = f.plan();
            assert_eq!(plan.name, "Edited title 140 F#m @tof");
            let receipt = execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap();
            assert!(receipt.upload_ready);
            assert!(receipt.key.confirmed);
            assert_eq!(receipt.key.mode, "minor");
            let project = Path::new(&f.config.projects_folder).join(&plan.name);
            assert_eq!(
                fs::read(project.join(format!("{}.flp", plan.name))).unwrap(),
                flp()
            );
            assert!(project.join("empty").is_dir());
            assert_eq!(
                fs::read(project.join("assets/sample.bin")).unwrap(),
                b"sample data"
            );
            assert!(!project.join("old.flp").exists());
            let audio = Path::new(&f.config.stems_folder).join(&plan.name);
            assert!(audio.join(format!("{}.wav", plan.name)).is_file());
            assert_eq!(audio.join("Kick.wav").exists(), stems);
            assert!(!audio.join("Current.wav").exists());
            f.assert_sources();
            let again = prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).unwrap();
            assert_eq!(again, plan);
            assert_eq!(
                execute(&again, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap(),
                receipt
            );
        }
    }
    #[test]
    fn cancelled_staging_and_partial_publication_resume_without_overwrite() {
        for phase in [0, 1] {
            let f = Fixture::new(true);
            let plan = f.plan();
            let cancel = AtomicBool::new(false);
            let result = execute_hook(
                &plan,
                &f.config,
                &cancel,
                |_, _| {
                    if phase == 0 {
                        cancel.store(true, Ordering::Release);
                    }
                },
                |index| {
                    if phase == 1 && index == 0 {
                        cancel.store(true, Ordering::Release);
                    }
                },
            );
            assert!(result.unwrap_err().contains("cancelled"));
            assert!(!f.request.attempt.join("delivery-complete.json").exists());
            f.assert_sources();
            let restored = prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).unwrap();
            assert_eq!(restored, plan);
            execute(&restored, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap();
            f.assert_sources();
        }
    }
    #[test]
    fn collision_and_external_changes_are_never_overwritten() {
        let f = Fixture::new(false);
        let plan = f.plan();
        fs::create_dir(&plan.targets[0].final_path).unwrap();
        fs::write(plan.targets[0].final_path.join("keep.txt"), b"user data").unwrap();
        assert!(execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).is_err());
        assert_eq!(
            fs::read(plan.targets[0].final_path.join("keep.txt")).unwrap(),
            b"user data"
        );
        assert!(!f.request.attempt.join("delivery-plan.json").exists());
        let f = Fixture::new(false);
        let plan = f.plan();
        execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap();
        let published = plan.targets[0]
            .final_path
            .join(format!("{}.flp", plan.name));
        fs::write(&published, b"user edited delivered project").unwrap();
        assert!(execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).is_err());
        assert_eq!(
            fs::read(published).unwrap(),
            b"user edited delivered project"
        );
        f.assert_sources();
    }
    #[test]
    fn changed_source_asset_audio_and_unreviewed_key_block_delivery() {
        let f = Fixture::new(false);
        let mut req = f.request.clone();
        req.key.confirmed = false;
        assert!(prepare(&f.config, req, &AtomicBool::new(false)).is_err());
        let plan = f.plan();
        fs::write(f.root.join("original/assets/sample.bin"), b"changed asset").unwrap();
        assert!(execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).is_err());
        assert!(!f.request.attempt.join("delivery-plan.json").exists());
        let f = Fixture::new(false);
        fs::write(f.root.join("original/old.flp"), b"changed project").unwrap();
        assert!(prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).is_err());
        let f = Fixture::new(false);
        fs::write(f.request.attempt.join("master.wav"), b"changed wave").unwrap();
        assert!(prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).is_err());
        let f = Fixture::new(false);
        let newer = f.root.join("original/new.flp");
        fs::write(&newer, flp()).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(newer)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(30)),
            )
            .unwrap();
        assert!(prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn conflicting_project_names_tampered_journals_and_nested_destinations_are_rejected() {
        let f = Fixture::new(false);
        fs::write(
            f.root.join("original/Edited title 140 F#m @tof.flp"),
            b"existing project",
        )
        .unwrap();
        assert!(prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).is_err());
        let f = Fixture::new(false);
        let plan = f.plan();
        let cancel = AtomicBool::new(false);
        let _ = execute(&plan, &f.config, &cancel, |_, _| {
            cancel.store(true, Ordering::Release)
        });
        let mut changed = plan.clone();
        changed.targets[0].final_path = f.root.join("unapproved");
        fs::write(
            f.request.attempt.join("delivery-plan.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(prepare(&f.config, f.request.clone(), &AtomicBool::new(false)).is_err());
        assert!(!f.root.join("unapproved").exists());
        let f = Fixture::new(false);
        let mut config = f.config.clone();
        config.mp3_folder = config.projects_folder.clone();
        assert!(prepare(&config, f.request.clone(), &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn sha256_known_vector_and_cancelled_hash() {
        let f = Fixture::new(false);
        let p = f.root.join("hash.bin");
        fs::write(&p, b"abc").unwrap();
        assert_eq!(
            integrity::fingerprint(&p, &AtomicBool::new(false))
                .unwrap()
                .sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(integrity::fingerprint(&p, &AtomicBool::new(true)).is_err());
    }
    #[test]
    #[ignore = "Explicit real-codec delivery matrix; all input and destination files are private copies"]
    fn real_codec_delivery_matrix() {
        let mp3 = std::env::var("ONECE_DELIVERY_SMOKE_MP3").expect("Set validated MP3 fixture");
        let wav = std::env::var("ONECE_DELIVERY_SMOKE_WAV").expect("Set validated WAV fixture");
        super::super::jobs::validate_mp3(Path::new(&mp3)).unwrap();
        super::super::wave::validate(Path::new(&wav)).unwrap();
        for format in ["wav", "mp3", "both"] {
            for stems in [false, true] {
                let f = Fixture::new(stems);
                let master = if stems {
                    "master_Master.wav"
                } else {
                    "master.wav"
                };
                fs::copy(&wav, f.request.attempt.join(master)).unwrap();
                if format != "wav" {
                    fs::copy(&mp3, f.request.attempt.join("master.mp3")).unwrap();
                }
                f.record(format, stems);
                let plan = f.plan();
                let receipt =
                    execute(&plan, &f.config, &AtomicBool::new(false), |_, _| {}).unwrap();
                assert!(receipt.upload_ready);
                assert_eq!(
                    receipt.destinations.len(),
                    1 + usize::from(format != "wav") + usize::from(format != "mp3" || stems)
                );
                f.assert_sources();
                println!("Delivery {format}, stems={stems}: verified");
            }
        }
    }
    #[test]
    #[ignore = "Explicit FL render through final delivery; all projects/destinations are private copies"]
    fn real_render_then_delivery() {
        let source =
            std::env::var("ONECE_EXPORT_SMOKE_PROJECT").expect("Set disposable FLP fixture");
        let before = fs::read(&source).unwrap();
        let mut f = Fixture::new(false);
        let input = f.root.join("original/old.flp");
        fs::write(&input, &before).unwrap();
        f.config.executable = "C:\\Program Files\\Image-Line\\FL Studio 2024\\FL64.exe".into();
        let cancel = AtomicBool::new(false);
        f.request.attempt =
            super::super::render_audio(&f.config, &input, "wav", false, &cancel, |_| {}).unwrap();
        f.request.title = format!(
            "Smoke {} @test",
            crate::flp_metadata::read_flp_tempo(&input).unwrap().bpm
        );
        let plan = f.plan();
        let receipt = execute(&plan, &f.config, &cancel, |_, _| {}).unwrap();
        assert!(receipt.upload_ready);
        finish_source_move(&plan, &cancel).unwrap();
        let delivered = Path::new(&f.config.projects_folder)
            .join(&receipt.final_name)
            .join(format!("{}.flp", receipt.final_name));
        assert_eq!(fs::read(delivered).unwrap(), before);
        assert!(!input.exists());
        assert_eq!(fs::read(source).unwrap(), before);
        println!(
            "Fresh FL render + checksums + verified project move: verified (temporary fixture)"
        );
    }
}

fn remember(state: &DeliveryState, plan: Plan) -> Result<Preview, String> {
    let resumed = plan.request.attempt.join("delivery-plan.json").exists();
    let result = preview(&plan, resumed);
    let mut plans = state.0.lock().map_err(err)?;
    plans.retain(|_, p| p.request.attempt != plan.request.attempt);
    if plans.len() >= 250 {
        return Err("Too many prepared delivery plans".into());
    }
    plans.insert(plan.id.clone(), plan);
    Ok(result)
}

#[tauri::command]
pub async fn prepare_delivery(
    app: tauri::AppHandle,
    exports: tauri::State<'_, jobs::ExportState>,
    state: tauri::State<'_, DeliveryState>,
    request: Request,
) -> Result<Preview, String> {
    let config = get_export_config(app)?;
    let operation = exports.reserve_operation()?;
    let cancel = operation.cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let attempt = request.attempt.clone();
        operation.status(
            &attempt,
            "preparing_delivery",
            "Preparing final names and checksums…",
            false,
        );
        let result = prepare(&config, request, &cancel);
        match &result {
            Ok(_) => operation.status(
                &attempt,
                "delivery_ready",
                "Final paths prepared · review before delivery",
                false,
            ),
            Err(e) => operation.status(&attempt, "delivery_failed", e, false),
        }
        drop(operation);
        result
    })
    .await
    .map_err(err)??;
    remember(&state, result)
}
#[tauri::command]
pub async fn resume_delivery(
    app: tauri::AppHandle,
    exports: tauri::State<'_, jobs::ExportState>,
    state: tauri::State<'_, DeliveryState>,
    attempt: PathBuf,
) -> Result<Preview, String> {
    let config = get_export_config(app)?;
    let operation = exports.reserve_operation()?;
    let cancel = operation.cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let attempt = regular_path(&attempt, true)?;
        let old: Plan = read_json(&attempt.join("delivery-plan.json"))?;
        let mut request = old.request;
        request.attempt = attempt;
        let result = prepare(&config, request, &cancel);
        drop(operation);
        result
    })
    .await
    .map_err(err)??;
    remember(&state, result)
}
#[tauri::command]
pub async fn execute_delivery(
    app: tauri::AppHandle,
    exports: tauri::State<'_, jobs::ExportState>,
    state: tauri::State<'_, DeliveryState>,
    id: String,
) -> Result<Receipt, String> {
    let plan = state
        .0
        .lock()
        .map_err(err)?
        .get(&id)
        .cloned()
        .ok_or("Delivery review expired; prepare or resume again")?;
    let config = get_export_config(app)?;
    let operation = exports.reserve_operation()?;
    let attempt = plan.request.attempt.clone();
    operation.status(
        &attempt,
        "delivering",
        "Copying and verifying final files…",
        false,
    );
    let cancel = operation.cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = finish_delivery(&plan, &config, &cancel, |done, total| {
            operation.status(
                &attempt,
                "delivering",
                &format!("Verifying delivery: {done}/{total} bytes"),
                false,
            )
        });
        match &result {
            Ok(receipt) => operation.receipt(&attempt, receipt),
            Err(e) => operation.status(&attempt, "delivery_failed", e, false),
        }
        drop(operation);
        result
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn export_recovery_directory() -> Result<PathBuf, String> {
    exports_private_root()
}
