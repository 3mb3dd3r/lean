//! HTTP downloads: temp file + rename, hash verification, retries, parallel workers.

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::Value;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::log;

const ATTEMPTS: u32 = 3;
pub const WORKERS: usize = 8;

#[derive(Clone, Debug)]
pub enum Hash {
    None,
    Sha1(String),
    Sha256(String),
}

enum Hasher {
    None,
    Sha1(Sha1),
    Sha256(Sha256),
}

impl Hasher {
    fn new(h: &Hash) -> Self {
        match h {
            Hash::None => Hasher::None,
            Hash::Sha1(_) => Hasher::Sha1(Sha1::new()),
            Hash::Sha256(_) => Hasher::Sha256(Sha256::new()),
        }
    }
    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::None => {}
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
        }
    }
    fn finish(self) -> Option<String> {
        match self {
            Hasher::None => None,
            Hasher::Sha1(h) => Some(hex(&h.finalize())),
            Hasher::Sha256(h) => Some(hex(&h.finalize())),
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn file_sha1(path: &Path) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha1::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// True when `path` exists and looks complete: matching size when known, else matching hash.
/// Files are only ever renamed into place after verification, so this is a cheap resume check.
pub fn is_present(path: &Path, size: Option<u64>, hash: &Hash) -> bool {
    let Ok(meta) = fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    if let Some(s) = size {
        return meta.len() == s;
    }
    match hash {
        Hash::Sha1(want) => file_sha1(path).map(|got| got.eq_ignore_ascii_case(want)).unwrap_or(false),
        _ => true,
    }
}

pub struct Http {
    client: Client,
    pub bytes: AtomicU64,
    pub files: AtomicU64,
}

fn retry<T>(what: &str, mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let mut last = None;
    for attempt in 1..=ATTEMPTS {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                log::warn(format!("{what}: attempt {attempt}/{ATTEMPTS} failed: {e:#}"));
                last = Some(e);
                if attempt < ATTEMPTS {
                    std::thread::sleep(Duration::from_millis(700 * attempt as u64));
                }
            }
        }
    }
    Err(last.unwrap()).with_context(|| format!("{what} failed after {ATTEMPTS} attempts"))
}

impl Http {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("lean-launcher/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(600))
            .build()?;
        Ok(Self { client, bytes: AtomicU64::new(0), files: AtomicU64::new(0) })
    }

    pub fn get_json(&self, url: &str) -> Result<Value> {
        retry(url, || Ok(self.client.get(url).send()?.error_for_status()?.json::<Value>()?))
    }

    pub fn get_text(&self, url: &str) -> Result<String> {
        retry(url, || Ok(self.client.get(url).send()?.error_for_status()?.text()?))
    }

    /// Downloads `url` to `dest` (via `dest.part`), verifies `hash`, retries up to 3 times.
    pub fn download(&self, url: &str, dest: &Path, hash: &Hash) -> Result<()> {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut tmp = dest.as_os_str().to_owned();
        tmp.push(".part");
        let tmp = PathBuf::from(tmp);
        let res = retry(url, || self.download_once(url, dest, &tmp, hash));
        if res.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        res
    }

    fn download_once(&self, url: &str, dest: &Path, tmp: &Path, hash: &Hash) -> Result<()> {
        let mut resp = self.client.get(url).send()?.error_for_status()?;
        let mut file = File::create(tmp).with_context(|| format!("creating {}", tmp.display()))?;
        let mut hasher = Hasher::new(hash);
        let mut buf = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            total += n as u64;
        }
        file.flush()?;
        drop(file);
        if let Some(got) = hasher.finish() {
            let want = match hash {
                Hash::Sha1(w) | Hash::Sha256(w) => w,
                Hash::None => unreachable!(),
            };
            if !got.eq_ignore_ascii_case(want) {
                let _ = fs::remove_file(tmp);
                bail!("checksum mismatch (expected {want}, got {got})");
            }
        }
        fs::rename(tmp, dest).with_context(|| format!("moving into place: {}", dest.display()))?;
        self.bytes.fetch_add(total, Ordering::Relaxed);
        self.files.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Download unless a complete copy is already there.
    pub fn ensure(&self, url: &str, dest: &Path, size: Option<u64>, hash: &Hash) -> Result<()> {
        if is_present(dest, size, hash) {
            return Ok(());
        }
        self.download(url, dest, hash)
    }
}

/// Runs `f` over `items` on `workers` threads; stops handing out work after the first error.
pub fn run_parallel<T: Send, F>(items: Vec<T>, workers: usize, f: F) -> Result<()>
where
    F: Fn(T) -> Result<()> + Sync,
{
    let queue = Mutex::new(items.into_iter().collect::<VecDeque<T>>());
    let failure: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let (queue, failure, f) = (&queue, &failure, &f);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(move || loop {
                if failure.lock().unwrap().is_some() {
                    return;
                }
                let Some(item) = queue.lock().unwrap().pop_front() else { return };
                if let Err(e) = f(item) {
                    failure.lock().unwrap().get_or_insert(e);
                    return;
                }
            });
        }
    });
    let result = failure.lock().unwrap().take();
    match result {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Extract a zip into `dest`, dropping the first `strip` path components and skipping
/// entries whose name starts with any of `exclude`. Existing files of equal size are kept.
pub fn extract_zip(zip_path: &Path, dest: &Path, strip: usize, exclude: &[String]) -> Result<usize> {
    let file = File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).with_context(|| format!("reading {}", zip_path.display()))?;
    let mut written = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let raw_name = entry.name().to_string();
        if exclude.iter().any(|e| raw_name.starts_with(e.as_str())) {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else {
            return Err(anyhow!("unsafe path in archive: {raw_name}"));
        };
        let out_rel: PathBuf = rel.components().skip(strip).collect();
        if out_rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&out_rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::metadata(&out).map(|m| m.len() == entry.size()).unwrap_or(false) {
            continue;
        }
        let mut f = File::create(&out).with_context(|| format!("creating {}", out.display()))?;
        std::io::copy(&mut entry, &mut f)?;
        written += 1;
    }
    Ok(written)
}
