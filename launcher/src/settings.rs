//! `launcher.json`: launcher settings.

use crate::paths::{write_json_atomic, Paths};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MEM_MIN: u32 = 1024;
pub const MEM_MAX: u32 = 4096;
pub const MEM_STEP: u32 = 256;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub selected_profile: String,
    pub memory_mb: u32,
    pub use_optifine: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_jar_override: Option<String>,
    /// Azure app (client) ID for Microsoft sign-in; `LEAN_MS_CLIENT_ID` takes precedence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms_client_id: Option<String>,
    /// Launcher accent: `red` (default), `blue`, `purple` or `mono`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Last window size in points at 100% scale; None until the window was closed once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_size: Option<[f32; 2]>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            selected_profile: "default".into(),
            memory_mb: 2048,
            use_optifine: false,
            client_jar_override: None,
            ms_client_id: None,
            accent: None,
            window_size: None,
            extra: Map::new(),
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Self {
        let mut s: Settings = std::fs::read(paths.launcher_json())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        s.memory_mb = s.memory_mb.clamp(MEM_MIN, MEM_MAX);
        // Older launchers stored the offline account's name here.
        s.extra.remove("username");
        s
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_json_atomic(&paths.launcher_json(), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_fields_missing() {
        let s: Settings = serde_json::from_str("{\"future\":1}").unwrap();
        assert_eq!(s.memory_mb, 2048);
        assert_eq!(s.selected_profile, "default");
        assert!(s.extra.contains_key("future"));
    }

    #[test]
    fn ms_client_id_is_only_added() {
        let src = r#"{"selectedProfile":"default","memoryMb":2048,"useOptifine":false,"clientJarOverride":"C:\\x\\lean-client.jar"}"#;
        let mut s: Settings = serde_json::from_str(src).unwrap();
        assert_eq!(s.ms_client_id, None);
        // Untouched settings serialize exactly as before (no new keys).
        assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::from_str::<Value>(src).unwrap());
        s.ms_client_id = Some("11111111-1111-1111-1111-111111111111".into());
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["msClientId"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(v["clientJarOverride"], "C:\\x\\lean-client.jar");
        let back: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back.ms_client_id.as_deref(), Some("11111111-1111-1111-1111-111111111111"));
        assert!(!back.extra.contains_key("msClientId"));
    }

    #[test]
    fn old_offline_username_is_dropped() {
        let dir = std::env::temp_dir().join(format!("lean-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::from_root(dir.clone());
        std::fs::write(paths.launcher_json(), r#"{"selectedProfile":"pvp","username":"Player","memoryMb":3072,"future":1}"#).unwrap();
        let s = Settings::load(&paths);
        assert!(!s.extra.contains_key("username"));
        assert_eq!((s.selected_profile.as_str(), s.memory_mb), ("pvp", 3072));
        s.save(&paths).unwrap();
        let text = std::fs::read_to_string(paths.launcher_json()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!text.contains("username"), "{text}");
        assert!(text.contains("future"));
    }
}
