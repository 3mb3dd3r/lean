//! Preparation: Java 8, version JSON and jar, libraries, natives, assets, client jar.
//! Every step is skipped when its files are already in place.

use crate::download::{extract_zip, file_sha1, is_present, run_parallel, Hash, Http, WORKERS};
use crate::log;
use crate::paths::Paths;
use crate::settings::Settings;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const JAVA_URL: &str = "https://api.adoptium.net/v3/assets/latest/8/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse";
const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const ASSET_BASE: &str = "https://resources.download.minecraft.net";
const MOJANG_LIBS: &str = "https://libraries.minecraft.net/";
const MAVEN_CENTRAL: &str = "https://repo1.maven.org/maven2/";
const TOTAL_STEPS: usize = 5;

// ---------------------------------------------------------------- progress

/// Shared between the worker thread and the UI.
#[derive(Default)]
pub struct Progress {
    inner: Mutex<Inner>,
}

#[derive(Default, Clone)]
struct Inner {
    step: usize,
    sub: f32,
    status: String,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn reset(&self) {
        *self.inner.lock().unwrap() = Inner::default();
    }
    fn begin(&self, step: usize, status: &str) {
        log::info(status);
        *self.inner.lock().unwrap() = Inner { step, sub: 0.0, status: status.to_string() };
    }
    fn set_sub(&self, sub: f32) {
        self.inner.lock().unwrap().sub = sub.clamp(0.0, 1.0);
    }
    pub fn status(&self, status: &str) {
        log::info(status);
        let mut i = self.inner.lock().unwrap();
        i.status = status.to_string();
    }
    /// (fraction 0..1, status text)
    pub fn snapshot(&self) -> (f32, String) {
        let i = self.inner.lock().unwrap().clone();
        (((i.step as f32 + i.sub) / TOTAL_STEPS as f32).clamp(0.0, 1.0), i.status)
    }
}

// ---------------------------------------------------------------- library plan (pure)

#[derive(Clone, Debug, PartialEq)]
pub struct LibFile {
    /// Maven-style path with `/` separators, relative to `libraries\`.
    pub rel: String,
    pub url: String,
    pub sha1: Option<String>,
    /// Extras without a known hash: fetch `<url>.sha1` and verify against it.
    pub sha1_url: Option<String>,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeFile {
    pub file: LibFile,
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LibPlan {
    pub files: Vec<LibFile>,
    pub natives: Vec<NativeFile>,
    /// Classpath entries (relative paths), in order, ending with launchwrapper and asm-all.
    pub classpath: Vec<String>,
}

/// Mojang library rules for Windows: no rules means allowed; otherwise the last matching rule wins.
pub fn rules_allow(rules: Option<&Value>) -> bool {
    let Some(rules) = rules.and_then(Value::as_array) else { return true };
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        let applies = match rule.get("os") {
            None => true,
            Some(os) => os.get("name").and_then(Value::as_str).map(|n| n == "windows").unwrap_or(true),
        } && rule.get("features").is_none();
        if applies {
            allowed = rule.get("action").and_then(Value::as_str) == Some("allow");
        }
    }
    allowed
}

fn maven_rel(group: &str, artifact: &str, version: &str) -> String {
    format!("{}/{artifact}/{version}/{artifact}-{version}.jar", group.replace('.', "/"))
}

fn extra_lib(base_url: &str, group: &str, artifact: &str, version: &str) -> LibFile {
    let rel = maven_rel(group, artifact, version);
    let url = format!("{base_url}{rel}");
    LibFile { rel, sha1_url: Some(format!("{url}.sha1")), url, sha1: None, size: 0 }
}

fn artifact_file(v: &Value) -> Option<LibFile> {
    Some(LibFile {
        rel: v.get("path")?.as_str()?.to_string(),
        url: v.get("url")?.as_str()?.to_string(),
        sha1: v.get("sha1").and_then(Value::as_str).map(str::to_string),
        sha1_url: None,
        size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
    })
}

pub fn plan_libraries(version: &Value) -> Result<LibPlan> {
    let mut plan = LibPlan::default();
    let libs = version
        .get("libraries")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("version JSON has no libraries"))?;
    let launchwrapper = extra_lib(MOJANG_LIBS, "net.minecraft", "launchwrapper", "1.12");
    let asm_all = extra_lib(MAVEN_CENTRAL, "org.ow2.asm", "asm-all", "5.0.3");
    let mut seen = HashSet::new();
    for lib in libs {
        if !rules_allow(lib.get("rules")) {
            continue;
        }
        let name = lib.get("name").and_then(Value::as_str).unwrap_or("");
        let ga: Vec<&str> = name.split(':').collect();
        // Our own extras take over these two.
        if name.starts_with("net.minecraft:launchwrapper:") || name.starts_with("org.ow2.asm:asm-all:") {
            continue;
        }
        let downloads = lib.get("downloads");
        let artifact = if name == "com.google.guava:guava:15.0" {
            Some(extra_lib(MAVEN_CENTRAL, "com.google.guava", "guava", "17.0"))
        } else {
            downloads.and_then(|d| d.get("artifact")).and_then(artifact_file)
        };
        if let Some(a) = artifact {
            if seen.insert(a.rel.clone()) {
                plan.classpath.push(a.rel.clone());
                plan.files.push(a);
            }
        } else if ga.len() < 3 && downloads.is_none() {
            bail!("library {name:?} has no download information");
        }
        if let Some(classifier) = lib.get("natives").and_then(|n| n.get("windows")).and_then(Value::as_str) {
            let classifier = classifier.replace("${arch}", "64");
            let file = downloads
                .and_then(|d| d.get("classifiers"))
                .and_then(|c| c.get(&classifier))
                .and_then(artifact_file)
                .ok_or_else(|| anyhow!("library {name} has no native classifier {classifier}"))?;
            let exclude = lib
                .get("extract")
                .and_then(|e| e.get("exclude"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            plan.files.push(file.clone());
            plan.natives.push(NativeFile { file, exclude });
        }
    }
    for extra in [launchwrapper, asm_all] {
        plan.classpath.push(extra.rel.clone());
        plan.files.push(extra);
    }
    Ok(plan)
}

// ---------------------------------------------------------------- results

#[derive(Debug, Default)]
pub struct Prepared {
    /// Library classpath entries (relative to `libraries\`), see [`LibPlan::classpath`].
    pub lib_classpath: Vec<String>,
    pub optifine: Option<PathBuf>,
    pub client_jar_source: PathBuf,
    pub library_count: usize,
    pub native_count: usize,
    pub asset_count: usize,
    pub downloaded_bytes: u64,
    pub downloaded_files: u64,
}

// ---------------------------------------------------------------- steps

pub fn prepare(paths: &Paths, settings: &Settings, pg: &Progress) -> Result<Prepared> {
    paths.ensure_dirs()?;
    let http = Http::new()?;
    let mut out = Prepared::default();

    pg.begin(0, "Checking Java 8");
    ensure_java(paths, &http, pg)?;

    pg.begin(1, "Checking Minecraft 1.7.10");
    let version = ensure_version(paths, &http)?;

    pg.begin(2, "Checking libraries");
    let plan = plan_libraries(&version)?;
    ensure_libraries(paths, &http, &plan, pg)?;
    out.library_count = plan.files.len();
    out.native_count = plan.natives.len();
    out.lib_classpath = plan.classpath.clone();

    pg.begin(3, "Checking assets");
    out.asset_count = ensure_assets(paths, &http, &version, pg)?;

    pg.begin(4, "Checking Lean client");
    out.client_jar_source = install_client_jar(paths, settings)?;
    if settings.use_optifine {
        out.optifine = Some(find_optifine(paths).ok_or_else(|| {
            anyhow!("OptiFine is turned on but no jar is imported; use Import OptiFine first")
        })?);
    }

    out.downloaded_bytes = http.bytes.load(Ordering::Relaxed);
    out.downloaded_files = http.files.load(Ordering::Relaxed);
    pg.status("Ready");
    Ok(out)
}

fn ensure_java(paths: &Paths, http: &Http, pg: &Progress) -> Result<()> {
    if paths.javaw().exists() {
        return Ok(());
    }
    pg.status("Downloading Java 8");
    let info = http.get_json(JAVA_URL)?;
    let pkg = info
        .get(0)
        .and_then(|v| v.get("binary"))
        .and_then(|v| v.get("package"))
        .ok_or_else(|| anyhow!("unexpected Adoptium response"))?;
    let link = pkg.get("link").and_then(Value::as_str).ok_or_else(|| anyhow!("Adoptium response has no link"))?;
    let sum = pkg.get("checksum").and_then(Value::as_str).ok_or_else(|| anyhow!("Adoptium response has no checksum"))?;

    let zip_path = paths.runtime_dir().join("java8.zip");
    http.download(link, &zip_path, &Hash::Sha256(sum.to_string()))?;

    pg.status("Unpacking Java 8");
    let tmp = paths.runtime_dir().join("java8.extract");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    extract_zip(&zip_path, &tmp, 1, &[])?;
    if paths.java_dir().exists() {
        fs::remove_dir_all(paths.java_dir())?;
    }
    fs::rename(&tmp, paths.java_dir()).context("installing the Java runtime")?;
    let _ = fs::remove_file(&zip_path);
    if !paths.javaw().exists() {
        bail!("Java archive did not contain bin\\javaw.exe");
    }
    Ok(())
}

fn ensure_version(paths: &Paths, http: &Http) -> Result<Value> {
    let jar = paths.version_jar();
    let json_path = paths.version_json();
    // Only ask Mojang for the manifest when something is missing or unverified.
    let manifest = http.get_json(MANIFEST_URL)?;
    let entry = manifest
        .get("versions")
        .and_then(Value::as_array)
        .and_then(|v| v.iter().find(|e| e.get("id").and_then(Value::as_str) == Some("1.7.10")))
        .ok_or_else(|| anyhow!("version 1.7.10 not found in Mojang's manifest"))?;
    let url = entry.get("url").and_then(Value::as_str).ok_or_else(|| anyhow!("manifest entry has no url"))?;
    let sha1 = entry.get("sha1").and_then(Value::as_str).ok_or_else(|| anyhow!("manifest entry has no sha1"))?;
    http.ensure(url, &json_path, None, &Hash::Sha1(sha1.to_string()))?;
    let version: Value = serde_json::from_slice(&fs::read(&json_path)?).context("parsing 1.7.10.json")?;

    let client = version
        .get("downloads")
        .and_then(|d| d.get("client"))
        .ok_or_else(|| anyhow!("1.7.10.json has no downloads.client"))?;
    let curl = client.get("url").and_then(Value::as_str).ok_or_else(|| anyhow!("client download has no url"))?;
    let csha = client.get("sha1").and_then(Value::as_str).ok_or_else(|| anyhow!("client download has no sha1"))?;
    let csize = client.get("size").and_then(Value::as_u64);
    http.ensure(curl, &jar, csize, &Hash::Sha1(csha.to_string()))?;
    Ok(version)
}

fn fetch_lib(paths: &Paths, http: &Http, f: &LibFile) -> Result<()> {
    let dest = paths.lib(&f.rel);
    let size = (f.size > 0).then_some(f.size);
    let hash = if let Some(s) = &f.sha1 {
        Hash::Sha1(s.clone())
    } else if let Some(u) = &f.sha1_url {
        if dest.is_file() {
            return Ok(()); // renamed into place only after verification
        }
        let text = http.get_text(u)?;
        let sum = text.split_whitespace().next().unwrap_or("").to_string();
        if sum.len() != 40 {
            bail!("bad checksum file at {u}");
        }
        Hash::Sha1(sum)
    } else {
        Hash::None
    };
    http.ensure(&f.url, &dest, size, &hash)
}

fn ensure_libraries(paths: &Paths, http: &Http, plan: &LibPlan, pg: &Progress) -> Result<()> {
    let total = plan.files.len();
    let done = AtomicUsize::new(0);
    run_parallel(plan.files.iter().collect(), WORKERS, |f| {
        fetch_lib(paths, http, f)?;
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        pg.set_sub(n as f32 / total as f32);
        Ok(())
    })?;
    pg.status("Extracting natives");
    for n in &plan.natives {
        extract_zip(&paths.lib(&n.file.rel), &paths.natives_dir(), 0, &n.exclude)
            .with_context(|| format!("extracting {}", n.file.rel))?;
    }
    Ok(())
}

fn ensure_assets(paths: &Paths, http: &Http, version: &Value, pg: &Progress) -> Result<usize> {
    let idx = version.get("assetIndex").ok_or_else(|| anyhow!("1.7.10.json has no assetIndex"))?;
    let url = idx.get("url").and_then(Value::as_str).ok_or_else(|| anyhow!("assetIndex has no url"))?;
    let sha1 = idx.get("sha1").and_then(Value::as_str).ok_or_else(|| anyhow!("assetIndex has no sha1"))?;
    http.ensure(url, &paths.asset_index(), None, &Hash::Sha1(sha1.to_string()))?;
    let index: Value = serde_json::from_slice(&fs::read(paths.asset_index())?).context("parsing asset index")?;
    let objects = index
        .get("objects")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("asset index has no objects"))?;

    let mut seen = HashSet::new();
    let mut all: Vec<(String, u64)> = Vec::new();
    for o in objects.values() {
        let hash = o.get("hash").and_then(Value::as_str).ok_or_else(|| anyhow!("asset without hash"))?;
        if hash.len() < 2 {
            bail!("bad asset hash {hash:?}");
        }
        let size = o.get("size").and_then(Value::as_u64).unwrap_or(0);
        if seen.insert(hash.to_string()) {
            all.push((hash.to_string(), size));
        }
    }
    let total_objects = all.len();
    let missing: Vec<(String, u64)> = all
        .into_iter()
        .filter(|(h, s)| !is_present(&paths.asset_object(h), Some(*s), &Hash::None))
        .collect();
    if missing.is_empty() {
        return Ok(total_objects);
    }
    pg.status(&format!("Downloading {} assets", missing.len()));
    let total = missing.len();
    let done = AtomicUsize::new(0);
    run_parallel(missing, WORKERS, |(hash, _size)| {
        let url = format!("{ASSET_BASE}/{}/{}", &hash[..2], hash);
        http.download(&url, &paths.asset_object(&hash), &Hash::Sha1(hash.clone()))?;
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        pg.set_sub(n as f32 / total as f32);
        if n % 100 == 0 || n == total {
            log::info(format!("assets: {n}/{total}"));
        }
        Ok(())
    })?;
    Ok(total_objects)
}

// ---------------------------------------------------------------- client jar and OptiFine

pub fn client_jar_candidates(settings: &Settings) -> Vec<PathBuf> {
    let mut c = Vec::new();
    if let Ok(p) = std::env::var("LEAN_CLIENT_JAR") {
        if !p.trim().is_empty() {
            c.push(PathBuf::from(p));
        }
    }
    if let Some(p) = &settings.client_jar_override {
        if !p.trim().is_empty() {
            c.push(PathBuf::from(p));
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        c.push(dir.join("lean-client.jar"));
    }
    c
}

fn install_client_jar(paths: &Paths, settings: &Settings) -> Result<PathBuf> {
    let candidates = client_jar_candidates(settings);
    let Some(src) = candidates.iter().find(|p| p.is_file()) else {
        let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
        bail!("lean-client.jar not found (looked at: {})", tried.join(", "));
    };
    let dest = paths.client_jar();
    let same = match (file_sha1(src), file_sha1(&dest)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        fs::create_dir_all(paths.client_dir())?;
        fs::copy(src, &dest).with_context(|| format!("copying {} to {}", src.display(), dest.display()))?;
    }
    Ok(src.clone())
}

pub fn find_optifine(paths: &Paths) -> Option<PathBuf> {
    let mut jars: Vec<PathBuf> = fs::read_dir(paths.optifine_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("jar")).unwrap_or(false))
        .collect();
    jars.sort();
    jars.into_iter().next()
}

/// Accepts only a .jar containing `optifine/OptiFineClassTransformer.class`, copies it into `optifine\`.
pub fn import_optifine(paths: &Paths, src: &Path) -> Result<PathBuf> {
    let is_jar = src.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("jar")).unwrap_or(false);
    if !is_jar {
        bail!("that file is not a .jar");
    }
    let file = fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| anyhow!("that file is not a valid jar"))?;
    if zip.by_name("optifine/OptiFineClassTransformer.class").is_err() {
        bail!("that jar is not OptiFine (optifine/OptiFineClassTransformer.class is missing)");
    }
    let name = src.file_name().ok_or_else(|| anyhow!("bad file name"))?;
    fs::create_dir_all(paths.optifine_dir())?;
    let dest = paths.optifine_dir().join(name);
    // Keep exactly one OptiFine jar so the classpath never has two.
    if let Ok(rd) = fs::read_dir(paths.optifine_dir()) {
        for e in rd.flatten() {
            if e.path() != dest && e.path().extension().and_then(|x| x.to_str()) == Some("jar") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    if src.canonicalize().ok() != dest.canonicalize().ok() {
        fs::copy(src, &dest).with_context(|| format!("copying to {}", dest.display()))?;
    }
    Ok(dest)
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules() {
        assert!(rules_allow(None));
        assert!(rules_allow(Some(&json!([]))));
        assert!(rules_allow(Some(&json!([{"action":"allow"}]))));
        assert!(rules_allow(Some(&json!([{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]))));
        assert!(!rules_allow(Some(&json!([{"action":"allow"},{"action":"disallow","os":{"name":"windows"}}]))));
        assert!(!rules_allow(Some(&json!([{"action":"allow","os":{"name":"osx"}}]))));
        assert!(rules_allow(Some(&json!([{"action":"allow","os":{"name":"windows"}}]))));
        assert!(!rules_allow(Some(&json!([{"action":"disallow"}]))));
    }

    fn art(path: &str) -> Value {
        json!({"artifact": {"path": path, "url": format!("https://x/{path}"), "sha1": "aa", "size": 5}})
    }

    fn sample() -> Value {
        json!({"libraries": [
            {"name": "com.a:a:1", "downloads": art("com/a/a/1/a-1.jar")},
            {"name": "com.google.guava:guava:15.0", "downloads": art("com/google/guava/guava/15.0/guava-15.0.jar")},
            {"name": "org.osx:only:1", "downloads": art("org/osx/only/1/only-1.jar"),
             "rules": [{"action":"allow","os":{"name":"osx"}}]},
            {"name": "org.lwjgl:platform:2", "downloads": {"classifiers": {
                "natives-windows": {"path":"org/lwjgl/platform/2/platform-2-natives-windows.jar","url":"https://x/n.jar","sha1":"bb","size":9}}},
             "natives": {"linux":"natives-linux","windows":"natives-windows"},
             "extract": {"exclude": ["META-INF/"]},
             "rules": [{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]},
            {"name": "org.arch:n:1", "downloads": {"classifiers": {
                "natives-windows-64": {"path":"org/arch/n/1/n-1-natives-windows-64.jar","url":"https://x/a.jar","sha1":"cc","size":3}}},
             "natives": {"windows":"natives-windows-${arch}"}},
            {"name": "com.b:b:2", "downloads": art("com/b/b/2/b-2.jar")}
        ]})
    }

    #[test]
    fn guava_replaced_and_extras_appended() {
        let plan = plan_libraries(&sample()).unwrap();
        assert_eq!(
            plan.classpath,
            vec![
                "com/a/a/1/a-1.jar",
                "com/google/guava/guava/17.0/guava-17.0.jar",
                "com/b/b/2/b-2.jar",
                "net/minecraft/launchwrapper/1.12/launchwrapper-1.12.jar",
                "org/ow2/asm/asm-all/5.0.3/asm-all-5.0.3.jar",
            ]
        );
        assert!(!plan.files.iter().any(|f| f.rel.contains("15.0")));
        let guava = plan.files.iter().find(|f| f.rel.contains("guava-17.0")).unwrap();
        assert_eq!(guava.url, "https://repo1.maven.org/maven2/com/google/guava/guava/17.0/guava-17.0.jar");
        assert_eq!(guava.sha1_url.as_deref(), Some("https://repo1.maven.org/maven2/com/google/guava/guava/17.0/guava-17.0.jar.sha1"));
        let lw = plan.files.iter().find(|f| f.rel.contains("launchwrapper")).unwrap();
        assert_eq!(lw.url, "https://libraries.minecraft.net/net/minecraft/launchwrapper/1.12/launchwrapper-1.12.jar");
        // osx-only library dropped
        assert!(!plan.files.iter().any(|f| f.rel.contains("osx")));
    }

    #[test]
    fn natives_use_windows_classifier_and_excludes() {
        let plan = plan_libraries(&sample()).unwrap();
        assert_eq!(plan.natives.len(), 2);
        assert_eq!(plan.natives[0].exclude, vec!["META-INF/".to_string()]);
        assert!(plan.natives[1].file.rel.ends_with("natives-windows-64.jar"));
        // natives-only entries are not on the classpath
        assert!(!plan.classpath.iter().any(|c| c.contains("natives")));
    }

    #[test]
    fn existing_launchwrapper_entry_is_replaced_by_extra() {
        let v = json!({"libraries": [
            {"name": "net.minecraft:launchwrapper:1.5", "downloads": art("net/minecraft/launchwrapper/1.5/launchwrapper-1.5.jar")}
        ]});
        let plan = plan_libraries(&v).unwrap();
        assert_eq!(plan.classpath.len(), 2);
        assert!(plan.classpath[0].contains("1.12"));
    }
}
