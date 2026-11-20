//! Launch command (docs/launch-contract.md) and detached process start.

use crate::accounts;
use crate::log;
use crate::msauth::Secret;
use crate::paths::Paths;
use crate::prepare::{self, Prepared, Progress};
use crate::profile::{self, Profile};
use crate::settings::Settings;
use anyhow::{Context, Result};
use std::fmt;
use std::fs::File;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Game logging config: Mojang's Log4Shell filter for 1.7, plus dropping the line where 1.7.10
/// prints the session token. Written to `log_configs\` before every launch.
pub const LOG4J_CONFIG: &str = include_str!("../assets/log4j2-lean.xml");

fn write_log4j_config(paths: &Paths) -> Result<()> {
    let path = paths.log4j_config();
    if std::fs::read_to_string(&path).ok().as_deref() == Some(LOG4J_CONFIG) {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("config has a parent folder"))?;
    std::fs::write(&path, LOG4J_CONFIG).with_context(|| format!("writing {}", path.display()))
}

/// Who the game runs as (`--username`, `--uuid`, `--accessToken`): the signed-in Microsoft
/// account. There is no offline identity: Play needs a Microsoft account that owns the game.
#[derive(Clone, Debug)]
pub struct Identity {
    pub name: String,
    /// No dashes.
    pub uuid: String,
    pub access_token: Secret,
}

impl Identity {
    pub fn microsoft(name: &str, uuid: &str, access_token: Secret) -> Self {
        Self { name: name.to_string(), uuid: uuid.to_string(), access_token }
    }
}

#[derive(Clone)]
pub struct LaunchCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl LaunchCommand {
    /// One-line form for logs and `--print-command`; the access token is replaced by `<redacted>`.
    pub fn display_redacted(&self) -> String {
        let mut parts = vec![quote(&self.program.display().to_string())];
        let mut hide = false;
        for a in &self.args {
            parts.push(if hide { "<redacted>".to_string() } else { quote(a) });
            hide = a == "--accessToken";
        }
        parts.join(" ")
    }
}

/// Redacted as well, so `{:?}` can never print the token.
impl fmt::Debug for LaunchCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_redacted())
    }
}

fn quote(s: &str) -> String {
    if s.is_empty() || s.contains(' ') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Contract classpath order: libraries (guava 17, launchwrapper, asm-all included),
/// client jar, OptiFine when on, then the vanilla jar last.
pub fn classpath(paths: &Paths, lib_classpath: &[String], optifine: Option<&PathBuf>) -> Vec<PathBuf> {
    let mut cp: Vec<PathBuf> = lib_classpath.iter().map(|rel| paths.lib(rel)).collect();
    cp.push(paths.client_jar());
    if let Some(o) = optifine {
        cp.push(o.clone());
    }
    cp.push(paths.version_jar());
    cp
}

pub fn build_command(paths: &Paths, settings: &Settings, prepared: &Prepared, identity: &Identity) -> LaunchCommand {
    let cp = classpath(paths, &prepared.lib_classpath, prepared.optifine.as_ref())
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(";");
    let s = |x: &str| x.to_string();
    let args = vec![
        s("-Xms512M"),
        format!("-Xmx{}M", settings.memory_mb),
        s("-XX:+UnlockExperimentalVMOptions"),
        s("-XX:+UseG1GC"),
        s("-XX:G1NewSizePercent=20"),
        s("-XX:G1ReservePercent=20"),
        s("-XX:MaxGCPauseMillis=50"),
        s("-XX:G1HeapRegionSize=32M"),
        format!("-Djava.library.path={}", paths.natives_dir().display()),
        format!("-Dlog4j.configurationFile={}", paths.log4j_config().display()),
        s("-cp"),
        cp,
        s("net.minecraft.launchwrapper.Launch"),
        s("--username"),
        identity.name.clone(),
        s("--version"),
        s("1.7.10"),
        s("--gameDir"),
        paths.game_dir().display().to_string(),
        s("--assetsDir"),
        paths.assets_dir().display().to_string(),
        s("--assetIndex"),
        s("1.7.10"),
        s("--uuid"),
        identity.uuid.clone(),
        s("--accessToken"),
        identity.access_token.expose().to_string(),
        s("--userProperties"),
        s("{}"),
        s("--userType"),
        s("msa"),
        s("--tweakClass"),
        s("lean.client.launch.LeanTweaker"),
    ];
    LaunchCommand { program: paths.javaw(), args }
}

/// Start the game detached from the launcher; stdout and stderr go to `logs\game-output.log`.
pub fn spawn(paths: &Paths, cmd: &LaunchCommand) -> Result<u32> {
    std::fs::create_dir_all(paths.logs_dir())?;
    let out = File::create(paths.game_log()).context("creating game-output.log")?;
    let err = out.try_clone()?;
    let child = Command::new(&cmd.program)
        .args(&cmd.args)
        .current_dir(paths.game_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .with_context(|| format!("starting {}", cmd.program.display()))?;
    Ok(child.id())
}

/// The launcher.log line for a launch; never contains the access token.
pub fn launch_log_line(cmd: &LaunchCommand) -> String {
    format!("launching: {}", cmd.display_redacted())
}

/// Full play pipeline shared by the GUI and `--play`: check the sign-in, prepare, renew a
/// Minecraft token that is about to expire, write the profile, launch.
pub fn play(paths: &Paths, settings: &Settings, profile: &Profile, pg: &Progress) -> Result<u32> {
    // Fails fast, before the slow preparation, when nobody is signed in.
    accounts::stored_identity(paths)?;
    let prepared = prepare::prepare(paths, settings, pg)?;
    let identity = accounts::renewed(paths, settings, pg)?;
    let cmd = build_command(paths, settings, &prepared, &identity);
    profile::write_game_profile(paths, profile)?;
    write_log4j_config(paths)?;
    log::info(launch_log_line(&cmd));
    pg.status("Starting Minecraft");
    let pid = spawn(paths, &cmd)?;
    log::info(format!("game started, pid {pid}"));
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classpath_order() {
        let paths = Paths::from_root(PathBuf::from("C:\\d"));
        let libs = vec!["a/a.jar".to_string(), "net/launchwrapper.jar".to_string(), "asm.jar".to_string()];
        let of = PathBuf::from("C:\\d\\optifine\\of.jar");
        let cp = classpath(&paths, &libs, Some(&of));
        let names: Vec<String> = cp.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(names[0], "C:\\d\\libraries\\a\\a.jar");
        assert_eq!(names[2], "C:\\d\\libraries\\asm.jar");
        assert_eq!(names[3], "C:\\d\\client\\lean-client.jar");
        assert_eq!(names[4], "C:\\d\\optifine\\of.jar");
        assert_eq!(names[5], "C:\\d\\versions\\1.7.10\\1.7.10.jar");
        assert_eq!(cp.len(), 6);
        assert_eq!(classpath(&paths, &libs, None).len(), 5);
    }

    #[test]
    fn command_shape_and_redaction() {
        let paths = Paths::from_root(PathBuf::from("C:\\Users\\A B\\.lean"));
        let settings = Settings { memory_mb: 3072, ..Settings::default() };
        let prepared = Prepared { lib_classpath: vec!["x.jar".into()], ..Prepared::default() };
        let id = Identity::microsoft("Steve", "0123456789abcdef0123456789abcdef", Secret::new(TOKEN));
        let cmd = build_command(&paths, &settings, &prepared, &id);
        assert_eq!(cmd.args[1], "-Xmx3072M");
        let i = cmd.args.iter().position(|a| a == "net.minecraft.launchwrapper.Launch").unwrap();
        assert_eq!(cmd.args[i - 2], "-cp");
        assert_eq!(cmd.args.last().unwrap(), "lean.client.launch.LeanTweaker");
        let game_args: Vec<&str> = cmd.args[i + 1..].iter().map(String::as_str).collect();
        assert_eq!(
            game_args[..18],
            [
                "--username", "Steve", "--version", "1.7.10", "--gameDir", "C:\\Users\\A B\\.lean\\game",
                "--assetsDir", "C:\\Users\\A B\\.lean\\assets", "--assetIndex", "1.7.10",
                "--uuid", "0123456789abcdef0123456789abcdef", "--accessToken", TOKEN, "--userProperties", "{}", "--userType", "msa",
            ]
        );
        let shown = cmd.display_redacted();
        assert!(shown.contains("--accessToken <redacted>"));
        assert!(!shown.contains(TOKEN));
        assert!(cmd.args.contains(&"-Dlog4j.configurationFile=C:\\Users\\A B\\.lean\\log_configs\\log4j2-lean.xml".to_string()));
    }

    #[test]
    fn log_config_blocks_lookups_and_the_session_line() {
        // Mojang's Log4Shell filter for 1.7, kept as is.
        assert!(LOG4J_CONFIG.contains(r#"<RegexFilter regex="(?s).*\$\{[^}]*\}.*" onMatch="DENY""#));
        // 1.7.10 logs "(Session ID is token:<access token>:<uuid>)" at startup.
        assert!(LOG4J_CONFIG.contains(r#"<RegexFilter regex="(?s).*Session ID is.*" onMatch="DENY""#));
        assert!(!LOG4J_CONFIG.contains("XMLLayout"));
    }

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.eyJ4dWlkIjoiMjUzNTQwMDAwMDAwMDAwMCJ9.real-looking-mc-token";

    fn microsoft_command() -> LaunchCommand {
        let paths = Paths::from_root(PathBuf::from("C:\\d"));
        let id = Identity::microsoft("Steve", "0123456789abcdef0123456789abcdef", Secret::new(TOKEN));
        build_command(&paths, &Settings::default(), &Prepared::default(), &id)
    }

    #[test]
    fn microsoft_identity_fills_the_contract_arguments() {
        let cmd = microsoft_command();
        let after = |flag: &str| cmd.args[cmd.args.iter().position(|a| a == flag).unwrap() + 1].as_str();
        assert_eq!(after("--username"), "Steve");
        assert_eq!(after("--uuid"), "0123456789abcdef0123456789abcdef");
        assert_eq!(after("--accessToken"), TOKEN);
        assert_eq!(after("--userType"), "msa");
        assert_eq!(after("--userProperties"), "{}");
    }

    #[test]
    fn logged_launch_command_never_contains_the_token() {
        let cmd = microsoft_command();
        for shown in [cmd.display_redacted(), format!("{cmd:?}"), launch_log_line(&cmd), format!("{:?}", Identity::microsoft("S", "u", Secret::new(TOKEN)))] {
            assert!(!shown.contains(TOKEN), "{shown}");
        }
        assert!(launch_log_line(&cmd).contains("--accessToken <redacted> --userProperties {} --userType msa"));

        // Through the real logger into a real file.
        let dir = std::env::temp_dir().join(format!("lean-launch-log-test-{}", std::process::id()));
        let log_path = dir.join("launcher.log");
        log::init(&log_path, false);
        log::info(launch_log_line(&cmd));
        log::close();
        let text = std::fs::read_to_string(&log_path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.contains("--accessToken <redacted>"), "{text}");
        assert!(!text.contains(TOKEN));
    }
}
