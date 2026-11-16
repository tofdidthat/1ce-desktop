//! Owned Windows process tree on a private desktop. Never attaches to or kills
//! an existing FL Studio. Assign the job atomically at process creation.
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::null,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        StationsAndDesktops::{
            CloseDesktop, CreateDesktopW, DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS,
            DESKTOP_WRITEOBJECTS, HDESK,
        },
        Threading::{
            CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
            InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
            CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTUPINFOEXW,
        },
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
struct Desktop(HDESK);
impl Drop for Desktop {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseDesktop(self.0);
            }
        }
    }
}
struct Attributes(Vec<usize>);
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr() as _);
        }
    }
}

fn wide(s: &OsStr) -> Result<Vec<u16>, String> {
    let mut chars: Vec<_> = s.encode_wide().collect();
    if chars.contains(&0) {
        return Err("A process argument contains a NUL character".into());
    }
    chars.push(0);
    Ok(chars)
}

// Microsoft CRT quoting, not shell quoting. No cmd.exe / PowerShell involved.
pub(super) fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        match ch {
            '\\' => slashes += 1,
            '"' => {
                result.push_str(&"\\".repeat(slashes * 2 + 1));
                result.push('"');
                slashes = 0;
            }
            _ => {
                result.push_str(&"\\".repeat(slashes));
                slashes = 0;
                result.push(ch);
            }
        }
    }
    result.push_str(&"\\".repeat(slashes * 2));
    result.push('"');
    result
}

fn error(stage: &str) -> String {
    format!("{stage}: {}", std::io::Error::last_os_error())
}

#[cfg(test)]
pub(super) fn run(
    executable: &Path,
    args: &[String],
    directory: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<u32, String> {
    run_with_mode(executable, args, directory, cancel, timeout, false)
}

pub(super) fn run_with_mode(
    executable: &Path,
    args: &[String],
    directory: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    compatibility: bool,
) -> Result<u32, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Cancelled before starting".into());
    }
    if timeout.is_zero() {
        return Err("Timeout must be greater than zero".into());
    }
    let application = wide(executable.as_os_str())?;
    let command = std::iter::once(
        executable
            .to_str()
            .ok_or("Executable path must be Unicode")?
            .to_owned(),
    )
    .chain(args.iter().cloned())
    .map(|v| quote(&v))
    .collect::<Vec<_>>()
    .join(" ");
    let mut command = wide(OsStr::new(&command))?;
    if command.len() > 32767 {
        return Err("Export command line is too long".into());
    }
    let cwd = wide(directory.as_os_str())?;
    let mut desktop_name = wide(OsStr::new(&format!("1ce-render-{}", super::attempt_id())))?;
    unsafe {
        let diagnostic_current_desktop = compatibility;
        let desktop = if diagnostic_current_desktop {
            Desktop(std::ptr::null_mut())
        } else {
            Desktop(CreateDesktopW(
                desktop_name.as_ptr(),
                null(),
                null(),
                0,
                DESKTOP_CREATEWINDOW | DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS,
                null(),
            ))
        };
        if !diagnostic_current_desktop && desktop.0.is_null() {
            return Err(error("Create private desktop"));
        }
        let job = Handle(CreateJobObjectW(null(), null()));
        if job.0.is_null() {
            return Err(error("Create render job"));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as _,
            std::mem::size_of_val(&limits) as u32,
        ) == 0
        {
            return Err(error("Set render job limits"));
        }
        // Windows 10+ can attach the owned job before any child code runs,
        // without starting and manually resuming a suspended primary thread.
        // Keep both the attribute storage and job-handle array alive until deletion.
        let jobs = [job.0];
        let mut bytes = 0;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
        if bytes == 0 || bytes > 1024 * 1024 {
            return Err(error("Size process attributes"));
        }
        let mut storage = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
        if InitializeProcThreadAttributeList(storage.as_mut_ptr() as _, 1, 0, &mut bytes) == 0 {
            return Err(error("Initialize process attributes"));
        }
        let mut attributes = Attributes(storage);
        let mut startup: STARTUPINFOEXW = std::mem::zeroed();
        startup.StartupInfo.cb = std::mem::size_of_val(&startup) as u32;
        startup.lpAttributeList = attributes.0.as_mut_ptr() as _;
        if UpdateProcThreadAttribute(
            startup.lpAttributeList,
            0,
            PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
            jobs.as_ptr() as _,
            std::mem::size_of_val(&jobs),
            std::ptr::null_mut(),
            null(),
        ) == 0
        {
            return Err(error("Attach owned job at creation"));
        }
        startup.StartupInfo.lpDesktop = if diagnostic_current_desktop {
            std::ptr::null_mut()
        } else {
            desktop_name.as_mut_ptr()
        };
        if diagnostic_current_desktop {
            startup.StartupInfo.dwFlags = 1;
            startup.StartupInfo.wShowWindow = 7;
        }
        let mut info: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            null(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        ) == 0
        {
            return Err(error("Launch renderer in owned job"));
        }
        drop(attributes);
        let process = Handle(info.hProcess);
        let thread = Handle(info.hThread);
        let started = Instant::now();
        let result = loop {
            if cancel.load(Ordering::Acquire) {
                break Err("Export cancelled; partial output retained for inspection".into());
            }
            if started.elapsed() >= timeout {
                break Err(
                    "Export timed out. A plugin may need attention; partial output retained".into(),
                );
            }
            match WaitForSingleObject(process.0, 100) {
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    if GetExitCodeProcess(process.0, &mut code) == 0 {
                        break Err(error("Read renderer exit code"));
                    }
                    break Ok(code);
                }
                WAIT_TIMEOUT => {}
                _ => break Err(error("Wait for renderer")),
            }
        };
        // Even on success, no bridge/plugin child from this attempt is left behind.
        TerminateJobObject(job.0, 1);
        WaitForSingleObject(process.0, 5000);
        drop(thread);
        drop(process);
        drop(job);
        drop(desktop);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn powershell() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
    }
    fn args(script: &str) -> Vec<String> {
        ["-NoProfile", "-NonInteractive", "-Command", script]
            .map(String::from)
            .to_vec()
    }
    #[test]
    fn quoting_handles_spaces_quotes_and_trailing_slashes() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("x\\"), "\"x\\\\\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
    #[test]
    fn owned_process_exits_and_nonzero_code_is_preserved() {
        assert_eq!(
            run(
                &powershell(),
                &args("exit 7"),
                &std::env::temp_dir(),
                &AtomicBool::new(false),
                Duration::from_secs(20)
            )
            .unwrap(),
            7
        );
    }
    #[test]
    fn timeout_and_cancellation_do_not_kill_unrelated_process() {
        use std::os::windows::process::CommandExt;
        let unrelated = std::process::Command::new(powershell())
            .args(args("Start-Sleep 20"))
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let mut unrelated = unrelated;
        let result = run(
            &powershell(),
            &args("Start-Sleep 20"),
            &std::env::temp_dir(),
            &AtomicBool::new(false),
            Duration::from_millis(200),
        );
        let alive = unrelated.try_wait().unwrap().is_none();
        let _ = unrelated.kill();
        let _ = unrelated.wait();
        assert!(result.unwrap_err().contains("timed out"));
        assert!(alive);
        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                cancel.store(true, Ordering::Release);
            });
            assert!(run(
                &powershell(),
                &args("Start-Sleep 20"),
                &std::env::temp_dir(),
                &cancel,
                Duration::from_secs(20)
            )
            .unwrap_err()
            .contains("cancelled"));
        });
    }
    #[test]
    fn child_process_is_stopped_with_its_render_job() {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let root =
            std::env::temp_dir().join(format!("onece-process-tree-{}", super::super::attempt_id()));
        std::fs::create_dir(&root).unwrap();
        // Fixed test script; paths are passed as arguments to the native runner.
        let script = root.join("child.ps1");
        std::fs::write(&script, "$p = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -NonInteractive -Command Start-Sleep 30' -WindowStyle Hidden -PassThru\n$p.Id | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'child.pid')\nStart-Sleep 30\n").unwrap();
        let result = run(
            &powershell(),
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-File".into(),
                script.to_string_lossy().into_owned(),
            ],
            &root,
            &AtomicBool::new(false),
            Duration::from_secs(3),
        );
        assert!(result.unwrap_err().contains("timed out"));
        let pid: u32 = std::fs::read_to_string(root.join("child.pid"))
            .expect("Child must actually start during the test")
            .trim()
            .parse()
            .unwrap();
        unsafe {
            let child = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
            if !child.0.is_null() {
                let mut code = 259;
                assert_ne!(GetExitCodeProcess(child.0, &mut code), 0);
                // TerminateJobObject is asynchronous: allow the owned child to exit.
                let start = Instant::now();
                while code == 259 && start.elapsed() < Duration::from_secs(5) {
                    std::thread::sleep(Duration::from_millis(25));
                    GetExitCodeProcess(child.0, &mut code);
                }
                assert_ne!(
                    code, 259,
                    "Owned child remained running after job cancellation"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
