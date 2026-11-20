//! Profile format (docs/launch-contract.md) and the on-disk profile store.
//!
//! Every struct keeps unknown fields in `extra`, so profiles written by newer
//! launcher or client versions survive a load/save round trip.

use crate::paths::{write_json_atomic, Paths};
use crate::settings::Settings;
use crate::log;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

type Extra = Map<String, Value>;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Hud {
    pub color: String,
    pub shadow: bool,
    #[serde(flatten)]
    pub extra: Extra,
}
impl Default for Hud {
    fn default() -> Self {
        Self { color: "FFFFFF".into(), shadow: true, extra: Extra::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ToggleSprint {
    pub enabled: bool,
    pub show_status: bool,
    pub toggled: bool,
    pub x: f64,
    pub y: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
impl Default for ToggleSprint {
    fn default() -> Self {
        Self { enabled: true, show_status: true, toggled: false, x: 0.5, y: 1.0, extra: Extra::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Cps {
    pub enabled: bool,
    pub show_right: bool,
    pub x: f64,
    pub y: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
impl Default for Cps {
    fn default() -> Self {
        Self { enabled: true, show_right: true, x: 0.5, y: 5.0, extra: Extra::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Coords {
    pub enabled: bool,
    pub show_direction: bool,
    pub x: f64,
    pub y: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
impl Default for Coords {
    fn default() -> Self {
        Self { enabled: true, show_direction: true, x: 0.5, y: 9.5, extra: Extra::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PotCounter {
    pub enabled: bool,
    pub x: f64,
    pub y: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
impl Default for PotCounter {
    fn default() -> Self {
        Self { enabled: true, x: 0.5, y: 18.0, extra: Extra::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Waypoints {
    pub enabled: bool,
    pub show_distance: bool,
    #[serde(flatten)]
    pub extra: Extra,
}
impl Default for Waypoints {
    fn default() -> Self {
        Self { enabled: true, show_distance: true, extra: Extra::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Keys {
    pub add_waypoint: String,
    pub waypoint_menu: String,
    pub hud_editor: String,
    #[serde(flatten)]
    pub extra: Extra,
}
impl Default for Keys {
    fn default() -> Self {
        Self {
            add_waypoint: "B".into(),
            waypoint_menu: "M".into(),
            hud_editor: "RSHIFT".into(),
            extra: Extra::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub hud: Hud,
    pub toggle_sprint: ToggleSprint,
    pub cps: Cps,
    pub coords: Coords,
    pub pot_counter: PotCounter,
    pub waypoints: Waypoints,
    pub keys: Keys,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            schema: 1,
            id: "default".into(),
            name: "Default".into(),
            hud: Hud::default(),
            toggle_sprint: ToggleSprint::default(),
            cps: Cps::default(),
            coords: Coords::default(),
            pot_counter: PotCounter::default(),
            waypoints: Waypoints::default(),
            keys: Keys::default(),
            extra: Extra::new(),
        }
    }
}

impl Profile {
    /// A profile with default values and the given id and name.
    pub fn new(id: &str, name: &str) -> Self {
        Self { id: id.into(), name: name.into(), ..Self::default() }
    }
}

/// Lowercase letters, digits and `-`; this is also the file name.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "profile".into() } else { out }
}

/// `base`, then `base-2`, `base-3`, ... until it is not in `existing`.
pub fn unique_id(base: &str, existing: &[String]) -> String {
    if !existing.iter().any(|e| e == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !existing.iter().any(|e| e == c))
        .unwrap()
}

pub fn list(paths: &Paths) -> Vec<Profile> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(paths.profiles_dir()) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if !valid_id(&stem) {
                continue;
            }
            match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<Profile>(&b)?)) {
                Ok(mut p) => {
                    p.id = stem; // the file name is the id
                    out.push(p);
                }
                Err(e) => log::warn(format!("skipping unreadable profile {}: {e:#}", path.display())),
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    out
}

pub fn save(paths: &Paths, p: &Profile) -> Result<()> {
    if !valid_id(&p.id) {
        bail!("invalid profile id {:?}", p.id);
    }
    write_json_atomic(&paths.profile_file(&p.id), p)
}

pub fn delete(paths: &Paths, id: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("invalid profile id {id:?}");
    }
    std::fs::remove_file(paths.profile_file(id)).with_context(|| format!("deleting profile {id}"))
}

/// Writes the profile the game will read (`game\lean\profile.json`).
pub fn write_game_profile(paths: &Paths, p: &Profile) -> Result<()> {
    write_json_atomic(&paths.game_profile(), p)
}

/// Contract "Profile round trip": replace the stored copy with the game's copy when the ids match.
pub fn import_game_profile(paths: &Paths) -> Result<Option<String>> {
    let path = paths.game_profile();
    if !path.exists() {
        return Ok(None);
    }
    let p: Profile = serde_json::from_slice(&std::fs::read(&path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    if !valid_id(&p.id) || !paths.profile_file(&p.id).exists() {
        return Ok(None);
    }
    save(paths, &p)?;
    Ok(Some(p.id))
}

/// First-run setup shared by the GUI and the CLI: folders, settings, default profile, round trip.
pub fn startup(paths: &Paths) -> Result<(Settings, Vec<Profile>)> {
    paths.ensure_dirs()?;
    match import_game_profile(paths) {
        Ok(Some(id)) => log::info(format!("imported profile {id} from the game folder")),
        Ok(None) => {}
        Err(e) => log::warn(format!("could not import game profile: {e:#}")),
    }
    let mut profiles = list(paths);
    if profiles.is_empty() {
        let p = Profile::default();
        save(paths, &p)?;
        profiles.push(p);
    }
    let mut settings = Settings::load(paths);
    if !profiles.iter().any(|p| p.id == settings.selected_profile) {
        settings.selected_profile = profiles[0].id.clone();
    }
    if !paths.launcher_json().exists() {
        settings.save(paths)?;
    }
    Ok((settings, profiles))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_contract() {
        let v = serde_json::to_value(Profile::default()).unwrap();
        let expected: Value = serde_json::from_str(
            r#"{
              "schema": 1, "id": "default", "name": "Default",
              "hud": { "color": "FFFFFF", "shadow": true },
              "toggleSprint": { "enabled": true, "showStatus": true, "toggled": false, "x": 0.5, "y": 1.0 },
              "cps":          { "enabled": true, "showRight": true, "x": 0.5, "y": 5.0 },
              "coords":       { "enabled": true, "showDirection": true, "x": 0.5, "y": 9.5 },
              "potCounter":   { "enabled": true, "x": 0.5, "y": 18.0 },
              "waypoints":    { "enabled": true, "showDistance": true },
              "keys": { "addWaypoint": "B", "waypointMenu": "M", "hudEditor": "RSHIFT" }
            }"#,
        )
        .unwrap();
        assert_eq!(v, expected);
    }

    #[test]
    fn round_trip_keeps_unknown_fields() {
        let src = r#"{
          "schema": 1, "id": "x", "name": "X", "futureTop": {"a": [1, 2]},
          "hud": { "color": "00FF00", "shadow": false, "outline": 3 },
          "toggleSprint": { "enabled": false, "showStatus": true, "toggled": true, "x": 1.5, "y": 2.5, "scale": 2 },
          "cps": { "enabled": true, "showRight": false, "x": 1.0, "y": 1.0, "mode": "avg" },
          "coords": { "enabled": true, "showDirection": true, "x": 1.0, "y": 1.0, "decimals": 2 },
          "potCounter": { "enabled": true, "x": 1.0, "y": 1.0, "icon": true },
          "waypoints": { "enabled": true, "showDistance": true, "max": 50 },
          "keys": { "addWaypoint": "B", "waypointMenu": "M", "hudEditor": "RSHIFT", "zoom": "C" }
        }"#;
        let original: Value = serde_json::from_str(src).unwrap();
        let p: Profile = serde_json::from_str(src).unwrap();
        assert_eq!(p.hud.color, "00FF00");
        assert!(p.toggle_sprint.toggled);
        let back = serde_json::to_value(&p).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let p: Profile = serde_json::from_str(r#"{"id":"a","name":"A","cps":{"x":9.0}}"#).unwrap();
        assert_eq!(p.cps.x, 9.0);
        assert_eq!(p.cps.y, 5.0);
        assert!(p.cps.enabled);
        assert_eq!(p.keys.hud_editor, "RSHIFT");
    }

    #[test]
    fn ids() {
        assert_eq!(slugify("My Cool Profile!"), "my-cool-profile");
        assert_eq!(slugify("***"), "profile");
        assert_eq!(slugify("  A  B "), "a-b");
        let ex = vec!["a".to_string(), "a-2".to_string()];
        assert_eq!(unique_id("a", &ex), "a-3");
        assert_eq!(unique_id("b", &ex), "b");
        assert!(valid_id("my-id-2"));
        assert!(!valid_id("../x"));
        assert!(!valid_id("Upper"));
    }
}
