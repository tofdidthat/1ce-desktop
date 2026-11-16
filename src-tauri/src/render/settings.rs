//! FL Studio 24 adapter, verified against the installed export dialog.
//! Scoped, journaled override of BatchBox only. No source FLP edits.
use super::*;
use std::{
    io::Write,
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0},
    Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
};
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
    RegKey,
};
const KEY: &str = "Software\\Image-Line\\FL Studio 24\\General\\WAVRenderForm";

struct Lock(HANDLE);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}
fn lock() -> Result<Lock, String> {
    let name: Vec<u16> = "Local\\1ce-fl24-render-settings"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let handle = CreateMutexW(null(), 0, name.as_ptr());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if !matches!(
            WaitForSingleObject(handle, 0),
            WAIT_OBJECT_0 | WAIT_ABANDONED
        ) {
            CloseHandle(handle);
            return Err("Another 1CE export owns the FL Studio render settings".into());
        }
        Ok(Lock(handle))
    }
}

fn no_open_fl() -> Result<(), String> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err("Cannot inspect existing FL Studio processes".into());
        }
        struct Snapshot(HANDLE);
        impl Drop for Snapshot {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let _snapshot = Snapshot(snapshot);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        if Process32FirstW(snapshot, &mut entry) == 0 {
            return Err("Cannot enumerate FL Studio processes".into());
        }
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_ascii_lowercase();
            if matches!(name.as_str(), "fl.exe" | "fl64.exe" | "fl64 (scaled).exe") {
                return Err("Close other FL Studio windows before automatic export; their settings must not be changed".into());
            }
            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }
    }
    Ok(())
}

fn supported_version(exe: &Path) -> Result<(), String> {
    let path: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut ignored = 0;
        let size = GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored);
        if size == 0 || size > 1024 * 1024 {
            return Err("Cannot identify FL Studio version for automatic stems control".into());
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr() as _) == 0 {
            return Err("Cannot read FL Studio version".into());
        }
        let mut value = null_mut();
        let mut len = 0;
        if VerQueryValueW(
            data.as_ptr() as _,
            [b'\\' as u16, 0].as_ptr(),
            &mut value,
            &mut len,
        ) == 0
            || len < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
            || value.is_null()
        {
            return Err("Invalid executable version information".into());
        }
        let info = std::ptr::read_unaligned(value as *const VS_FIXEDFILEINFO);
        if info.dwSignature != 0xFEEF04BD || info.dwFileVersionMS >> 16 != 24 {
            return Err(
                "Automatic stems control is currently validated for FL Studio 24 only".into(),
            );
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    original: String,
    applied: String,
}
fn valid(record: &Record) -> bool {
    record.version == 1
        && matches!(record.original.as_str(), "0" | "1")
        && matches!(record.applied.as_str(), "0" | "1")
}
fn flush(key: &RegKey) -> Result<(), String> {
    let code = unsafe { windows_sys::Win32::System::Registry::RegFlushKey(key.raw_handle()) };
    if code == 0 {
        Ok(())
    } else {
        Err(format!("Cannot persist render settings: OS error {code}"))
    }
}
fn restore(key: &RegKey, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let record: Record = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Cannot read render-settings recovery record: {e}"))?;
    if !valid(&record) {
        return Err("Invalid render-settings recovery record; no preferences changed".into());
    }
    let current: String = key.get_value("BatchBox").map_err(|e| e.to_string())?;
    if current != record.original && current != record.applied {
        return Err("Render settings changed externally; recovery will not overwrite them".into());
    }
    key.set_value("BatchBox", &record.original)
        .map_err(|e| e.to_string())?;
    flush(key)?;
    let actual: String = key.get_value("BatchBox").map_err(|e| e.to_string())?;
    if actual != record.original {
        return Err("Could not confirm restoration of FL Studio render settings".into());
    }
    fs::remove_file(path).map_err(|e| e.to_string())
}

pub(super) struct SettingsGuard {
    key: RegKey,
    journal: PathBuf,
    _lock: Lock,
}
impl SettingsGuard {
    pub(super) fn begin(exe: &Path, stems: bool) -> Result<Self, String> {
        supported_version(exe)?;
        let mutex = lock()?;
        no_open_fl()?;
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(KEY, KEY_READ | KEY_WRITE)
            .map_err(|e| format!("Cannot open FL Studio 24 render preferences: {e}"))?;
        let directory = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or("APPDATA is unavailable")?
            .join("lol.onece.desktop");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let journal = directory.join("render-settings-recovery.json");
        restore(&key, &journal)?;
        let original: String = key.get_value("BatchBox").map_err(|e| e.to_string())?;
        let record = Record {
            version: 1,
            original,
            applied: if stems { "1" } else { "0" }.into(),
        };
        if !valid(&record) {
            return Err("Unrecognized FL Studio split-track preference; refusing to guess".into());
        }
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&journal)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        let guard = Self {
            key,
            journal,
            _lock: mutex,
        };
        guard
            .key
            .set_value("BatchBox", &record.applied)
            .map_err(|e| e.to_string())?;
        let current: String = guard.key.get_value("BatchBox").map_err(|e| e.to_string())?;
        if current != record.applied {
            return Err("Cannot confirm requested Stems setting".into());
        }
        Ok(guard)
    }
    pub(super) fn finish(self) -> Result<(), String> {
        no_open_fl()?;
        restore(&self.key, &self.journal)
    }
}
impl Drop for SettingsGuard {
    fn drop(&mut self) {
        if no_open_fl().is_ok() {
            let _ = restore(&self.key, &self.journal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Explicit local FL Studio preference roundtrip; close FL Studio first"]
    fn real_preference_on_off_roundtrip() {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).unwrap();
        let before: String = key.get_value("BatchBox").unwrap();
        let exe = Path::new("C:\\Program Files\\Image-Line\\FL Studio 2024\\FL64.exe");
        for (stems, expected) in [(true, "1"), (false, "0")] {
            let guard = SettingsGuard::begin(exe, stems).unwrap();
            let current: String = key.get_value("BatchBox").unwrap();
            assert_eq!(current, expected);
            guard.finish().unwrap();
            assert_eq!(key.get_value::<String, _>("BatchBox").unwrap(), before);
        }
    }
    #[test]
    fn restores_original_after_simulated_interruption_and_rejects_foreign_changes() {
        let name = format!("Software\\1CE-Test-{}", attempt_id());
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&name)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("onece-pref-test-{}", attempt_id()));
        fs::create_dir(&dir).unwrap();
        let journal = dir.join("recovery.json");
        for (old, new) in [("0", "1"), ("1", "0")] {
            key.set_value("BatchBox", &new).unwrap();
            fs::write(
                &journal,
                serde_json::to_vec(&Record {
                    version: 1,
                    original: old.into(),
                    applied: new.into(),
                })
                .unwrap(),
            )
            .unwrap();
            restore(&key, &journal).unwrap();
            let actual: String = key.get_value("BatchBox").unwrap();
            assert_eq!(actual, old);
            assert!(!journal.exists());
        }
        key.set_value("BatchBox", &"external").unwrap();
        fs::write(
            &journal,
            serde_json::to_vec(&Record {
                version: 1,
                original: "0".into(),
                applied: "1".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(restore(&key, &journal).is_err());
        assert_eq!(key.get_value::<String, _>("BatchBox").unwrap(), "external");
        drop(key);
        RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey(&name)
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
