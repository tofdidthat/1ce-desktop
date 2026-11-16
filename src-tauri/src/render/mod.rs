//! Windows audio export, scoped FL Studio 24 settings and output validation.
//! Key detection, final organization and upload remain separate pending stages.
pub mod finalize;
mod integrity;
pub mod jobs;
mod pipeline;
#[cfg(windows)]
mod settings;
mod source;
pub(crate) mod wave;
#[cfg(windows)]
mod win32;
pub use jobs::ExportState;

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

pub(super) fn attempt_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

pub(super) fn exports_private_root() -> Result<PathBuf, String> {
    let root =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
            .join("1ce-desktop")
            .join("exports");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    regular_path(&root, true)
}

// Canonical filesystem paths use the Win32 extended prefix; FL's own command
// parser expects ordinary DOS/UNC paths rather than interpreting '?' as a mask.
fn fl_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{unc}")
    } else {
        text.strip_prefix("\\\\?\\").unwrap_or(&text).to_owned()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportConfig {
    executable: String,
    mp3_folder: String,
    stems_folder: String,
    projects_folder: String,
    compatibility_mode: bool,
    #[serde(skip_serializing)]
    output_folder: String, // Read legacy single-root preferences only.
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            executable: String::new(),
            mp3_folder: String::new(),
            stems_folder: String::new(),
            projects_folder: String::new(),
            compatibility_mode: true,
            output_folder: String::new(),
        }
    }
}

fn migrate_config(mut config: ExportConfig) -> ExportConfig {
    if !config.output_folder.is_empty() {
        let root = Path::new(&config.output_folder);
        if config.mp3_folder.is_empty() {
            config.mp3_folder = root.join("MP3").to_string_lossy().into_owned();
        }
        if config.stems_folder.is_empty() {
            config.stems_folder = root.join("Stems").to_string_lossy().into_owned();
        }
        if config.projects_folder.is_empty() {
            config.projects_folder = root.join("Projects").to_string_lossy().into_owned();
        }
        config.output_folder.clear();
    }
    config
}

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("export-config.json"))
}

pub(crate) fn regular_path(path: &Path, directory: bool) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute local path".into());
    }
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Export does not support junctions or cloud placeholders".into());
            }
        }
        if meta.file_type().is_symlink() {
            return Err("Export does not support symbolic links".into());
        }
    }
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err("Wrong path type".into());
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}

fn validate_config(config: &ExportConfig) -> Result<(), String> {
    if !config.executable.is_empty() {
        let exe = Path::new(&config.executable);
        if !exe
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("FL64.exe") || n.eq_ignore_ascii_case("FL.exe"))
        {
            return Err("Select FL Studio's FL64.exe or FL.exe".into());
        }
        regular_path(exe, false)?;
    }
    for folder in [
        &config.mp3_folder,
        &config.stems_folder,
        &config.projects_folder,
    ] {
        if !folder.is_empty() {
            regular_path(Path::new(folder), true)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_export_config(app: tauri::AppHandle) -> Result<ExportConfig, String> {
    let path = config_path(&app)?;
    if path.exists() {
        return serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map(migrate_config)
            .map_err(|e| e.to_string());
    }
    let default_exe = Path::new("C:\\Program Files\\Image-Line\\FL Studio 2024\\FL64.exe");
    Ok(ExportConfig {
        executable: if default_exe.is_file() {
            default_exe.to_string_lossy().into_owned()
        } else {
            String::new()
        },
        ..Default::default()
    })
}

#[tauri::command]
pub fn save_export_config(
    app: tauri::AppHandle,
    config: ExportConfig,
) -> Result<ExportConfig, String> {
    let config = migrate_config(config);
    validate_config(&config)?;
    let path = config_path(&app)?;
    fs::create_dir_all(path.parent().ok_or("Invalid config directory")?)
        .map_err(|e| e.to_string())?;
    fs::write(
        path,
        serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(config)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLayout {
    mp3: PathBuf,
    stems_and_master_wav: PathBuf,
    projects: PathBuf,
}

fn output_layout(config: &ExportConfig) -> Result<OutputLayout, String> {
    if [
        &config.mp3_folder,
        &config.stems_folder,
        &config.projects_folder,
    ]
    .iter()
    .any(|p| p.is_empty())
    {
        return Err("Choose the MP3, Stems + WAV and Projects destinations".into());
    }
    Ok(OutputLayout {
        mp3: regular_path(Path::new(&config.mp3_folder), true)?,
        stems_and_master_wav: regular_path(Path::new(&config.stems_folder), true)?,
        projects: regular_path(Path::new(&config.projects_folder), true)?,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    project: PathBuf,
    output_layout: OutputLayout,
    formats: String,
    stems: bool,
    execution_enabled: bool,
    notes: Vec<String>,
}

#[tauri::command]
pub fn preview_export_plan(
    app: tauri::AppHandle,
    folder: String,
    expected_file: String,
    format: String,
    stems: bool,
) -> Result<ExportPlan, String> {
    let config = get_export_config(app)?;
    validate_config(&config)?;
    if config.executable.is_empty() {
        return Err("Configure FL Studio first".into());
    }
    let source = regular_path(Path::new(&folder), true)?;
    let scan = crate::scanner::inspect_folder(&source, crate::naming::DEFAULT_TEMPLATE);
    if scan.project_file.as_deref() != Some(expected_file.as_str()) {
        return Err("Selected FLP changed. Scan again.".into());
    }
    let project = regular_path(&source.join(expected_file), false)?;
    crate::flp_metadata::read_flp_tempo(&project)
        .map_err(|e| format!("Cannot validate FLP: {e}"))?;
    let layout = output_layout(&config)?;
    for output in [&layout.mp3, &layout.stems_and_master_wav, &layout.projects] {
        if output.starts_with(&source) || source.starts_with(output) {
            return Err("Every output destination must be separate from the source project folder and its ancestors".into());
        }
    }
    let formats = match format.as_str() {
        "wav" => "wav",
        "mp3" | "both" => "wav,mp3",
        _ => return Err("Unsupported audio format".into()),
    };
    Ok(ExportPlan { project, output_layout: layout, formats: formats.into(), stems, execution_enabled: true,
        notes: vec!["WAV master required internally for Find Key; no source rename during rendering".into(), "Stems switch is applied automatically for FL Studio 24 and the prior split preference is restored after rendering; absent or invalid stems fail validation".into(), "Audio export only: key detection, final naming and upload remain pending".into()] })
}

// Private until acceptance. All paths originate in native preflight, not raw IPC.
// A worker may call this sequentially, then enqueue the returned WAV for Find Key
// while the next render runs. Never marks upload-ready or assumes a key exists.
#[allow(dead_code)]
#[cfg(windows)]
fn render_master(
    config: &ExportConfig,
    project: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let folder = render_audio(config, project, "wav", false, cancel, |_| {})?;
    Ok(folder.join("master.wav"))
}

#[cfg(windows)]
fn render_audio(
    config: &ExportConfig,
    project: &Path,
    format: &str,
    stems: bool,
    cancel: &AtomicBool,
    on_stage: impl Fn(&str),
) -> Result<PathBuf, String> {
    retry_startup(
        cancel,
        Duration::from_secs(5),
        || render_audio_once(config, project, format, stems, cancel, &on_stage),
        || on_stage("retrying"),
    )
}

const STARTUP_NO_AUDIO: &str = "FL_STARTUP_NO_AUDIO: ";
fn retry_startup<T>(
    cancel: &AtomicBool,
    delay: Duration,
    mut action: impl FnMut() -> Result<T, String>,
    mut retry: impl FnMut(),
) -> Result<T, String> {
    for index in 0..3 {
        if cancel.load(Ordering::Acquire) {
            return Err("Export cancelled before renderer startup".into());
        }
        match action() {
            Err(error) if error.starts_with(STARTUP_NO_AUDIO) && index < 2 => {
                retry();
                let start = std::time::Instant::now();
                while start.elapsed() < delay {
                    if cancel.load(Ordering::Acquire) {
                        return Err("Export cancelled during startup recovery".into());
                    }
                    std::thread::sleep(
                        Duration::from_millis(50).min(delay.saturating_sub(start.elapsed())),
                    );
                }
            }
            Err(error) => {
                return Err(error
                    .strip_prefix(STARTUP_NO_AUDIO)
                    .unwrap_or(&error)
                    .to_owned())
            }
            Ok(value) => return Ok(value),
        }
    }
    unreachable!()
}

#[cfg(windows)]
fn render_audio_once(
    config: &ExportConfig,
    project: &Path,
    format: &str,
    stems: bool,
    cancel: &AtomicBool,
    on_stage: impl Fn(&str),
) -> Result<PathBuf, String> {
    validate_config(config)?;
    let project = regular_path(project, false)?;
    crate::flp_metadata::read_flp_tempo(&project).map_err(|e| e.to_string())?;
    let layout = output_layout(config)?;
    let output = &layout.stems_and_master_wav;
    let source = project.parent().ok_or("Project has no parent")?;
    for destination in [&layout.mp3, output, &layout.projects] {
        if destination.starts_with(source) || source.starts_with(destination) {
            return Err("Output overlaps the source project".into());
        }
    }
    // Keep the source read-only to other processes for the duration of the job.
    use std::os::windows::fs::OpenOptionsExt;
    let _source_lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&project)
        .map_err(|e| format!("Close FL Studio before export: {e}"))?;
    if cancel.load(Ordering::Acquire) {
        return Err("Cancelled".into());
    }
    let attempt = exports_private_root()?.join(format!("1ce-attempt-{}", attempt_id()));
    fs::create_dir(&attempt).map_err(|e| e.to_string())?;
    let manifest = attempt.join("attempt.json");
    let record = serde_json::json!({"project":project,"state":"rendering","uploadReady":false});
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&manifest)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(serde_json::to_string_pretty(&record).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    let wav = attempt.join("master.wav");
    let formats = if format == "wav" { "wav" } else { "wav,mp3" };
    on_stage("preparing_source");
    let renderer_project = source::working_copy(&project, &attempt, cancel)?;
    let renderer_output = source.join("stems");
    if !renderer_output.exists() {
        fs::create_dir(&renderer_output).map_err(|e| e.to_string())?;
    }
    let renderer_output = regular_path(&renderer_output, true)?;
    let export_prefix = format!("1ce-{}", attempt_id());
    if fs::read_dir(&renderer_output)
        .map_err(|e| e.to_string())?
        .any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with(&export_prefix)))
    {
        return Err("Export prefix collision; existing stems preserved".into());
    }
    let args = vec![
        format!(
            "/R{}",
            fl_path(&renderer_output.join(format!("{export_prefix}.wav")))
        ),
        format!("/E{formats}"),
        format!("/O{}", fl_path(&renderer_output)),
        fl_path(&renderer_project),
    ];
    fs::write(attempt.join("renderer-command.json"),serde_json::to_vec_pretty(&serde_json::json!({"executable":config.executable,"arguments":args,"format":format,"stems":stems,"sourceUnchanged":project})).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    on_stage("rendering");
    let prefs = settings::SettingsGuard::begin(Path::new(&config.executable), stems)?;
    let exe_directory = Path::new(&config.executable)
        .parent()
        .ok_or("FL Studio installation directory not found")?;
    let started = std::time::Instant::now();
    let process = win32::run_with_mode(
        Path::new(&config.executable),
        &args,
        exe_directory,
        cancel,
        Duration::from_secs(if cfg!(test) { 180 } else { 30 * 60 }),
        config.compatibility_mode,
    );
    let restored = prefs.finish();
    let diagnostic = serde_json::json!({"mode":if config.compatibility_mode{"compatibility"}else{"isolated"},"elapsedMs":started.elapsed().as_millis(),"processResult":process,"settingsRestored":restored.is_ok(),"settingsError":restored.as_ref().err()});
    fs::write(
        attempt.join("process-result.json"),
        serde_json::to_vec_pretty(&diagnostic).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    restored.map_err(|e|format!("FL Studio render preference restoration needs attention: {e}. Recovery record retained in app data"))?;
    let code = process?;
    if code != 0 {
        return Err(format!(
            "FL Studio exited with code {code}; retained {}",
            attempt.display()
        ));
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Cancelled after render; not finalized".into());
    }
    collect_generated_audio(&renderer_output, &attempt, &export_prefix, cancel)?;
    // FL Studio 24 names the mix master_Master.wav when split rendering is on.
    let master = if stems {
        attempt.join("master_Master.wav")
    } else {
        wav.clone()
    };
    if !master.exists() {
        let no_audio = fs::read_dir(&attempt).is_ok_and(|mut entries| {
            entries.all(|entry| {
                entry.is_ok_and(|e| {
                    !e.path().extension().is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("wav") || ext.eq_ignore_ascii_case("mp3")
                    })
                })
            })
        });
        let retryable = started.elapsed() < Duration::from_secs(5) && no_audio;
        return Err(format!("{}FL Studio exited without producing the master WAV (exit {code}, {} ms, {} mode). This is not a completed export. {}Details retained at {}",if retryable{STARTUP_NO_AUDIO}else{""},started.elapsed().as_millis(),if config.compatibility_mode{"compatibility"}else{"isolated"},if config.compatibility_mode{""}else{"Enable Render compatibility mode in Settings and try again. "},attempt.display()));
    }
    on_stage("validating");
    wave::validate(&master)?;
    if format != "wav" {
        jobs::validate_mp3(&attempt.join("master.mp3"))?;
    }
    let mut count = 0;
    for entry in fs::read_dir(&attempt).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path != master
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        {
            if !stems {
                return Err(
                    "FL Studio generated split tracks despite Stems being off; export rejected"
                        .into(),
                );
            }
            regular_path(&path, false)?;
            wave::validate(&path)?;
            if path
                .file_name()
                .is_none_or(|name| name != "master_Current.wav")
            {
                count += 1;
            }
        }
    }
    if stems && count == 0 {
        return Err(format!("FL Studio did not honor automatic Stems; no separate tracks were generated. Files retained at {}",attempt.display()));
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Cancelled during validation".into());
    }
    let source_fingerprint = integrity::fingerprint(&project, cancel)?;
    let mut artifacts = Vec::new();
    for entry in fs::read_dir(&attempt).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wav") || ext.eq_ignore_ascii_case("mp3"))
        {
            artifacts.push(integrity::Artifact {
                file: path
                    .file_name()
                    .ok_or("Missing audio filename")?
                    .to_string_lossy()
                    .into_owned(),
                fingerprint: integrity::fingerprint(&path, cancel)?,
            });
        }
    }
    artifacts.sort_by(|a, b| a.file.cmp(&b.file));
    let record = serde_json::json!({"project":project,"sourceFingerprint":source_fingerprint,"artifacts":artifacts,"state":"audio_exported","uploadReady":false,"key":"pending","finalNaming":"pending","format":format,"stems":stems,"masterWav":master,"separateWavCount":count});
    let mut completion = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(attempt.join("audio-validated.json"))
        .map_err(|e| e.to_string())?;
    completion
        .write_all(serde_json::to_string_pretty(&record).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;
    completion.sync_all().map_err(|e| e.to_string())?;
    // This is validated WAV only, not a completed export/upload. Retain the
    // manifest as unfinished until codec/stems/key/finalization are implemented.
    Ok(attempt)
}

// Move only this export's uniquely prefixed audio, never pre-existing stems.
fn collect_generated_audio(
    from: &Path,
    to: &Path,
    prefix: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
        integrity::check(cancel)?;
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = name.strip_prefix(prefix) else {
            continue;
        };
        if !suffix.starts_with(['.', '_'])
            || !entry.path().extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("wav") || ext.eq_ignore_ascii_case("mp3")
            })
        {
            continue;
        }
        let source = regular_path(&entry.path(), false)?;
        let target = to.join(format!("master{suffix}"));
        let expected = integrity::fingerprint(&source, cancel)?;
        let mut input = integrity::read_locked(&source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| e.to_string())?;
        use std::io::{Read, Write};
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
        integrity::matches(&target, &expected, cancel)?;
        integrity::matches(&source, &expected, cancel)?;
        drop(input);
        integrity::check(cancel)?;
        fs::remove_file(source).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collect_moves_only_owned_audio_and_preserves_existing_stems() {
        let root = std::env::temp_dir().join(format!("onece-collect-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let from = root.join("stems");
        let to = root.join("attempt");
        fs::create_dir(&from).unwrap();
        fs::create_dir(&to).unwrap();
        for (name, bytes) in [
            ("old.wav", b"old".as_slice()),
            ("batch9.wav", b"neighbor"),
            ("batch.wav", b"new master"),
            ("batch_Kick.wav", b"new stem"),
            ("batch.mp3", b"new mp3"),
        ] {
            fs::write(from.join(name), bytes).unwrap();
        }
        collect_generated_audio(&from, &to, "batch", &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(from.join("old.wav")).unwrap(), b"old");
        assert_eq!(fs::read(from.join("batch9.wav")).unwrap(), b"neighbor");
        assert_eq!(fs::read(to.join("master.wav")).unwrap(), b"new master");
        assert_eq!(fs::read(to.join("master_Kick.wav")).unwrap(), b"new stem");
        assert_eq!(fs::read(to.join("master.mp3")).unwrap(), b"new mp3");
        assert!(!from.join("batch.wav").exists());
        fs::write(from.join("batch.wav"), b"do not replace").unwrap();
        assert!(collect_generated_audio(&from, &to, "batch", &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(from.join("batch.wav")).unwrap(), b"do not replace");
        assert!(collect_generated_audio(&from, &to, "batch", &AtomicBool::new(true)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn startup_recovery_is_bounded_and_does_not_repeat_normal_errors() {
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let mut retries = 0;
        let value = retry_startup(
            &cancel,
            Duration::ZERO,
            || {
                calls += 1;
                if calls < 3 {
                    Err(format!("{STARTUP_NO_AUDIO}early exit"))
                } else {
                    Ok(7)
                }
            },
            || retries += 1,
        )
        .unwrap();
        assert_eq!((value, calls, retries), (7, 3, 2));
        calls = 0;
        let result: Result<(), String> = retry_startup(
            &cancel,
            Duration::ZERO,
            || {
                calls += 1;
                Err(format!("{STARTUP_NO_AUDIO}still failing"))
            },
            || {},
        );
        assert_eq!(calls, 3);
        assert_eq!(result.unwrap_err(), "still failing");
        for error in [
            "Export timed out",
            "Export cancelled",
            "Invalid WAV",
            "Partial audio",
            "Settings restoration failed",
        ] {
            calls = 0;
            let result: Result<(), String> = retry_startup(
                &cancel,
                Duration::ZERO,
                || {
                    calls += 1;
                    Err(error.into())
                },
                || panic!("Unexpected retry"),
            );
            assert_eq!(calls, 1);
            assert_eq!(result.unwrap_err(), error);
        }
    }
    #[test]
    fn cancellation_stops_startup_recovery_before_a_second_launch() {
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let result: Result<(), String> = retry_startup(
            &cancel,
            Duration::from_secs(5),
            || {
                calls += 1;
                Err(format!("{STARTUP_NO_AUDIO}early exit"))
            },
            || cancel.store(true, Ordering::Release),
        );
        assert_eq!(calls, 1);
        assert!(result.unwrap_err().contains("cancelled"));
        let result: Result<(), String> = retry_startup(
            &cancel,
            Duration::ZERO,
            || panic!("Cancelled job launched"),
            || {},
        );
        assert!(result.is_err());
    }
    #[test]
    #[ignore = "Explicit local FL Studio smoke, writes only to a new temporary output directory"]
    fn real_fl_studio_wav_smoke() {
        let source =
            std::env::var("ONECE_EXPORT_SMOKE_PROJECT").expect("Set the explicit source project");
        let root = std::env::temp_dir().join(format!("onece-real-export-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let preserve_name =
            std::env::var("ONECE_EXPORT_SMOKE_PRESERVE_NAME").is_ok_and(|v| v == "1");
        let source_path = Path::new(&source);
        let input = root.join(if preserve_name {
            source_path.parent().unwrap().file_name().unwrap()
        } else {
            std::ffi::OsStr::new("input")
        });
        fs::create_dir(&input).unwrap();
        let project = input.join(if preserve_name {
            source_path.file_name().unwrap()
        } else {
            std::ffi::OsStr::new("smoke.flp")
        });
        fs::copy(&source, &project).unwrap();
        let outputs: Vec<_> = ["mp3", "stems", "projects"]
            .iter()
            .map(|name| {
                let p = root.join(name);
                fs::create_dir(&p).unwrap();
                p
            })
            .collect();
        let config = ExportConfig {
            executable: "C:\\Program Files\\Image-Line\\FL Studio 2024\\FL64.exe".into(),
            mp3_folder: outputs[0].to_string_lossy().into_owned(),
            stems_folder: outputs[1].to_string_lossy().into_owned(),
            projects_folder: outputs[2].to_string_lossy().into_owned(),
            compatibility_mode: std::env::var("ONECE_DIAGNOSTIC_CURRENT_DESKTOP")
                .is_ok_and(|v| v == "1"),
            ..Default::default()
        };
        let stems = std::env::var("ONECE_EXPORT_SMOKE_STEMS").is_ok_and(|v| v == "1");
        let format = std::env::var("ONECE_EXPORT_SMOKE_FORMAT").unwrap_or_else(|_| "both".into());
        assert!(["wav", "mp3", "both"].contains(&format.as_str()));
        let before = fs::read(&project).unwrap();
        let pref = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey("Software\\Image-Line\\FL Studio 24\\General\\WAVRenderForm")
            .unwrap();
        let pref_before: String = pref.get_value("BatchBox").unwrap();
        let cancel = AtomicBool::new(false);
        let cancel_ms = std::env::var("ONECE_EXPORT_SMOKE_CANCEL_MS")
            .ok()
            .map(|v| v.parse::<u64>().unwrap());
        let result = std::thread::scope(|scope| {
            if let Some(ms) = cancel_ms {
                let token = &cancel;
                scope.spawn(move || {
                    std::thread::sleep(Duration::from_millis(ms));
                    token.store(true, Ordering::Release);
                });
            }
            render_audio(&config, &project, &format, stems, &cancel, |stage| {
                println!("Stage: {stage}")
            })
        });
        assert_eq!(fs::read(&project).unwrap(), before, "Source copy changed");
        assert_eq!(
            pref.get_value::<String, _>("BatchBox").unwrap(),
            pref_before,
            "Stems preference was not restored"
        );
        println!("Smoke output retained at {}", root.display());
        println!("Result: {result:?}");
        if cancel_ms.is_some() {
            assert!(result.unwrap_err().to_ascii_lowercase().contains("cancel"));
            assert!(!fs::read_dir(&outputs[1]).unwrap().any(|e| e
                .unwrap()
                .path()
                .join("audio-validated.json")
                .exists()));
        } else {
            assert!(result.is_ok());
        }
    }
    #[test]
    fn refuses_arbitrary_executables_and_relative_paths() {
        assert!(validate_config(&ExportConfig {
            executable: "C:\\Windows\\System32\\cmd.exe".into(),
            ..Default::default()
        })
        .is_err());
        assert!(regular_path(Path::new("relative"), true).is_err());
    }
    #[test]
    fn legacy_root_migrates_without_overwriting_an_explicit_destination() {
        let config: ExportConfig = serde_json::from_str(
            r#"{"outputFolder":"C:\\Exports\\beats","mp3Folder":"D:\\Music"}"#,
        )
        .unwrap();
        let migrated = migrate_config(config);
        assert_eq!(migrated.mp3_folder, "D:\\Music");
        assert_eq!(
            migrated.stems_folder,
            Path::new("C:\\Exports\\beats")
                .join("Stems")
                .to_string_lossy()
        );
        assert_eq!(
            migrated.projects_folder,
            Path::new("C:\\Exports\\beats")
                .join("Projects")
                .to_string_lossy()
        );
        assert!(!serde_json::to_string(&migrated)
            .unwrap()
            .contains("outputFolder"));
        assert!(migrated.compatibility_mode);
        assert!(
            !serde_json::from_str::<ExportConfig>(r#"{"compatibilityMode":false}"#)
                .unwrap()
                .compatibility_mode
        );
    }
    #[test]
    fn destinations_are_independent_existing_directories() {
        let root = std::env::temp_dir().join(format!("onece-destinations-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let paths: Vec<_> = ["audio", "wav-output", "project-archive"]
            .iter()
            .map(|name| {
                let p = root.join(name);
                fs::create_dir(&p).unwrap();
                p
            })
            .collect();
        let config = ExportConfig {
            mp3_folder: paths[0].to_string_lossy().into_owned(),
            stems_folder: paths[1].to_string_lossy().into_owned(),
            projects_folder: paths[2].to_string_lossy().into_owned(),
            ..Default::default()
        };
        let layout = output_layout(&config).unwrap();
        assert_eq!(layout.mp3, fs::canonicalize(&paths[0]).unwrap());
        assert_eq!(
            layout.stems_and_master_wav,
            fs::canonicalize(&paths[1]).unwrap()
        );
        assert_eq!(layout.projects, fs::canonicalize(&paths[2]).unwrap());
        assert!(output_layout(&ExportConfig::default()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
