use chrono::{Datelike, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindowBuilder,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

const TIMER_WINDOW: &str = "production-timer";
const TIMER_TRAY: &str = "production-timer";
const OPEN_DESKTOP: &str = "open-desktop";
const QUIT_DESKTOP: &str = "quit-desktop";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionSession {
    id: String,
    kind: String,
    local_date: String,
    started_at: i64,
    finished_at: i64,
    active_seconds: u64,
    status: String,
    source: String,
    note: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveTimer {
    id: String,
    kind: String,
    local_date: String,
    started_at: i64,
    active_seconds: u64,
    running: bool,
    last_checkpoint: i64,
    interrupted: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimerStore {
    enabled: bool,
    active: Option<ActiveTimer>,
    sessions: Vec<ProductionSession>,
    next_id: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayTotal {
    date: String,
    beat_seconds: u64,
    loop_seconds: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerSnapshot {
    enabled: bool,
    active: Option<ActiveTimer>,
    sessions: Vec<ProductionSession>,
    today: DayTotal,
    week: Vec<DayTotal>,
    beat_average_seconds: u64,
    beat_average_days: usize,
    loop_average_seconds: u64,
    loop_average_days: usize,
}

pub struct ProductionTimerState {
    store: Arc<Mutex<TimerStore>>,
    tray: Mutex<Option<TrayIcon>>,
    path: PathBuf,
}

impl ProductionTimerState {
    pub fn load(app: &AppHandle) -> Result<Self, String> {
        let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join("production-timer.json");
        let mut store = if path.exists() {
            serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Production timer data is invalid: {e}"))?
        } else {
            TimerStore::default()
        };
        if let Some(active) = store.active.as_mut() {
            if active.running {
                active.running = false;
                active.interrupted = true;
            }
        }
        let state = Self {
            store: Arc::new(Mutex::new(store)),
            tray: Mutex::new(None),
            path,
        };
        {
            let store = state.store.lock().map_err(|_| "Timer lock failed")?;
            state.save_locked(&store)?;
        }
        Ok(state)
    }

    fn save_locked(&self, store: &TimerStore) -> Result<(), String> {
        let temporary = self.path.with_extension("json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(&temporary, &self.path).map_err(|e| e.to_string())
    }
}

fn now() -> i64 {
    Utc::now().timestamp()
}
fn local_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}
fn validate_kind(kind: &str) -> Result<(), String> {
    if matches!(kind, "beat" | "loop") {
        Ok(())
    } else {
        Err("Kind must be beat or loop".into())
    }
}
fn checkpoint_at(store: &mut TimerStore, current: i64) {
    if let Some(active) = store.active.as_mut() {
        if active.running {
            let elapsed = current.saturating_sub(active.last_checkpoint);
            if elapsed > 5 {
                active.running = false;
                active.interrupted = true;
                active.last_checkpoint = current;
                return;
            }
            active.active_seconds = active.active_seconds.saturating_add(elapsed as u64);
            active.last_checkpoint = current;
        }
    }
}
fn checkpoint(store: &mut TimerStore) {
    checkpoint_at(store, now());
}

fn week_start(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn snapshot(store: &TimerStore) -> TimerSnapshot {
    let today_date = Local::now().date_naive();
    let start = week_start(today_date);
    let mut totals = BTreeMap::<String, (u64, u64)>::new();
    for offset in 0..7 {
        totals.insert(
            (start + chrono::Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string(),
            (0, 0),
        );
    }
    for session in &store.sessions {
        if let Some(total) = totals.get_mut(&session.local_date) {
            if session.kind == "beat" {
                total.0 += session.active_seconds;
            }
            if session.kind == "loop" {
                total.1 += session.active_seconds;
            }
        }
    }
    if let Some(active) = &store.active {
        if let Some(total) = totals.get_mut(&active.local_date) {
            if active.kind == "beat" {
                total.0 += active.active_seconds;
            }
            if active.kind == "loop" {
                total.1 += active.active_seconds;
            }
        }
    }
    let week: Vec<_> = totals
        .into_iter()
        .map(|(date, (beat_seconds, loop_seconds))| DayTotal {
            date,
            beat_seconds,
            loop_seconds,
        })
        .collect();
    let beat_days: Vec<_> = week.iter().filter(|day| day.beat_seconds > 0).collect();
    let loop_days: Vec<_> = week.iter().filter(|day| day.loop_seconds > 0).collect();
    let today_key = today_date.format("%Y-%m-%d").to_string();
    let today = week
        .iter()
        .find(|day| day.date == today_key)
        .map(|day| DayTotal {
            date: day.date.clone(),
            beat_seconds: day.beat_seconds,
            loop_seconds: day.loop_seconds,
        })
        .unwrap_or(DayTotal {
            date: today_key,
            beat_seconds: 0,
            loop_seconds: 0,
        });
    TimerSnapshot {
        enabled: store.enabled,
        active: store.active.clone(),
        sessions: store.sessions.clone(),
        today,
        beat_average_seconds: if beat_days.is_empty() {
            0
        } else {
            beat_days.iter().map(|d| d.beat_seconds).sum::<u64>() / beat_days.len() as u64
        },
        beat_average_days: beat_days.len(),
        loop_average_seconds: if loop_days.is_empty() {
            0
        } else {
            loop_days.iter().map(|d| d.loop_seconds).sum::<u64>() / loop_days.len() as u64
        },
        loop_average_days: loop_days.len(),
        week,
    }
}

fn timer_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(TIMER_WINDOW) {
        return Ok(window);
    }
    let window = WebviewWindowBuilder::new(
        app,
        TIMER_WINDOW,
        WebviewUrl::App("index.html?view=timer".into()),
    )
    .title("1CE Time Tracker")
    .inner_size(320.0, 158.0)
    .min_inner_size(320.0, 158.0)
    .max_inner_size(320.0, 158.0)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    let target = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = target.hide();
        }
    });
    Ok(window)
}

fn fit_to_monitor(app: &AppHandle, anchor: PhysicalPosition<f64>) -> PhysicalPosition<i32> {
    let monitors = app.available_monitors().unwrap_or_default();
    let monitor = monitors
        .iter()
        .find(|monitor| {
            let p = monitor.position();
            let s = monitor.size();
            anchor.x >= p.x as f64
                && anchor.x < (p.x + s.width as i32) as f64
                && anchor.y >= p.y as f64
                && anchor.y < (p.y + s.height as i32) as f64
        })
        .or_else(|| monitors.first());
    let Some(monitor) = monitor else {
        return PhysicalPosition::new(anchor.x as i32 - 320, anchor.y as i32 - 158);
    };
    let work = monitor.work_area();
    let scale = monitor.scale_factor();
    let size = PhysicalSize::new(
        (320.0 * scale).round() as u32 + 2,
        (158.0 * scale).round() as u32 + 2,
    );
    let monitor_right = monitor.position().x + monitor.size().width as i32;
    let work_right = work.position.x + work.size.width as i32;
    let work_bottom = work.position.y + work.size.height as i32;
    let (preferred_x, preferred_y) = if work_right < monitor_right {
        (
            work_right - size.width as i32 - 8,
            anchor.y as i32 - size.height as i32 / 2,
        )
    } else if work.position.x > monitor.position().x {
        (
            work.position.x + 8,
            anchor.y as i32 - size.height as i32 / 2,
        )
    } else if work.position.y > monitor.position().y {
        (anchor.x as i32 - size.width as i32 / 2, work.position.y + 8)
    } else {
        (
            anchor.x as i32 - size.width as i32 / 2,
            work_bottom - size.height as i32 - 8,
        )
    };
    let max_x = work.position.x + work.size.width.saturating_sub(size.width) as i32;
    let max_y = work.position.y + work.size.height.saturating_sub(size.height) as i32;
    PhysicalPosition::new(
        preferred_x.clamp(work.position.x, max_x),
        preferred_y.clamp(work.position.y, max_y),
    )
}

fn show_window(app: &AppHandle, anchor: PhysicalPosition<f64>) -> Result<(), String> {
    let window = timer_window(app)?;
    window
        .set_position(fit_to_monitor(app, anchor))
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

pub fn show_after_main_hidden(app: AppHandle, anchor: PhysicalPosition<f64>) {
    tauri::async_runtime::spawn(async move {
        let _ = show_window(&app, anchor);
    });
}

fn toggle_window(app: &AppHandle, anchor: PhysicalPosition<f64>) -> Result<(), String> {
    let window = timer_window(app)?;
    if window.is_visible().map_err(|e| e.to_string())? {
        window.hide().map_err(|e| e.to_string())?;
    } else {
        show_window(app, anchor)?;
    }
    Ok(())
}

pub fn sync_tray(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<ProductionTimerState>();
    let enabled = state.store.lock().map_err(|_| "Timer lock failed")?.enabled;
    let mut tray = state.tray.lock().map_err(|_| "Tray lock failed")?;
    if enabled && tray.is_none() {
        let app_handle = app.clone();
        let menu = MenuBuilder::new(app)
            .text(OPEN_DESKTOP, "Open 1CE Desktop")
            .text(QUIT_DESKTOP, "Quit 1CE Desktop")
            .build()
            .map_err(|e| e.to_string())?;
        let icon = app
            .default_window_icon()
            .cloned()
            .ok_or("Application icon unavailable")?;
        *tray = Some(
            TrayIconBuilder::with_id(TIMER_TRAY)
                .icon(icon)
                .tooltip("1CE Production Timer")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    OPEN_DESKTOP => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    QUIT_DESKTOP => {
                        checkpoint_now(app);
                        if session_running(app) {
                            let handle = app.clone();
                            app.dialog()
                                .message("Uma gravação está em andamento. Sair do 1CE Desktop?")
                                .title("Sair do 1CE Desktop")
                                .buttons(MessageDialogButtons::OkCancel)
                                .show(move |confirmed| {
                                    if confirmed {
                                        checkpoint_now(&handle);
                                        handle.exit(0);
                                    }
                                });
                        } else {
                            app.exit(0);
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(move |_tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        position,
                        ..
                    } = event
                    {
                        let app = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = toggle_window(&app, position);
                        });
                    }
                })
                .build(app)
                .map_err(|e| e.to_string())?,
        );
    } else if !enabled {
        if let Some(window) = app.get_webview_window(TIMER_WINDOW) {
            let _ = window.hide();
        }
        tray.take();
    }
    Ok(())
}

fn session_running(app: &AppHandle) -> bool {
    let state = app.state::<ProductionTimerState>();
    state.store.lock().is_ok_and(|store| store.active.is_some())
}

pub fn should_keep_running(app: &AppHandle) -> bool {
    let state = app.state::<ProductionTimerState>();
    state.store.lock().is_ok_and(|store| store.enabled)
}

pub fn start_checkpoint_worker(app: &AppHandle) {
    let state = app.state::<ProductionTimerState>();
    let store = Arc::clone(&state.store);
    let path = state.path.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let Ok(mut value) = store.lock() else {
            continue;
        };
        checkpoint(&mut value);
        let temporary = path.with_extension("json.tmp");
        if let Ok(serialized) = serde_json::to_vec_pretty(&*value) {
            if fs::write(&temporary, serialized).is_ok() {
                let _ = fs::rename(&temporary, &path);
            }
        }
    });
}

pub fn checkpoint_now(app: &AppHandle) {
    let state = app.state::<ProductionTimerState>();
    let Ok(mut store) = state.store.lock() else {
        return;
    };
    checkpoint(&mut store);
    let _ = state.save_locked(&store);
}

#[tauri::command]
pub fn production_timer_snapshot(
    state: State<'_, ProductionTimerState>,
) -> Result<TimerSnapshot, String> {
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    checkpoint(&mut store);
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn set_production_timer_enabled(
    app: AppHandle,
    state: State<'_, ProductionTimerState>,
    enabled: bool,
) -> Result<TimerSnapshot, String> {
    {
        let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
        checkpoint(&mut store);
        store.enabled = enabled;
        if !enabled {
            if let Some(active) = store.active.as_mut() {
                active.running = false;
            }
        }
        state.save_locked(&store)?;
    }
    sync_tray(&app)?;
    production_timer_snapshot(state)
}

#[tauri::command]
pub fn production_timer_start(
    state: State<'_, ProductionTimerState>,
    kind: String,
) -> Result<TimerSnapshot, String> {
    validate_kind(&kind)?;
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    if store.active.is_some() {
        return Err("Finish the current timer before starting another".into());
    }
    let current = now();
    store.next_id += 1;
    store.active = Some(ActiveTimer {
        id: format!("timer-{}-{}", current, store.next_id),
        kind,
        local_date: local_date(),
        started_at: current,
        active_seconds: 0,
        running: true,
        last_checkpoint: current,
        interrupted: false,
    });
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn production_timer_pause(
    state: State<'_, ProductionTimerState>,
) -> Result<TimerSnapshot, String> {
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    checkpoint(&mut store);
    let active = store.active.as_mut().ok_or("No active timer")?;
    active.running = false;
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn production_timer_resume(
    state: State<'_, ProductionTimerState>,
) -> Result<TimerSnapshot, String> {
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    let active = store.active.as_mut().ok_or("No active timer")?;
    active.running = true;
    active.last_checkpoint = now();
    active.interrupted = false;
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn production_timer_finish(
    state: State<'_, ProductionTimerState>,
) -> Result<TimerSnapshot, String> {
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    checkpoint(&mut store);
    let active = store.active.take().ok_or("No active timer")?;
    store.sessions.push(ProductionSession {
        id: active.id,
        kind: active.kind,
        local_date: active.local_date,
        started_at: active.started_at,
        finished_at: now(),
        active_seconds: active.active_seconds,
        status: if active.interrupted {
            "interrupted"
        } else {
            "completed"
        }
        .into(),
        source: "timer".into(),
        note: None,
    });
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn production_timer_add_manual(
    state: State<'_, ProductionTimerState>,
    kind: String,
    local_date: String,
    active_seconds: u64,
    note: Option<String>,
) -> Result<TimerSnapshot, String> {
    validate_kind(&kind)?;
    NaiveDate::parse_from_str(&local_date, "%Y-%m-%d").map_err(|_| "Use a valid local date")?;
    if active_seconds == 0 {
        return Err("Minutes must be greater than zero".into());
    }
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    store.next_id += 1;
    let next_id = store.next_id;
    let noon = Local
        .from_local_datetime(
            &NaiveDate::parse_from_str(&local_date, "%Y-%m-%d")
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .single()
        .ok_or("Invalid local date")?
        .timestamp();
    store.sessions.push(ProductionSession {
        id: format!("manual-{}-{}", noon, next_id),
        kind,
        local_date,
        started_at: noon,
        finished_at: noon,
        active_seconds,
        status: "completed".into(),
        source: "manual".into(),
        note,
    });
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn production_timer_update(
    state: State<'_, ProductionTimerState>,
    id: String,
    kind: String,
    local_date: String,
    active_seconds: u64,
    note: Option<String>,
) -> Result<TimerSnapshot, String> {
    validate_kind(&kind)?;
    NaiveDate::parse_from_str(&local_date, "%Y-%m-%d").map_err(|_| "Use a valid local date")?;
    if active_seconds == 0 {
        return Err("Minutes must be greater than zero".into());
    }
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    let session = store
        .sessions
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or("Session not found")?;
    session.kind = kind;
    session.local_date = local_date;
    session.active_seconds = active_seconds;
    session.note = note;
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn production_timer_delete(
    state: State<'_, ProductionTimerState>,
    id: String,
) -> Result<TimerSnapshot, String> {
    let mut store = state.store.lock().map_err(|_| "Timer lock failed")?;
    let before = store.sessions.len();
    store.sessions.retain(|item| item.id != id);
    if before == store.sessions.len() {
        return Err("Session not found".into());
    }
    state.save_locked(&store)?;
    Ok(snapshot(&store))
}

#[tauri::command]
pub fn production_timer_hide(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(TIMER_WINDOW) {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn production_timer_open_history(app: AppHandle) -> Result<(), String> {
    let main = app
        .get_webview_window("main")
        .ok_or("Main window unavailable")?;
    main.show().map_err(|e| e.to_string())?;
    main.set_focus().map_err(|e| e.to_string())?;
    main.emit("open-production-history", ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn production_timer_show(app: AppHandle) -> Result<(), String> {
    let main = app
        .get_webview_window("main")
        .ok_or("Main window unavailable")?;
    let position = main.outer_position().map_err(|e| e.to_string())?;
    let size = main.outer_size().map_err(|e| e.to_string())?;
    show_window(
        &app,
        PhysicalPosition::new(
            (position.x + size.width as i32) as f64,
            (position.y + size.height as i32) as f64,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(date: &str, kind: &str, seconds: u64) -> ProductionSession {
        ProductionSession {
            id: format!("{date}-{kind}-{seconds}"),
            kind: kind.into(),
            local_date: date.into(),
            started_at: 0,
            finished_at: 0,
            active_seconds: seconds,
            status: "completed".into(),
            source: "manual".into(),
            note: None,
        }
    }
    #[test]
    fn weekly_average_uses_only_days_with_records() {
        let start = week_start(Local::now().date_naive());
        let d0 = start.format("%Y-%m-%d").to_string();
        let d1 = (start + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let store = TimerStore {
            sessions: vec![
                session(&d0, "beat", 600),
                session(&d0, "beat", 300),
                session(&d1, "beat", 900),
                session(&d1, "loop", 1200),
            ],
            ..Default::default()
        };
        let value = snapshot(&store);
        assert_eq!(value.beat_average_days, 2);
        assert_eq!(value.beat_average_seconds, 900);
        assert_eq!(value.loop_average_days, 1);
        assert_eq!(value.loop_average_seconds, 1200);
    }
    #[test]
    fn week_starts_on_monday() {
        let monday = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(week_start(monday + chrono::Duration::days(6)), monday);
    }
    #[test]
    fn checkpoint_counts_short_runs_and_pauses_after_suspension() {
        let mut store = TimerStore {
            active: Some(ActiveTimer {
                id: "a".into(),
                kind: "beat".into(),
                local_date: "2026-11-15".into(),
                started_at: 100,
                active_seconds: 7,
                running: true,
                last_checkpoint: 100,
                interrupted: false,
            }),
            ..Default::default()
        };
        checkpoint_at(&mut store, 103);
        let active = store.active.as_ref().unwrap();
        assert_eq!(active.active_seconds, 10);
        assert!(active.running);
        checkpoint_at(&mut store, 120);
        let active = store.active.as_ref().unwrap();
        assert_eq!(active.active_seconds, 10);
        assert!(!active.running);
        assert!(active.interrupted);
    }
}




