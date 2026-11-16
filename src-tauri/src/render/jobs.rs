use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportJob {
    pub folder: String,
    pub state: String,
    pub message: String,
    pub output: Option<String>,
    pub audio_exported: bool,
    pub upload_ready: bool,
    pub key_analysis: Option<crate::key::Analysis>,
    pub key_progress: Option<u8>,
    pub delivery: Option<super::finalize::Receipt>,
}
#[derive(Default)]
pub struct ExportState {
    jobs: Arc<Mutex<Vec<ExportJob>>>,
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

struct BatchReset {
    running: Arc<AtomicBool>,
    jobs: Arc<Mutex<Vec<ExportJob>>>,
}
fn active(state: &str) -> bool {
    matches!(
        state,
        "queued"
            | "preparing_source"
            | "rendering"
            | "validating"
            | "key_queued"
            | "analyzing"
            | "preparing_delivery"
            | "delivering"
    )
}
fn fail_active(jobs: &Mutex<Vec<ExportJob>>, reason: &str) {
    if let Ok(mut list) = jobs.lock() {
        for job in list.iter_mut().filter(|j| active(&j.state)) {
            job.state = "failed".into();
            job.message = reason.into();
        }
    }
}
impl Drop for BatchReset {
    fn drop(&mut self) {
        fail_active(
            &self.jobs,
            "Export worker stopped unexpectedly; partial files were retained",
        );
        // Publish idle only after all old workers are joined and old job cleanup ends.
        self.running.store(false, Ordering::Release);
    }
}
fn update(jobs: &Mutex<Vec<ExportJob>>, index: usize, stage: &str, message: &str) {
    if let Ok(mut list) = jobs.lock() {
        if let Some(job) = list.get_mut(index) {
            job.state = stage.into();
            job.message = message.into();
        }
    }
}

pub(super) struct Operation {
    running: Arc<AtomicBool>,
    jobs: Arc<Mutex<Vec<ExportJob>>>,
    pub(super) cancel: Arc<AtomicBool>,
}
impl Operation {
    pub(super) fn status(&self, attempt: &Path, state: &str, message: &str, ready: bool) {
        if let Ok(mut list) = self.jobs.lock() {
            if let Some(job) = list
                .iter_mut()
                .find(|j| j.output.as_deref() == Some(attempt.to_string_lossy().as_ref()))
            {
                job.state = state.into();
                job.message = message.into();
                job.upload_ready = ready;
                if !ready {
                    job.delivery = None;
                }
            }
        }
    }
}
impl Operation {
    pub(super) fn receipt(&self, attempt: &Path, receipt: &super::finalize::Receipt) {
        if let Ok(mut list) = self.jobs.lock() {
            if let Some(job) = list
                .iter_mut()
                .find(|j| j.output.as_deref() == Some(attempt.to_string_lossy().as_ref()))
            {
                job.state = "delivered".into();
                job.message = "Final files verified · reviewed key · ready for upload".into();
                job.upload_ready = true;
                job.delivery = Some(receipt.clone());
            }
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        fail_active(
            &self.jobs,
            "Delivery interrupted; staged files retained for resume",
        );
        self.running.store(false, Ordering::Release);
    }
}
impl ExportState {
    pub(super) fn reserve_operation(&self) -> Result<Operation, String> {
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Wait for the current export/delivery to finish")?;
        self.cancel.store(false, Ordering::Release);
        Ok(Operation {
            running: self.running.clone(),
            jobs: self.jobs.clone(),
            cancel: self.cancel.clone(),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportInput {
    folder: String,
    expected_file: String,
    title: String,
}

#[tauri::command]
pub fn export_jobs(state: tauri::State<ExportState>) -> Result<Vec<ExportJob>, String> {
    Ok(state.jobs.lock().map_err(|e| e.to_string())?.clone())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStatus {
    jobs: Vec<ExportJob>,
    running: bool,
}
#[tauri::command]
pub fn export_status(state: tauri::State<ExportState>) -> Result<ExportStatus, String> {
    let list = state.jobs.lock().map_err(|e| e.to_string())?;
    Ok(ExportStatus {
        jobs: list.clone(),
        running: state.running.load(Ordering::Acquire),
    })
}

#[tauri::command]
pub fn cancel_export(state: tauri::State<ExportState>) {
    state.cancel.store(true, Ordering::Release);
}

#[tauri::command]
pub fn start_export(
    app: tauri::AppHandle,
    state: tauri::State<ExportState>,
    inputs: Vec<ExportInput>,
    format: String,
    stems: bool,
    template: String,
) -> Result<(), String> {
    if inputs.is_empty() || inputs.len() > 250 {
        return Err("Choose between 1 and 250 projects".into());
    }
    if !matches!(format.as_str(), "wav" | "mp3" | "both") {
        return Err("Unsupported export format".into());
    }
    if !template.contains("{key}") {
        return Err("Naming template must include {key} for automatic scale naming".into());
    }
    let config = get_export_config(app.clone())?;
    if config.executable.is_empty() {
        return Err("Choose FL Studio in Settings".into());
    }
    validate_config(&config)?;
    output_layout(&config)?;
    let mut sources = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for input in &inputs {
        let plan = preview_export_plan(
            app.clone(),
            input.folder.clone(),
            input.expected_file.clone(),
            format.clone(),
            stems,
        )?;
        if !seen.insert(plan.project.clone()) {
            return Err("A project occurs twice in the queue".into());
        }
        let metadata = fs::metadata(&plan.project).map_err(|e| e.to_string())?;
        sources.push((
            plan.project,
            metadata.len(),
            metadata.modified().map_err(|e| e.to_string())?,
        ));
    }
    if state
        .running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("An export batch is already running".into());
    }
    state.cancel.store(false, Ordering::Release);
    let jobs = state.jobs.clone();
    let running = state.running.clone();
    let cancel = state.cancel.clone();
    let mut guard = match jobs.lock() {
        Ok(guard) => guard,
        Err(e) => {
            state.running.store(false, Ordering::Release);
            return Err(e.to_string());
        }
    };
    *guard = inputs
        .iter()
        .map(|i| ExportJob {
            folder: i.folder.clone(),
            state: "queued".into(),
            message: "Queued".into(),
            output: None,
            audio_exported: false,
            upload_ready: false,
            key_analysis: None,
            key_progress: None,
            delivery: None,
        })
        .collect();
    drop(guard);
    let worker = std::thread::Builder::new()
        .name("1ce-export".into())
        .spawn(move || {
            let _reset = BatchReset {
                running,
                jobs: jobs.clone(),
            };
            let result = run_batch_finish(
                &sources,
                &jobs,
                &cancel,
                stems,
                |index, folder, analysis| {
                    let candidate = analysis
                        .candidate
                        .as_ref()
                        .ok_or("Key inconclusive; choose a scale before final delivery")?;
                    update(
                        &jobs,
                        index,
                        "delivering",
                        "Applying detected scale and delivering named copies?",
                    );
                    let receipt = super::finalize::automatic(
                        &config,
                        super::finalize::Request {
                            attempt: folder.to_path_buf(),
                            title: inputs[index].title.clone(),
                            template: template.clone(),
                            key: super::finalize::ReviewedKey {
                                root: candidate.root.clone(),
                                mode: candidate.mode.clone(),
                                confirmed: true,
                            },
                        },
                        &cancel,
                    )?;
                    if let Ok(mut list) = jobs.lock() {
                        let job = &mut list[index];
                        job.state = "delivered".into();
                        job.message = "100% ? named files delivered with detected scale".into();
                        job.upload_ready = true;
                        job.delivery = Some(receipt);
                    }
                    Ok(())
                },
                |_, source, stage| {
                    #[cfg(windows)]
                    {
                        render_audio(&config, source, &format, stems, &cancel, stage)
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = (&config, source, &format, stage);
                        Err("Export requires Windows".into())
                    }
                },
            );
            if let Err(error) = result {
                fail_active(&jobs, &error);
            }
        });
    if let Err(e) = worker {
        fail_active(&state.jobs, "Could not start export worker");
        state.running.store(false, Ordering::Release);
        return Err(e.to_string());
    }
    Ok(())
}

type Source = (PathBuf, u64, SystemTime);
#[cfg(test)]
fn run_batch(
    sources: &[Source],
    jobs: &Arc<Mutex<Vec<ExportJob>>>,
    cancel: &AtomicBool,
    stems: bool,
    render: impl FnMut(usize, &Path, &dyn Fn(&str)) -> Result<PathBuf, String>,
) -> Result<(), String> {
    run_batch_finish(sources, jobs, cancel, stems, |_, _, _| Ok(()), render)
}
fn run_batch_finish(
    sources: &[Source],
    jobs: &Arc<Mutex<Vec<ExportJob>>>,
    cancel: &AtomicBool,
    stems: bool,
    finish: impl Fn(usize, &Path, &crate::key::Analysis) -> Result<(), String> + Sync,
    mut render: impl FnMut(usize, &Path, &dyn Fn(&str)) -> Result<PathBuf, String>,
) -> Result<(), String> {
    super::pipeline::run(
        sources.len(),
        cancel,
        |index| {
            let (source, size, modified) = &sources[index];
            let unchanged = fs::metadata(source)
                .is_ok_and(|m| m.len() == *size && m.modified().ok() == Some(*modified));
            let newest = source.parent().and_then(|folder| {
                crate::scanner::inspect_folder(folder, crate::naming::DEFAULT_TEMPLATE).project_file
            });
            if !unchanged || newest.as_deref() != source.file_name().and_then(|s| s.to_str()) {
                update(
                    jobs,
                    index,
                    "failed",
                    "Project changed or a newer FLP appeared after the batch started; scan again",
                );
                return None;
            }
            update(jobs, index, "rendering", "Rendering in FL Studio…");
            match render(index, source, &|stage| {
                if stage == "retrying" {
                    update(
                        jobs,
                        index,
                        "rendering",
                        "FL Studio stopped during startup; retrying safely…",
                    )
                } else {
                    update(
                        jobs,
                        index,
                        stage,
                        match stage {
                            "preparing_source" => "Preparing protected working copy...",
                            "rendering" => "Rendering in FL Studio...",
                            _ => "Checking generated audio...",
                        },
                    )
                }
            }) {
                Ok(folder) => {
                    if let Ok(mut list) = jobs.lock() {
                        let job = &mut list[index];
                        job.state = "key_queued".into();
                        job.audio_exported = true;
                        job.key_progress = Some(0);
                        job.message = "Audio validated · waiting for Find Key".into();
                        job.output = Some(folder.to_string_lossy().into_owned());
                    }
                    Some(folder)
                }
                Err(error) => {
                    update(
                        jobs,
                        index,
                        if cancel.load(Ordering::Acquire) {
                            "cancelled"
                        } else {
                            "failed"
                        },
                        &error,
                    );
                    None
                }
            }
        },
        |index, folder| {
            update(
                jobs,
                index,
                "analyzing",
                "Find Key: 0% · analyzing validated master…",
            );
            let analysis = analyze_exported_master_progress(&folder, stems, cancel, |progress| {
                if let Ok(mut list) = jobs.lock() {
                    let job = &mut list[index];
                    job.key_progress = Some(progress.min(99));
                    job.message = format!(
                        "Find Key: {}% · analyzing validated master…",
                        progress.min(99)
                    );
                }
            });
            match analysis {
                Ok((result, save_error)) => {
                    let save_ok = save_error.is_none();
                    if let Ok(mut list) = jobs.lock() {
                        let job = &mut list[index];
                        job.key_progress = Some(100);
                        job.state = if save_error.is_some() {
                            "key_failed"
                        } else {
                            "exported"
                        }
                        .into();
                        job.message = if let Some(error) = save_error {
                            format!("Audio validated · key result could not be saved: {error}")
                        } else if result.candidate.is_some() {
                            "Find Key 100% · estimate requires review; final naming pending".into()
                        } else {
                            "Find Key 100% · inconclusive; final naming pending".into()
                        };
                        job.key_analysis = Some(result.clone());
                    }
                    if save_ok {
                        if let Err(error) = finish(index, &folder, &result) {
                            update(
                                jobs,
                                index,
                                if cancel.load(Ordering::Acquire) {
                                    "cancelled"
                                } else {
                                    "delivery_failed"
                                },
                                &error,
                            );
                        }
                    }
                }
                Err(error) => update(
                    jobs,
                    index,
                    if cancel.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "key_failed"
                    },
                    &format!("Audio validated · {error}; final naming pending"),
                ),
            }
        },
        |index, event| match event {
            super::pipeline::Event::CancelledBeforeRender => {
                update(jobs, index, "cancelled", "Cancelled before rendering")
            }
            super::pipeline::Event::CancelledBeforeAnalysis => update(
                jobs,
                index,
                "cancelled",
                "Audio validated · Find Key cancelled before analysis",
            ),
            super::pipeline::Event::AnalysisPanicked => update(
                jobs,
                index,
                "key_failed",
                "Audio validated · key worker stopped on this beat; result not trusted",
            ),
        },
    )
}

#[cfg(test)]
fn analyze_exported_master(
    folder: &Path,
    stems: bool,
    cancel: &AtomicBool,
) -> Result<(crate::key::Analysis, Option<String>), String> {
    analyze_exported_master_progress(folder, stems, cancel, |_| {})
}
fn analyze_exported_master_progress(
    folder: &Path,
    stems: bool,
    cancel: &AtomicBool,
    on_progress: impl Fn(u8),
) -> Result<(crate::key::Analysis, Option<String>), String> {
    let master = folder.join(if stems {
        "master_Master.wav"
    } else {
        "master.wav"
    });
    let result = crate::key::analyze_with_progress(&master, cancel, on_progress)?;
    if cancel.load(Ordering::Acquire) {
        return Err("Key analysis cancelled before saving".into());
    }
    let error = save_key_analysis(folder, &result).err();
    Ok((result, error))
}

fn save_key_analysis(folder: &Path, result: &crate::key::Analysis) -> Result<(), String> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(folder.join("key-analysis.json"))
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

pub(super) fn validate_mp3(path: &Path) -> Result<(), String> {
    use symphonia::core::{
        codecs::DecoderOptions, errors::Error, formats::FormatOptions, io::MediaSourceStream,
        meta::MetadataOptions, probe::Hint,
    };
    let source = fs::File::open(path).map_err(|e| format!("Missing MP3: {e}"))?;
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Invalid MP3: {e}"))?
        .format;
    let track = format.default_track().ok_or("MP3 has no audio track")?;
    if track.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_MP3 {
        return Err("Expected MPEG Layer III audio".into());
    }
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let mut frames = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Unreadable MP3 packet: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("MP3 decode failed: {e}"))?;
        frames += decoded.frames();
    }
    if frames == 0 {
        return Err("MP3 has no decodable audio".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(folder: String) -> ExportJob {
        ExportJob {
            folder,
            state: "queued".into(),
            message: "Queued".into(),
            output: None,
            audio_exported: false,
            upload_ready: false,
            key_analysis: None,
            key_progress: None,
            delivery: None,
        }
    }
    fn test_sources(root: &Path, count: usize, bytes: &[u8]) -> Vec<Source> {
        (0..count)
            .map(|i| {
                let folder = root.join(format!("input-{i}"));
                fs::create_dir(&folder).unwrap();
                let path = folder.join("smoke.flp");
                fs::write(&path, bytes).unwrap();
                let meta = fs::metadata(&path).unwrap();
                (path, meta.len(), meta.modified().unwrap())
            })
            .collect()
    }
    #[test]
    fn batch_analysis_failure_is_isolated_and_completed_audio_remains_valid() {
        let root = std::env::temp_dir().join(format!("onece-pipeline-fixture-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let sources = test_sources(&root, 3, b"source bytes must not change");
        let jobs = Arc::new(Mutex::new(
            sources
                .iter()
                .map(|s| row(s.0.parent().unwrap().to_string_lossy().into_owned()))
                .collect(),
        ));
        let running = Arc::new(AtomicBool::new(true));
        let reset = BatchReset {
            running: running.clone(),
            jobs: jobs.clone(),
        };
        let samples: Vec<i16> = (0..11025 * 4)
            .map(|i| {
                let t = i as f32 / 11025.;
                ([261.63f32, 329.63, 392.]
                    .iter()
                    .map(|f| (std::f32::consts::TAU * f * t).sin())
                    .sum::<f32>()
                    * 5000.) as i16
            })
            .collect();
        let data: Vec<u8> = samples.iter().flat_map(|x| x.to_le_bytes()).collect();
        let mut wav = b"RIFF".to_vec();
        wav.extend((36 + data.len() as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(11025u32.to_le_bytes());
        wav.extend(22050u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((data.len() as u32).to_le_bytes());
        wav.extend(data);
        let finished = Mutex::new(Vec::new());
        run_batch_finish(
            &sources,
            &jobs,
            &AtomicBool::new(false),
            false,
            |index, folder, analysis| {
                assert!(folder.join("key-analysis.json").exists());
                assert!(analysis.candidate.is_some());
                finished.lock().unwrap().push(index);
                Ok(())
            },
            |index, _, _| {
                let output = root.join(format!("output-{index}"));
                fs::create_dir(&output).unwrap();
                // Model damage between validated render and analysis for one row.
                fs::write(
                    output.join("master.wav"),
                    if index == 1 {
                        b"damaged WAV".as_slice()
                    } else {
                        wav.as_slice()
                    },
                )
                .unwrap();
                Ok(output)
            },
        )
        .unwrap();
        assert_eq!(
            *finished.lock().unwrap(),
            vec![0, 2],
            "Only successfully analyzed and saved beats can enter automatic naming"
        );
        assert!(running.load(Ordering::Acquire));
        drop(reset);
        assert!(!running.load(Ordering::Acquire));
        let list = jobs.lock().unwrap();
        assert_eq!(
            list.iter().map(|j| j.state.as_str()).collect::<Vec<_>>(),
            vec!["exported", "key_failed", "exported"]
        );
        for (index, job) in list.iter().enumerate() {
            assert!(job.audio_exported);
            assert!(!job.upload_ready);
            assert_eq!(
                fs::read(&sources[index].0).unwrap(),
                b"source bytes must not change"
            );
            if index != 1 {
                assert_eq!(job.key_progress, Some(100));
                assert!(!job.key_analysis.as_ref().unwrap().auto_rename_allowed);
                assert!(root
                    .join(format!("output-{index}/key-analysis.json"))
                    .exists());
            } else {
                assert_ne!(job.key_progress, Some(100));
                assert!(job.key_analysis.is_none());
            }
        }
    }
    #[test]
    #[ignore = "Real two-project FL/DSP overlap smoke; copies inputs and retains only temporary outputs"]
    fn real_overlapped_export_batch() {
        let source = std::env::var("ONECE_EXPORT_SMOKE_PROJECT")
            .expect("Provide explicit disposable source FLP");
        let original = fs::read(&source).unwrap();
        let root = std::env::temp_dir().join(format!("onece-overlap-real-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let sources = test_sources(&root, 2, &original);
        let jobs = Arc::new(Mutex::new(
            sources
                .iter()
                .map(|s| row(s.0.parent().unwrap().to_string_lossy().into_owned()))
                .collect(),
        ));
        let paths: Vec<_> = ["mp3", "stems", "projects"]
            .iter()
            .map(|name| {
                let p = root.join(name);
                fs::create_dir(&p).unwrap();
                p
            })
            .collect();
        let config = ExportConfig {
            executable: "C:\\Program Files\\Image-Line\\FL Studio 2024\\FL64.exe".into(),
            mp3_folder: paths[0].to_string_lossy().into_owned(),
            stems_folder: paths[1].to_string_lossy().into_owned(),
            projects_folder: paths[2].to_string_lossy().into_owned(),
            compatibility_mode: true,
            ..Default::default()
        };
        let saw_analysis = AtomicBool::new(false);
        let saw_done = AtomicBool::new(false);
        let cancel = AtomicBool::new(false);
        let result = run_batch(&sources, &jobs, &cancel, false, |index, input, stage| {
            if index == 0 {
                return render_audio(&config, input, "wav", false, &cancel, stage);
            }
            let rendering = AtomicBool::new(true);
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    while rendering.load(Ordering::Acquire) {
                        let state = jobs.lock().unwrap()[0].state.clone();
                        if state == "analyzing" {
                            saw_analysis.store(true, Ordering::Release);
                        }
                        if state == "exported" {
                            saw_done.store(true, Ordering::Release);
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                });
                struct StopObserver<'a>(&'a AtomicBool);
                impl Drop for StopObserver<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let stop = StopObserver(&rendering);
                let outcome = render_audio(&config, input, "wav", false, &cancel, stage);
                drop(stop);
                outcome
            })
        });
        println!("Overlap output retained at {}", root.display());
        println!(
            "Saw analysis during next render: {}, saw first result before second render ended: {}",
            saw_analysis.load(Ordering::Acquire),
            saw_done.load(Ordering::Acquire)
        );
        result.unwrap();
        let list = jobs.lock().unwrap();
        for (index, job) in list.iter().enumerate() {
            assert_eq!(job.state, "exported", "{}", job.message);
            assert_eq!(job.key_progress, Some(100));
            assert!(job.audio_exported);
            assert!(!job.upload_ready);
            assert!(job.key_analysis.is_some());
            assert_eq!(fs::read(&sources[index].0).unwrap(), original);
        }
        assert!(saw_analysis.load(Ordering::Acquire));
        assert!(saw_done.load(Ordering::Acquire));
        assert_eq!(fs::read(source).unwrap(), original);
    }
    #[test]
    #[ignore = "Explicit real WAV integration; writes only to a new temporary test folder"]
    fn real_exported_key_result_preserved() {
        let source =
            std::env::var("ONECE_KEY_SMOKE_WAV").expect("Provide an existing validated master WAV");
        let root = std::env::temp_dir().join(format!("onece-key-integration-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let wav = root.join("master_Master.wav");
        fs::copy(source, &wav).unwrap();
        let before = fs::read(&wav).unwrap();
        fs::write(
            root.join("audio-validated.json"),
            b"audio-marker-must-remain-unchanged",
        )
        .unwrap();
        let (result, error) =
            analyze_exported_master(&root, true, &AtomicBool::new(false)).unwrap();
        assert!(error.is_none());
        let saved: crate::key::Analysis =
            serde_json::from_slice(&fs::read(root.join("key-analysis.json")).unwrap()).unwrap();
        assert_eq!(saved.status, result.status);
        assert_eq!(
            saved.audio_path,
            wav.canonicalize().unwrap().to_string_lossy()
        );
        assert!(!saved.auto_rename_allowed);
        assert_eq!(
            fs::read(root.join("audio-validated.json")).unwrap(),
            b"audio-marker-must-remain-unchanged"
        );
        assert_eq!(fs::read(&wav).unwrap(), before);
        assert!(analyze_exported_master(&root, true, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled"));
        assert!(
            save_key_analysis(&root, &result).is_err(),
            "Must never overwrite an existing result"
        );
        println!("Key integration output retained at {}", root.display());
    }
    #[test]
    fn missing_or_fake_mp3_cannot_count_as_an_export() {
        let root = std::env::temp_dir().join(format!("onece-mp3-check-{}", attempt_id()));
        fs::create_dir(&root).unwrap();
        let path = root.join("master.mp3");
        assert!(validate_mp3(&path).is_err());
        fs::write(&path, b"this is not encoded audio").unwrap();
        assert!(validate_mp3(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
