#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod app;
mod download;
mod dpapi;
mod keys;
mod launch;
mod log;
mod msauth;
mod paths;
mod prepare;
mod profile;
mod settings;
mod theme;

use accounts::Accounts;
use msauth::LoginEvent;
use paths::Paths;
use prepare::Progress;
use std::sync::atomic::AtomicBool;

const USAGE: &str = "lean-launcher [--prepare | --print-command | --play] [--profile <id>] [--optifine | --no-optifine]\n\
lean-launcher --login | --logout\n\
With no arguments the launcher window opens.\n\
--login signs in with a Microsoft account (prints a code to enter on Microsoft's site), --logout forgets it.\n\
--play and --print-command need a signed-in Microsoft account that owns Minecraft.";

#[derive(Default, Debug, PartialEq)]
struct Cli {
    prepare: bool,
    print_command: bool,
    play: bool,
    login: bool,
    logout: bool,
    profile: Option<String>,
    optifine: Option<bool>,
}

fn parse_cli(args: &[String]) -> Result<Option<Cli>, String> {
    if args.is_empty() {
        return Ok(None);
    }
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--prepare" => cli.prepare = true,
            "--print-command" => cli.print_command = true,
            "--play" => cli.play = true,
            "--login" => cli.login = true,
            "--logout" => cli.logout = true,
            "--optifine" => cli.optifine = Some(true),
            "--no-optifine" => cli.optifine = Some(false),
            "--profile" => cli.profile = Some(it.next().ok_or("--profile needs a value")?.clone()),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if cli.login && cli.logout {
        return Err("choose either --login or --logout".into());
    }
    if !(cli.prepare || cli.print_command || cli.play || cli.login || cli.logout) {
        return Err("choose one of --prepare, --print-command, --play, --login, --logout".into());
    }
    Ok(Some(cli))
}

/// A windows-subsystem build has no console of its own; attach to the parent's so output is visible.
fn attach_parent_console() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return; // already has a console, or launched without a parent console (pipes work as is)
        }
        for std_id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let h = GetStdHandle(std_id);
            if h.is_null() || h as isize == -1 {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
                    SetStdHandle(std_id, f.as_raw_handle() as _);
                    std::mem::forget(f);
                }
            }
        }
    }
}

fn run_cli(cli: Cli) -> i32 {
    let paths = match Paths::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e:#}");
            return 1;
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("error: {e:#}");
        return 1;
    }
    log::init(&paths.launcher_log(), true);
    match run_cli_inner(&paths, cli) {
        Ok(()) => 0,
        Err(e) => {
            log::error(format!("{e:#}"));
            1
        }
    }
}

fn run_cli_inner(paths: &Paths, cli: Cli) -> anyhow::Result<()> {
    let (mut settings, profiles) = profile::startup(paths)?;
    if let Some(id) = &cli.profile {
        if !profiles.iter().any(|p| &p.id == id) {
            let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
            anyhow::bail!("no profile {id:?} (have: {})", ids.join(", "));
        }
        settings.selected_profile = id.clone();
    }
    if let Some(o) = cli.optifine {
        settings.use_optifine = o;
    }
    if cli.logout {
        cli_logout(paths)?;
    }
    if cli.login {
        cli_login(paths, &settings)?;
    }
    if !(cli.play || cli.prepare || cli.print_command) {
        return Ok(());
    }
    let profile = profiles.iter().find(|p| p.id == settings.selected_profile).cloned().unwrap_or_default();
    let pg = Progress::new();

    if cli.play {
        let pid = launch::play(paths, &settings, &profile, &pg)?;
        log::out(format!("Started Minecraft 1.7.10 (pid {pid}) with profile {}", profile.id));
        return Ok(());
    }

    // The sign-in is checked first, so a missing one fails before any download.
    let identity = if cli.print_command { Some(accounts::stored_identity(paths)?) } else { None };
    let prepared = prepare::prepare(paths, &settings, &pg)?;
    if cli.prepare {
        log::out("Prepared OK");
        log::out(format!("  data folder : {}", paths.root.display()));
        log::out(format!("  java        : {}", paths.javaw().display()));
        log::out(format!("  libraries   : {} files ({} natives archives)", prepared.library_count, prepared.native_count));
        log::out(format!("  assets      : {} objects", prepared.asset_count));
        log::out(format!("  client jar  : {} (from {})", paths.client_jar().display(), prepared.client_jar_source.display()));
        match &prepared.optifine {
            Some(o) => log::out(format!("  optifine    : {}", o.display())),
            None => log::out("  optifine    : off"),
        }
        log::out(format!(
            "  downloaded  : {} files, {:.1} MB this run",
            prepared.downloaded_files,
            prepared.downloaded_bytes as f64 / 1_048_576.0
        ));
    }
    if let Some(identity) = identity {
        log::out(launch::build_command(paths, &settings, &prepared, &identity).display_redacted());
    }
    Ok(())
}

/// `--login`: device code sign-in on the console, then the account is stored.
fn cli_login(paths: &Paths, settings: &settings::Settings) -> anyhow::Result<()> {
    let client = msauth::client_id(settings)?;
    log::info(format!("Microsoft sign-in with app ID {} ({})", client.id, client.source.describe()));
    // Ctrl+C ends the process; nothing is stored until the sign-in completes.
    let never = AtomicBool::new(false);
    let session = msauth::sign_in(&client.id, &never, &mut |event| match event {
        LoginEvent::Code { user_code, verification_uri, expires_in } => {
            // Console only: the code stays out of launcher.log.
            log::console(format!("To sign in, open {verification_uri} in a browser and enter the code {user_code}"));
            if msauth::checked_verification_uri(&verification_uri).is_none() {
                log::console("Careful: that address is not a Microsoft sign-in page.");
            }
            log::console(format!("Waiting up to {} minutes (Ctrl+C cancels)...", expires_in.div_ceil(60)));
        }
        LoginEvent::Status(s) => log::out(s),
    })?;
    let mut accounts = Accounts::load(paths);
    accounts.store(&session)?;
    accounts.save(paths)?;
    log::out(format!("Signed in as {}. Play now uses this Microsoft account.", session.name));
    Ok(())
}

/// `--logout`: forget the Microsoft sign-in (tokens included).
fn cli_logout(paths: &Paths) -> anyhow::Result<()> {
    let mut accounts = Accounts::load(paths);
    if accounts.sign_out() {
        accounts.save(paths)?;
        log::out("Signed out: the Microsoft sign-in is removed from accounts.json.");
    } else {
        log::out("No Microsoft account was signed in.");
    }
    Ok(())
}

fn run_gui() {
    let paths = match Paths::new() {
        Ok(p) => p,
        Err(_) => return,
    };
    if paths.ensure_dirs().is_err() {
        return;
    }
    log::init(&paths.launcher_log(), false);
    let state = match profile::startup(&paths) {
        Ok(s) => s,
        Err(e) => {
            log::error(format!("startup failed: {e:#}"));
            return;
        }
    };
    // Frameless: the window draws its own title bar and resize edges.
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size(window_size(&state.0))
        .with_min_inner_size(app::MIN_WINDOW_SIZE)
        .with_resizable(true)
        .with_decorations(false)
        .with_title("Lean");
    if let Some(icon) = theme::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    let result = eframe::run_native(
        "Lean",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(app::App::new(paths, state.0, state.1)))
        }),
    );
    if let Err(e) = result {
        log::error(format!("window error: {e}"));
    }
}

/// The window opens at its last size; `LEAN_DEV_WINDOW_SIZE=<w>x<h>` overrides it (dev screenshots).
fn window_size(settings: &settings::Settings) -> [f32; 2] {
    let dev = std::env::var("LEAN_DEV_WINDOW_SIZE").ok().and_then(|v| {
        let (w, h) = v.split_once('x')?;
        Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
    });
    let [w, h] = dev.or(settings.window_size).unwrap_or(app::WINDOW_SIZE);
    let ok = |v: f32, min: f32| if v.is_finite() { v.clamp(min, 8192.0) } else { min };
    [ok(w, app::MIN_WINDOW_SIZE[0]), ok(h, app::MIN_WINDOW_SIZE[1])]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli(&args) {
        Ok(None) => run_gui(),
        Ok(Some(cli)) => {
            attach_parent_console();
            std::process::exit(run_cli(cli));
        }
        Err(msg) => {
            attach_parent_console();
            if !msg.is_empty() {
                println!("error: {msg}");
            }
            println!("{USAGE}");
            std::process::exit(if msg.is_empty() { 0 } else { 2 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Cli>, String> {
        parse_cli(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn cli_flags() {
        assert_eq!(parse(&[]), Ok(None));
        let c = parse(&["--play", "--profile", "pvp"]).unwrap().unwrap();
        assert!(c.play && !c.login);
        assert_eq!(c.profile.as_deref(), Some("pvp"));
        assert!(parse(&["--login"]).unwrap().unwrap().login);
        assert!(parse(&["--logout"]).unwrap().unwrap().logout);
        assert!(parse(&["--login", "--logout"]).is_err());
        let c = parse(&["--prepare", "--print-command", "--no-optifine"]).unwrap().unwrap();
        assert!(c.prepare && c.print_command && c.optifine == Some(false));
        // There is no offline account any more.
        assert_eq!(parse(&["--play", "--offline"]), Err("unknown argument --offline".into()));
        assert_eq!(parse(&["--play", "--username", "Dev"]), Err("unknown argument --username".into()));
        assert_eq!(parse(&["--bogus"]), Err("unknown argument --bogus".into()));
    }
}
