//! `accounts.json`: the Microsoft sign-in that Play uses.
//!
//! The refresh token and the Minecraft token are encrypted with Windows DPAPI (current user)
//! and stored as base64. When they cannot be decrypted (another Windows user or PC, a damaged
//! file) the account counts as signed out.

use crate::dpapi;
use crate::launch::Identity;
use crate::log;
use crate::msauth::{self, AuthError, Secret, Session};
use crate::paths::{write_json_atomic, Paths};
use crate::prepare::Progress;
use crate::settings::Settings;
use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Play never starts the game without a signed-in Microsoft account that owns Minecraft: Mojang
/// only approves launchers that check sign-in and ownership before any play, on any server.
pub const NOT_SIGNED_IN: &str = "Sign in with the Microsoft account that owns Minecraft first, under Settings > \
Account (from the command line: --login).";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredMicrosoft {
    pub name: String,
    /// No dashes.
    pub uuid: String,
    /// DPAPI-encrypted, base64.
    pub refresh_token: String,
    /// DPAPI-encrypted, base64.
    pub minecraft_token: String,
    /// Unix seconds.
    pub minecraft_token_expires: i64,
    /// The app ID that issued the refresh token.
    pub client_id: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Accounts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microsoft: Option<StoredMicrosoft>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Accounts {
    pub fn load(paths: &Paths) -> Self {
        let Ok(bytes) = std::fs::read(paths.accounts_json()) else { return Self::default() };
        Self::parse(&bytes).unwrap_or_else(|e| {
            // Position only: serde's message can quote file content.
            log::warn(format!("accounts.json is unreadable (line {}, column {}); treating it as signed out", e.line(), e.column()));
            Self::default()
        })
    }

    /// Older launchers also stored `"type"` (`offline` or `microsoft`); it is dropped.
    fn parse(bytes: &[u8]) -> serde_json::Result<Self> {
        let mut a: Self = serde_json::from_slice(bytes)?;
        a.extra.remove("type");
        Ok(a)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_json_atomic(&paths.accounts_json(), self)
    }

    /// The Microsoft sign-in with both tokens decrypted; None when signed out or unreadable.
    pub fn session(&self) -> Option<Session> {
        let m = self.microsoft.as_ref()?;
        let (Some(refresh_token), Some(mc_token)) = (decrypt(&m.refresh_token), decrypt(&m.minecraft_token)) else {
            log::warn("The stored Microsoft sign-in could not be decrypted (another Windows user or PC?); treating it as signed out");
            return None;
        };
        if m.name.is_empty() || m.uuid.len() != 32 {
            log::warn("The stored Microsoft account is incomplete; treating it as signed out");
            return None;
        }
        Some(Session {
            name: m.name.clone(),
            uuid: m.uuid.clone(),
            refresh_token,
            mc_token,
            mc_expires: m.minecraft_token_expires,
            client_id: m.client_id.clone(),
        })
    }

    /// Stores a sign-in, encrypting both tokens. Unknown fields of the old entry are kept.
    pub fn store(&mut self, s: &Session) -> Result<()> {
        let refresh_token = encrypt(&s.refresh_token)?;
        let minecraft_token = encrypt(&s.mc_token)?;
        let extra = self.microsoft.take().map(|m| m.extra).unwrap_or_default();
        self.microsoft = Some(StoredMicrosoft {
            name: s.name.clone(),
            uuid: s.uuid.clone(),
            refresh_token,
            minecraft_token,
            minecraft_token_expires: s.mc_expires,
            client_id: s.client_id.clone(),
            extra,
        });
        Ok(())
    }

    /// Forgets the Microsoft sign-in. Returns whether there was one.
    pub fn sign_out(&mut self) -> bool {
        self.microsoft.take().is_some()
    }
}

fn encrypt(s: &Secret) -> Result<String> {
    if s.expose().is_empty() {
        bail!("refusing to store an empty token");
    }
    Ok(BASE64.encode(dpapi::protect(s.expose().as_bytes())?))
}

fn decrypt(stored: &str) -> Option<Secret> {
    let blob = BASE64.decode(stored.trim()).ok()?;
    let plain = dpapi::unprotect(&blob).ok()?;
    String::from_utf8(plain).ok().filter(|t| !t.is_empty()).map(Secret::new)
}

// ---------------------------------------------------------------- launch

/// The identity Play would use, from what is stored (no network). Called before the slow
/// preparation so a missing sign-in fails fast.
pub fn stored_identity(paths: &Paths) -> Result<Identity> {
    let s = Accounts::load(paths).session().ok_or_else(|| anyhow!(NOT_SIGNED_IN))?;
    Ok(Identity::microsoft(&s.name, &s.uuid, s.mc_token))
}

/// Right before a launch: the signed-in identity, with its Minecraft token renewed when it
/// expires within 10 minutes (refresh, then Xbox Live, XSTS, Minecraft and profile again).
pub fn renewed(paths: &Paths, settings: &Settings, pg: &Progress) -> Result<Identity> {
    let mut accounts = Accounts::load(paths);
    let session = accounts.session().ok_or_else(|| anyhow!(NOT_SIGNED_IN))?;
    if !msauth::needs_renewal(session.mc_expires, msauth::now_secs()) {
        return Ok(Identity::microsoft(&session.name, &session.uuid, session.mc_token));
    }
    pg.status("Renewing the Microsoft sign-in");
    let client_id = if session.client_id.is_empty() { msauth::client_id(settings)?.id } else { session.client_id.clone() };
    // Microsoft may retire the old refresh token once it hands out a new one: store it at once.
    let mut keep_refresh_token = |token: &Secret| {
        let mut s = session.clone();
        s.refresh_token = token.clone();
        if let Err(e) = accounts.store(&s).and_then(|_| accounts.save(paths)) {
            log::warn(format!("could not store the renewed refresh token: {e:#}"));
        }
    };
    match msauth::renew(&client_id, &session.refresh_token, &mut keep_refresh_token) {
        Ok(new) => {
            accounts.store(&new)?;
            accounts.save(paths)?;
            log::info(format!("Microsoft sign-in renewed for {}", new.name));
            Ok(Identity::microsoft(&new.name, &new.uuid, new.mc_token))
        }
        Err(AuthError::SignInAgain(msg)) => {
            log::warn("Microsoft refused the stored refresh token; signing out");
            accounts.sign_out();
            if let Err(e) = accounts.save(paths) {
                log::warn(format!("could not update accounts.json: {e:#}"));
            }
            bail!(msg)
        }
        Err(e) => bail!(
            "Could not renew the Microsoft sign-in: {e} If this keeps happening, sign out and sign in again under Settings > Account."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const REFRESH: &str = "M.C5_BAY.0.U.-test-refresh-token-value!*$";
    const MC: &str = "eyJraWQiOiJ0ZXN0In0.eyJzdWIiOiJ0ZXN0LW1jLXRva2VuIn0.c2ln";

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lean-accounts-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn session() -> Session {
        Session {
            name: "Steve".into(),
            uuid: "0123456789abcdef0123456789abcdef".into(),
            refresh_token: Secret::new(REFRESH),
            mc_token: Secret::new(MC),
            mc_expires: 1_900_000_000,
            client_id: "11111111-1111-1111-1111-111111111111".into(),
        }
    }

    #[test]
    fn stored_tokens_are_encrypted_and_round_trip() {
        let dir = TempDir::new("roundtrip");
        let paths = Paths::from_root(dir.0.clone());
        let mut a = Accounts::default();
        a.store(&session()).unwrap();
        a.save(&paths).unwrap();

        let text = std::fs::read_to_string(paths.accounts_json()).unwrap();
        assert!(!text.contains(REFRESH) && !text.contains(MC), "tokens must not be stored in plain text");
        assert!(text.contains("\"name\": \"Steve\""));
        let v: Value = serde_json::from_str(&text).unwrap();
        let blob = BASE64.decode(v["microsoft"]["refreshToken"].as_str().unwrap()).unwrap();
        assert_eq!(dpapi::unprotect(&blob).unwrap(), REFRESH.as_bytes());

        let back = Accounts::load(&paths);
        let s = back.session().expect("signed in");
        assert_eq!((s.name.as_str(), s.uuid.as_str(), s.mc_expires), ("Steve", "0123456789abcdef0123456789abcdef", 1_900_000_000));
        assert_eq!((s.refresh_token.expose(), s.mc_token.expose()), (REFRESH, MC));
        assert_eq!(s.client_id, "11111111-1111-1111-1111-111111111111");
    }

    #[test]
    fn undecryptable_tokens_mean_signed_out() {
        let mut a = Accounts::default();
        a.store(&session()).unwrap();
        let mut broken = a.clone();
        broken.microsoft.as_mut().unwrap().refresh_token = BASE64.encode(b"not a dpapi blob");
        assert!(broken.session().is_none());
        let mut garbage = a.clone();
        garbage.microsoft.as_mut().unwrap().minecraft_token = "%%% not base64".into();
        assert!(garbage.session().is_none());
        assert!(a.session().is_some());
        assert!(Accounts::default().session().is_none());
    }

    #[test]
    fn file_format_and_unknown_fields() {
        let a = Accounts::parse(br#"{"type":"offline","future":1,"microsoft":{"name":"A","later":true}}"#).unwrap();
        let back = serde_json::to_value(&a).unwrap();
        assert!(back.get("type").is_none(), "the old account type is dropped");
        assert_eq!(back["future"], 1);
        assert_eq!(back["microsoft"]["later"], true);
        assert_eq!(serde_json::to_value(Accounts::default()).unwrap(), serde_json::json!({}));
        assert!(!Accounts::default().sign_out());
    }

    #[test]
    fn play_needs_a_signed_in_account() {
        let dir = TempDir::new("identity");
        let paths = Paths::from_root(dir.0.clone());
        let settings = Settings::default();

        // No accounts.json: nobody to play as.
        assert_eq!(stored_identity(&paths).unwrap_err().to_string(), NOT_SIGNED_IN);
        assert_eq!(renewed(&paths, &settings, &Progress::new()).unwrap_err().to_string(), NOT_SIGNED_IN);

        // A file from an older launcher that chose the offline account: still signed out.
        std::fs::write(paths.accounts_json(), r#"{"type": "offline"}"#).unwrap();
        assert_eq!(stored_identity(&paths).unwrap_err().to_string(), NOT_SIGNED_IN);

        // Signed in, token valid for a day: used as is, without network.
        let mut a = Accounts::load(&paths);
        let mut s = session();
        s.mc_expires = msauth::now_secs() + 86_400;
        a.store(&s).unwrap();
        a.save(&paths).unwrap();
        let id = stored_identity(&paths).unwrap();
        assert_eq!((id.name.as_str(), id.access_token.expose()), ("Steve", MC));
        let id = renewed(&paths, &settings, &Progress::new()).unwrap();
        assert_eq!(id.access_token.expose(), MC);
    }
}
