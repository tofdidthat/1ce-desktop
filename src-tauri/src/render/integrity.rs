use super::*;
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    pub file: String,
    pub fingerprint: Fingerprint,
}
pub(super) fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Delivery cancelled; staged files retained for retry".into())
    } else {
        Ok(())
    }
}
pub(super) fn read_locked(path: &Path) -> Result<fs::File, String> {
    regular_path(path, false)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    options.open(path).map_err(|e| e.to_string())
}
pub(super) fn fingerprint(path: &Path, cancel: &AtomicBool) -> Result<Fingerprint, String> {
    check(cancel)?;
    let mut file = read_locked(path)?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if before.len() > 128 * 1024 * 1024 * 1024 {
        return Err("Files larger than 128 GiB are not supported for delivery".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut bytes = 0;
    loop {
        check(cancel)?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        bytes += n as u64;
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if bytes != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("File changed while hashing".into());
    }
    Ok(Fingerprint {
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}
pub(super) fn matches(
    path: &Path,
    expected: &Fingerprint,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if fingerprint(path, cancel)? != *expected {
        return Err(format!(
            "File changed or checksum mismatch: {}",
            path.display()
        ));
    }
    Ok(())
}
