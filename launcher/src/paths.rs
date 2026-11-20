//! Data folder layout (`%APPDATA%\.lean`), see docs/launch-contract.md.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new() -> Result<Self> {
        let base = dirs::data_dir().ok_or_else(|| anyhow!("cannot find the AppData folder"))?;
        Ok(Self::in_base(&base))
    }

    /// `<base>\.lean`. Before the project was called Lean its folder was `.medal`, with the
    /// client's files in `game\medal`; an old folder is moved over once, so nothing is
    /// downloaded again and waypoints are kept. If the move fails (a file in use), the old
    /// folder is used as it is for this run.
    pub fn in_base(base: &Path) -> Self {
        let new = base.join(".lean");
        let old = base.join(".medal");
        let looks_like_ours = old.join("launcher.json").is_file() || old.join("profiles").is_dir();
        let root = if !new.exists() && looks_like_ours && std::fs::rename(&old, &new).is_err() { old } else { new };
        let game = root.join("game");
        if !game.join("lean").exists() && game.join("medal").is_dir() {
            let _ = std::fs::rename(game.join("medal"), game.join("lean"));
        }
        Self::from_root(root)
    }
    pub fn from_root(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn launcher_json(&self) -> PathBuf { self.root.join("launcher.json") }
    pub fn accounts_json(&self) -> PathBuf { self.root.join("accounts.json") }
    pub fn profiles_dir(&self) -> PathBuf { self.root.join("profiles") }
    pub fn profile_file(&self, id: &str) -> PathBuf { self.profiles_dir().join(format!("{id}.json")) }
    pub fn runtime_dir(&self) -> PathBuf { self.root.join("runtime") }
    pub fn java_dir(&self) -> PathBuf { self.runtime_dir().join("java8") }
    pub fn javaw(&self) -> PathBuf { self.java_dir().join("bin").join("javaw.exe") }
    pub fn version_dir(&self) -> PathBuf { self.root.join("versions").join("1.7.10") }
    pub fn version_json(&self) -> PathBuf { self.version_dir().join("1.7.10.json") }
    pub fn version_jar(&self) -> PathBuf { self.version_dir().join("1.7.10.jar") }
    pub fn libraries_dir(&self) -> PathBuf { self.root.join("libraries") }
    /// Library path from a Maven-style relative path with `/` separators.
    pub fn lib(&self, rel: &str) -> PathBuf { self.libraries_dir().join(rel.replace('/', "\\")) }
    pub fn natives_dir(&self) -> PathBuf { self.root.join("natives").join("1.7.10") }
    pub fn assets_dir(&self) -> PathBuf { self.root.join("assets") }
    pub fn asset_index(&self) -> PathBuf { self.assets_dir().join("indexes").join("1.7.10.json") }
    pub fn asset_object(&self, hash: &str) -> PathBuf {
        self.assets_dir().join("objects").join(&hash[..2]).join(hash)
    }
    pub fn client_dir(&self) -> PathBuf { self.root.join("client") }
    pub fn client_jar(&self) -> PathBuf { self.client_dir().join("lean-client.jar") }
    pub fn optifine_dir(&self) -> PathBuf { self.root.join("optifine") }
    pub fn game_dir(&self) -> PathBuf { self.root.join("game") }
    pub fn game_profile(&self) -> PathBuf { self.game_dir().join("lean").join("profile.json") }
    pub fn logs_dir(&self) -> PathBuf { self.root.join("logs") }
    pub fn launcher_log(&self) -> PathBuf { self.logs_dir().join("launcher.log") }
    pub fn game_log(&self) -> PathBuf { self.logs_dir().join("game-output.log") }
    pub fn log4j_config(&self) -> PathBuf { self.root.join("log_configs").join("log4j2-lean.xml") }

    pub fn ensure_dirs(&self) -> Result<()> {
        for d in [
            self.root.clone(),
            self.profiles_dir(),
            self.runtime_dir(),
            self.version_dir(),
            self.libraries_dir(),
            self.natives_dir(),
            self.assets_dir().join("indexes"),
            self.assets_dir().join("objects"),
            self.client_dir(),
            self.optifine_dir(),
            self.game_dir().join("lean"),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(&d).with_context(|| format!("creating {}", d.display()))?;
        }
        Ok(())
    }
}

/// Write JSON via a temp file and rename, so a crash never leaves a half-written file.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_medal_folder_moves_to_lean() {
        let base = std::env::temp_dir().join(format!("lean-paths-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join(".medal");
        std::fs::create_dir_all(old.join("game").join("medal")).unwrap();
        std::fs::write(old.join("launcher.json"), "{}").unwrap();
        std::fs::write(old.join("game").join("medal").join("waypoints.json"), "[]").unwrap();

        let paths = Paths::in_base(&base);
        assert_eq!(paths.root, base.join(".lean"));
        assert!(!old.exists());
        assert!(paths.launcher_json().is_file());
        assert!(paths.game_dir().join("lean").join("waypoints.json").is_file());

        // An unrelated `.medal` folder (no launcher files) is left alone.
        let other = base.join("other");
        std::fs::create_dir_all(other.join(".medal").join("stuff")).unwrap();
        assert_eq!(Paths::in_base(&other).root, other.join(".lean"));
        assert!(other.join(".medal").join("stuff").is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }
}
