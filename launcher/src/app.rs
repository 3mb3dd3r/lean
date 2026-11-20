//! The launcher window (egui / eframe): a frameless window with a sidebar and three pages,
//! Home, Profiles and Settings. Look and custom painting live in `theme.rs`.

use crate::accounts::{self, Accounts};
use crate::keys;
use crate::launch;
use crate::log;
use crate::msauth::{self, AuthError, ClientIdSource, LoginEvent, Session};
use crate::paths::Paths;
use crate::prepare::{self, Progress};
use crate::profile::{self, Profile};
use crate::settings::{Settings, MEM_MAX, MEM_MIN, MEM_STEP};
use crate::theme::{self, icons::Icon, white, Accent, Logo, Palette, PlayLabel, TEXT, TEXT_2, TEXT_3};
use eframe::egui::{self, pos2, vec2, Color32, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SAVE_DELAY: Duration = Duration::from_millis(400);

/// The size the layout is designed for, in points. The window can be resized: the whole UI is
/// scaled so this size fits, and any extra room in the other direction goes to the pages.
pub const WINDOW_SIZE: [f32; 2] = [1040.0, 660.0];
/// Smallest window (the UI is then drawn at 75%).
pub const MIN_WINDOW_SIZE: [f32; 2] = [780.0, 495.0];
/// Within this distance of the window edge the pointer resizes the window.
const RESIZE_MARGIN: f32 = 6.0;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Home,
    Profiles,
    Settings,
}

enum Dialog {
    None,
    New,
    Rename,
    Delete,
}

pub struct App {
    paths: Paths,
    settings: Settings,
    profiles: Vec<Profile>,
    sel: usize,
    tab: Tab,
    dialog: Dialog,
    dialog_text: String,
    key_bufs: [String; 3],
    key_buf_id: String,
    save_due: Option<Instant>,
    error: Option<String>,
    note: Option<String>,
    progress: Arc<Progress>,
    job: Option<Receiver<Result<u32, String>>>,
    optifine_jar: Option<String>,
    dev_shot: Option<DevShot>,
    /// Name of the signed-in Microsoft account; None when signed out or not decryptable.
    ms_name: Option<String>,
    client_id_buf: String,
    client_id_edit: bool,
    login: Option<Login>,
    accent: Accent,
    styled_for: Option<Accent>,
    logo: Option<Logo>,
    logo_tried: bool,
    /// Window size (points at 100%) while not maximized; saved on exit.
    window_size: Option<[f32; 2]>,
}

/// A Microsoft sign-in running on a background thread.
struct Login {
    rx: Receiver<LoginMsg>,
    cancel: Arc<AtomicBool>,
    code: Option<ShownCode>,
    status: String,
}

struct ShownCode {
    user_code: String,
    uri: String,
    /// Passed the sign-in address check, so the button may open it.
    uri_trusted: bool,
    expires: Instant,
}

enum LoginMsg {
    Event(LoginEvent),
    Done(Result<Session, AuthError>),
}

/// Dev only: `LEAN_DEV_SCREENSHOT=<file.png>` saves the Home page to that file, Profiles to
/// `<file>.profiles.png` and Settings to `<file>.settings.png`, then closes the window. Used to
/// check the UI without anyone looking at the screen.
struct DevShot {
    path: std::path::PathBuf,
    frame: u32,
    saved: u32,
}

impl App {
    pub fn new(paths: Paths, mut settings: Settings, profiles: Vec<Profile>) -> Self {
        let sel = profiles.iter().position(|p| p.id == settings.selected_profile).unwrap_or(0);
        let optifine_jar = prepare::find_optifine(&paths)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        if optifine_jar.is_none() {
            settings.use_optifine = false;
        }
        let accounts = Accounts::load(&paths);
        Self {
            ms_name: accounts.session().map(|s| s.name),
            client_id_buf: String::new(),
            client_id_edit: false,
            login: None,
            accent: Accent::from_name(settings.accent.as_deref()),
            styled_for: None,
            logo: None,
            logo_tried: false,
            window_size: None,
            paths,
            settings,
            profiles,
            sel,
            tab: Tab::Home,
            dialog: Dialog::None,
            dialog_text: String::new(),
            key_bufs: Default::default(),
            key_buf_id: String::new(),
            save_due: None,
            error: None,
            note: None,
            progress: Arc::new(Progress::new()),
            job: None,
            optifine_jar,
            dev_shot: std::env::var_os("LEAN_DEV_SCREENSHOT").map(|p| DevShot { path: p.into(), frame: 0, saved: 0 }),
        }
    }

    fn dev_screenshot(&mut self, ctx: &egui::Context) {
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(shot) = self.dev_shot.as_mut() else { return };
        ctx.request_repaint();
        match image {
            None => {
                shot.frame += 1;
                if shot.frame == 10 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                }
            }
            Some(image) => {
                let path = match shot.saved {
                    0 => shot.path.clone(),
                    1 => shot.path.with_extension("profiles.png"),
                    _ => shot.path.with_extension("settings.png"),
                };
                match save_png(&path, &image) {
                    Ok(()) => log::info(format!("dev screenshot saved: {}", path.display())),
                    Err(e) => log::error(format!("dev screenshot failed: {e:#}")),
                }
                shot.saved += 1;
                shot.frame = 0;
                match shot.saved {
                    1 => self.tab = Tab::Profiles,
                    2 => self.tab = Tab::Settings,
                    _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                }
            }
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.paths) {
            self.fail(format!("Could not save settings: {e:#}"));
        }
    }

    fn fail(&mut self, msg: String) {
        log::error(&msg);
        self.error = Some(msg);
    }

    fn save_profile_now(&mut self) {
        self.save_due = None;
        if let Some(p) = self.profiles.get(self.sel) {
            if let Err(e) = profile::save(&self.paths, p) {
                let msg = format!("Could not save profile: {e:#}");
                self.fail(msg);
            }
        }
    }

    fn select(&mut self, idx: usize) {
        if self.save_due.is_some() {
            self.save_profile_now();
        }
        self.sel = idx.min(self.profiles.len().saturating_sub(1));
        self.settings.selected_profile = self.profiles[self.sel].id.clone();
        self.save_settings();
    }

    fn add_profile(&mut self, p: Profile) {
        if let Err(e) = profile::save(&self.paths, &p) {
            self.fail(format!("Could not save profile: {e:#}"));
            return;
        }
        let id = p.id.clone();
        self.profiles.push(p);
        self.profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
        let idx = self.profiles.iter().position(|x| x.id == id).unwrap_or(0);
        self.select(idx);
    }

    fn ids(&self) -> Vec<String> {
        self.profiles.iter().map(|p| p.id.clone()).collect()
    }

    fn duplicate(&mut self) {
        let Some(src) = self.profiles.get(self.sel).cloned() else { return };
        let name = format!("{} copy", src.name);
        let mut p = src;
        p.id = profile::unique_id(&profile::slugify(&name), &self.ids());
        p.name = name;
        self.add_profile(p);
    }

    fn delete_selected(&mut self) {
        if self.profiles.len() <= 1 {
            return;
        }
        let id = self.profiles[self.sel].id.clone();
        if let Err(e) = profile::delete(&self.paths, &id) {
            self.fail(format!("Could not delete profile: {e:#}"));
            return;
        }
        self.save_due = None;
        self.profiles.remove(self.sel);
        self.select(self.sel);
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.job else { return };
        match rx.try_recv() {
            Ok(Ok(_pid)) => {
                // The game runs on its own now; nothing else should keep running beside it.
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Ok(Err(msg)) => {
                self.job = None;
                // The launch may have renewed the sign-in, or dropped one Microsoft refused.
                self.reload_account();
                self.fail(msg);
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.reload_account();
                self.fail("The launch task stopped unexpectedly".into());
            }
        }
    }

    fn start_play(&mut self, ctx: &egui::Context) {
        if self.save_due.is_some() {
            self.save_profile_now();
        }
        if self.ms_name.is_none() {
            self.tab = Tab::Settings;
            self.error = Some(accounts::NOT_SIGNED_IN.into());
            return;
        }
        let Some(profile) = self.profiles.get(self.sel).cloned() else { return };
        self.error = None;
        self.note = None;
        self.progress.reset();
        let (paths, settings, pg) = (self.paths.clone(), self.settings.clone(), self.progress.clone());
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let r = launch::play(&paths, &settings, &profile, &pg).map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.job = Some(rx);
    }

    fn set_accent(&mut self, accent: Accent) {
        if accent == self.accent {
            return;
        }
        self.accent = accent;
        self.settings.accent = (accent != Accent::Red).then(|| accent.name().to_string());
        self.save_settings();
    }

    // ------------------------------------------------------------ account

    fn reload_account(&mut self) {
        self.ms_name = Accounts::load(&self.paths).session().map(|s| s.name);
    }

    /// Read, change and save accounts.json, so a token the Play task renewed is never
    /// overwritten with an older copy.
    fn update_accounts(&mut self, change: impl FnOnce(&mut Accounts) -> anyhow::Result<()>) -> bool {
        let mut a = Accounts::load(&self.paths);
        match change(&mut a).and_then(|_| a.save(&self.paths)) {
            Ok(()) => true,
            Err(e) => {
                self.fail(format!("Could not save the account: {e:#}"));
                false
            }
        }
    }

    fn start_login(&mut self, ctx: &egui::Context, client_id: String) {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (cancel2, ctx2) = (cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let events = tx.clone();
            let ctx3 = ctx2.clone();
            let result = msauth::sign_in(&client_id, &cancel2, &mut |event| {
                let _ = events.send(LoginMsg::Event(event));
                ctx3.request_repaint();
            });
            let _ = tx.send(LoginMsg::Done(result));
            ctx2.request_repaint();
        });
        self.error = None;
        self.note = None;
        self.login = Some(Login { rx, cancel, code: None, status: "Contacting Microsoft".into() });
    }

    /// Stops a sign-in in progress; its thread stores nothing and exits at the next check.
    fn cancel_login(&mut self) {
        if let Some(login) = self.login.take() {
            login.cancel.store(true, Ordering::Relaxed);
            log::info("Microsoft sign-in cancelled");
        }
    }

    fn poll_login(&mut self, ctx: &egui::Context) {
        let Some(login) = self.login.as_mut() else { return };
        let mut done = None;
        loop {
            match login.rx.try_recv() {
                Ok(LoginMsg::Event(LoginEvent::Code { user_code, verification_uri, expires_in })) => {
                    login.code = Some(ShownCode {
                        uri_trusted: msauth::checked_verification_uri(&verification_uri).is_some(),
                        user_code,
                        uri: verification_uri,
                        expires: Instant::now() + Duration::from_secs(expires_in),
                    });
                }
                Ok(LoginMsg::Event(LoginEvent::Status(s))) => login.status = s.to_string(),
                Ok(LoginMsg::Done(result)) => {
                    done = Some(result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = Some(Err(AuthError::Failed("The sign-in task stopped unexpectedly.".into())));
                    break;
                }
            }
        }
        let Some(result) = done else {
            // Keeps the expiry countdown moving.
            ctx.request_repaint_after(Duration::from_millis(500));
            return;
        };
        self.login = None;
        match result {
            Ok(session) => self.finish_login(session),
            Err(AuthError::Cancelled) => {}
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn finish_login(&mut self, session: Session) {
        let name = session.name.clone();
        let stored = self.update_accounts(|a| a.store(&session));
        if stored {
            log::info(format!("Microsoft sign-in complete: {name}"));
            self.error = None;
            self.note = Some(format!("Signed in as {name}"));
            self.ms_name = Some(name);
        }
    }

    fn sign_out(&mut self) {
        if self.update_accounts(|a| {
            a.sign_out();
            Ok(())
        }) {
            log::info("Signed out of the Microsoft account");
            self.ms_name = None;
            self.note = Some("Signed out".into());
        }
    }

    // ------------------------------------------------------------ frame: sidebar and title bar

    fn sidebar(&mut self, ui: &mut Ui, p: &Palette) {
        // Logo and wordmark; dragging here moves the window.
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::click_and_drag());
        if resp.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let logo_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 50.0), Vec2::splat(86.0));
        theme::radial(ui.painter(), logo_rect.center(), vec2(80.0, 70.0), p.soft_glow);
        if let Some(logo) = &self.logo {
            logo.paint(ui, logo_rect, self.accent);
        }
        theme::wordmark(ui.painter(), pos2(rect.center().x, rect.top() + 124.0), 2.6, p);
        ui.add_space(18.0);

        let busy = self.busy();
        ui.add_enabled_ui(!busy, |ui| {
            for (tab, icon, label) in [(Tab::Home, Icon::Home, "Home"), (Tab::Profiles, Icon::Profiles, "Profiles"), (Tab::Settings, Icon::Settings, "Settings")] {
                if nav_item(ui, icon, label, self.tab == tab, p).clicked() {
                    self.tab = tab;
                }
                ui.add_space(-4.0);
            }
        });

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let (dot, status) = if busy {
                (p.c, "Launching…")
            } else if self.ms_name.is_some() {
                (theme::GREEN, "Ready to play")
            } else {
                (theme::AMBER, "Sign in to play")
            };
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let painter = ui.painter();
            let c = pos2(row.left() + 8.0, row.center().y);
            theme::glow(painter, Rect::from_center_size(c, Vec2::splat(8.0)), 4.0, theme::mul_alpha(dot, 0.6), 10.0);
            painter.circle_filled(c, 4.0, dot);
            painter.text(pos2(row.left() + 22.0, row.center().y), egui::Align2::LEFT_CENTER, status, theme::regular(12.0), TEXT_2);
            let line_y = row.top() - 8.0;
            painter.hline(row.x_range(), line_y, Stroke::new(1.0_f32, white(18)));
            ui.add_space(14.0);
            if self.account_chip(ui, p).clicked() && !busy {
                self.tab = Tab::Settings;
            }
        });
    }

    fn account_chip(&self, ui: &mut Ui, p: &Palette) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
        let painter = ui.painter();
        painter.rect_filled(rect, 14.0, white(if resp.hovered() { 14 } else { 8 }));
        painter.rect_stroke(rect, 14.0, Stroke::new(1.0_f32, white(if resp.hovered() { 40 } else { 22 })));
        let avatar = Rect::from_min_size(pos2(rect.left() + 12.0, rect.center().y - 18.0), Vec2::splat(36.0));
        avatar_tile(painter, avatar, 9.0, p, &initials(self.ms_name.as_deref()), 13.0);
        let (name, sub) = match &self.ms_name {
            Some(n) => (n.as_str(), "Microsoft account"),
            None => ("Not signed in", "Open Settings"),
        };
        let x = avatar.right() + 12.0;
        painter.text(pos2(x, rect.center().y - 9.0), egui::Align2::LEFT_CENTER, name, theme::regular(14.0), TEXT);
        painter.text(pos2(x, rect.center().y + 10.0), egui::Align2::LEFT_CENTER, sub, theme::regular(12.0), TEXT_3);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    /// The top row of the content area: a page title or eyebrow, the window buttons, and
    /// empty space that drags the window.
    fn title_bar(&mut self, ui: &mut Ui) {
        let (rect, drag) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click_and_drag());
        let maximized = is_maximized(ui.ctx());
        if drag.double_clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let painter = ui.painter().clone();
        match self.tab {
            Tab::Home => {
                let spaced: String = "MINECRAFT 1.7.10  ·  POT PVP".chars().flat_map(|c| [c, '\u{200A}']).collect();
                painter.text(pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, spaced, theme::regular(11.5), TEXT_3);
            }
            Tab::Profiles => {
                painter.text(pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, "Profiles", theme::semibold(26.0), TEXT);
            }
            Tab::Settings => {
                painter.text(pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, "Settings", theme::semibold(26.0), TEXT);
            }
        }
        // Window buttons in a thin pill.
        let buttons = [
            (Icon::Minimize, "Minimize", egui::ViewportCommand::Minimized(true)),
            if maximized {
                (Icon::Restore, "Restore", egui::ViewportCommand::Maximized(false))
            } else {
                (Icon::Maximize, "Maximize", egui::ViewportCommand::Maximized(true))
            },
            (Icon::Close, "Close", egui::ViewportCommand::Close),
        ];
        let pill_w = 8.0 + 42.0 * buttons.len() as f32;
        let pill = Rect::from_min_size(pos2(rect.right() - pill_w, rect.center().y - 19.0), vec2(pill_w, 38.0));
        painter.rect_filled(pill, 12.0, white(8));
        painter.rect_stroke(pill, 12.0, Stroke::new(1.0_f32, white(22)));
        for (i, (icon, label, cmd)) in buttons.into_iter().enumerate() {
            let b = Rect::from_min_size(pos2(pill.left() + 4.0 + i as f32 * 42.0, pill.top() + 4.0), vec2(42.0, 30.0));
            let resp = ui.interact(b, ui.id().with(label), Sense::click()).on_hover_text(label);
            let is_close = matches!(icon, Icon::Close);
            if resp.hovered() {
                let fill = if is_close { Color32::from_rgba_unmultiplied(232, 17, 35, 210) } else { white(20) };
                painter.rect_filled(b, 8.0, fill);
            }
            let size = if is_close || matches!(icon, Icon::Minimize) { 15.0 } else { 13.0 };
            theme::icons::paint(&painter, icon, Rect::from_center_size(b.center(), Vec2::splat(size)), Color32::from_rgb(214, 214, 220));
            if resp.clicked() {
                ui.ctx().send_viewport_cmd(cmd);
            }
        }
    }

    /// Errors and short notes, under the title bar.
    fn banners(&mut self, ui: &mut Ui) {
        if let Some(err) = self.error.clone() {
            ui.add_space(6.0);
            let frame = egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(255, 61, 82, 26))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 120, 135, 90)))
                .rounding(12.0)
                .inner_margin(egui::Margin::symmetric(14.0, 10.0));
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 90.0;
                    ui.add_sized([w, 0.0], egui::Label::new(RichText::new(err).color(theme::ERROR)).wrap());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                });
            });
        } else if let Some(note) = self.note.clone() {
            ui.add_space(6.0);
            theme::card_frame().inner_margin(egui::Margin::symmetric(14.0, 8.0)).rounding(12.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(note).color(TEXT_2));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                            self.note = None;
                        }
                    });
                });
            });
        }
    }

    // ------------------------------------------------------------ Home

    fn home(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(22.0);
        let mut job = egui::text::LayoutJob::default();
        let fmt = |color| egui::TextFormat { font_id: theme::semibold(44.0), color, ..Default::default() };
        job.append("Welcome back", 0.0, fmt(TEXT));
        match &self.ms_name {
            Some(name) => job.append(&format!(", {name}."), 0.0, fmt(Color32::from_rgb(111, 111, 120))),
            None => job.append(".", 0.0, fmt(Color32::from_rgb(111, 111, 120))),
        }
        ui.label(job);
        ui.add_space(-4.0);
        ui.label(RichText::new("Pick a profile and play. Lean closes once Minecraft starts.").font(theme::light(17.0)).color(TEXT_2));
        let (line, _) = ui.allocate_exact_size(vec2(110.0, 2.0), Sense::hover());
        theme::glow(ui.painter(), line, 1.0, p.glow, 10.0);
        theme::h_gradient(ui.painter(), line, 1.0, |t| theme::mul_alpha(p.c, 1.0 - t));
        ui.add_space(16.0);

        // Profile and version cards.
        let gap = 16.0;
        let w = (ui.available_width() - gap) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            self.profile_card(ui, w, p);
            version_card(ui, w);
        });
        ui.add_space(18.0);

        let busy = self.busy();
        if busy {
            let (frac, status) = self.progress.snapshot();
            theme::progress_bar(ui, p, frac, &status, 76.0);
        } else if self.ms_name.is_none() {
            if theme::play_button(ui, p, PlayLabel::Text("Sign in with Microsoft to play"), 76.0).clicked() {
                self.tab = Tab::Settings;
            }
        } else if theme::play_button(ui, p, PlayLabel::Play, 76.0).clicked() {
            let ctx = ui.ctx().clone();
            self.start_play(&ctx);
        }
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 26.0;
            let optifine = match (&self.optifine_jar, self.settings.use_optifine) {
                (Some(_), true) => "OptiFine HD U E7".to_string(),
                _ => "OptiFine off".to_string(),
            };
            stat(ui, Icon::Chip, &format!("{} MB memory", self.settings.memory_mb));
            stat(ui, Icon::Spark, &optifine);
            stat(ui, Icon::Cup, "Java 8, bundled");
            stat(ui, Icon::Power, "Closes when the game starts");
        });
        ui.add_space(22.0);

        ui.add_enabled_ui(!busy, |ui| self.feature_tiles(ui, p));
    }

    fn profile_card(&mut self, ui: &mut Ui, w: f32, p: &Palette) {
        let name = self.profiles.get(self.sel).map(|x| x.name.clone()).unwrap_or_default();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 76.0), Sense::click());
        let painter = ui.painter();
        select_card(painter, rect, resp.hovered());
        let icon_box = Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - 24.0), Vec2::splat(48.0));
        painter.rect_filled(icon_box, 12.0, p.soft);
        painter.rect_stroke(icon_box, 12.0, Stroke::new(1.0_f32, white(20)));
        theme::icons::paint(painter, Icon::User, Rect::from_center_size(icon_box.center(), Vec2::splat(22.0)), p.icon);
        let x = icon_box.right() + 16.0;
        painter.text(pos2(x, rect.center().y - 12.0), egui::Align2::LEFT_CENTER, "Profile", theme::regular(12.0), TEXT_3);
        painter.text(pos2(x, rect.center().y + 11.0), egui::Align2::LEFT_CENTER, &name, theme::regular(19.0), TEXT);
        theme::icons::paint(painter, Icon::Chevron, Rect::from_center_size(pos2(rect.right() - 28.0, rect.center().y), Vec2::splat(18.0)), TEXT_2);

        let popup_id = ui.id().with("profile-popup");
        if resp.clicked() && !self.busy() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let mut pick = None;
        let mut manage = false;
        egui::popup_below_widget(ui, popup_id, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(w - 12.0);
            for (i, prof) in self.profiles.iter().enumerate() {
                if ui.add(egui::SelectableLabel::new(i == self.sel, RichText::new(&prof.name).size(15.0))).clicked() {
                    pick = Some(i);
                }
            }
            ui.separator();
            if ui.add(egui::Button::new(RichText::new("Manage profiles…").color(TEXT_2)).frame(false)).clicked() {
                manage = true;
            }
        });
        if let Some(i) = pick {
            if i != self.sel {
                self.select(i);
            }
        }
        if manage {
            self.tab = Tab::Profiles;
        }
    }

    fn feature_tiles(&mut self, ui: &mut Ui, p: &Palette) {
        ui.horizontal(|ui| {
            theme::section_label(ui, "IN THIS PROFILE");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("Edit HUD layout").color(TEXT_2).size(13.0)).frame(false)).clicked() {
                    self.tab = Tab::Profiles;
                }
            });
        });
        ui.add_space(2.0);
        let Some(prof) = self.profiles.get_mut(self.sel) else { return };
        let details = [
            if prof.toggle_sprint.show_status { "Status shown" } else { "Status hidden" }.to_string(),
            if prof.cps.show_right { "Left and right click" } else { "Left click" }.to_string(),
            if prof.coords.show_direction { "With direction" } else { "Position only" }.to_string(),
            "Splash Healing II".to_string(),
            format!("{} adds · {} lists", key_label(&prof.keys.add_waypoint), key_label(&prof.keys.waypoint_menu)),
        ];
        let gap = 12.0;
        let w = (ui.available_width() - 4.0 * gap) / 5.0;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (k, detail) in details.iter().enumerate() {
                let (icon, name, flag) = match k {
                    0 => (Icon::Bolt, "ToggleSprint", &mut prof.toggle_sprint.enabled),
                    1 => (Icon::Mouse, "CPS counter", &mut prof.cps.enabled),
                    2 => (Icon::Compass, "Coordinates", &mut prof.coords.enabled),
                    3 => (Icon::Flask, "Pot counter", &mut prof.pot_counter.enabled),
                    _ => (Icon::Pin, "Waypoints", &mut prof.waypoints.enabled),
                };
                changed |= feature_tile(ui, w, icon, name, detail, flag, p);
            }
        });
        if changed {
            self.save_due = Some(Instant::now());
        }
    }

    // ------------------------------------------------------------ Profiles

    fn profiles_page(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(12.0);
        let list_w = 200.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            ui.vertical(|ui| {
                ui.set_width(list_w);
                self.profile_list(ui, p);
            });
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                self.profile_editor(ui, p);
            });
        });
    }

    fn profile_list(&mut self, ui: &mut Ui, p: &Palette) {
        let mut clicked = None;
        theme::card_frame().inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            for (i, prof) in self.profiles.iter().enumerate() {
                let on = [prof.toggle_sprint.enabled, prof.cps.enabled, prof.coords.enabled, prof.pot_counter.enabled, prof.waypoints.enabled]
                    .iter()
                    .filter(|b| **b)
                    .count();
                if list_row(ui, &prof.name, &format!("{on} on"), i == self.sel, p).clicked() {
                    clicked = Some(i);
                }
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
            let painter = ui.painter();
            painter.rect_stroke(rect, 11.0, Stroke::new(1.0_f32, white(if resp.hovered() { 50 } else { 26 })));
            theme::icons::paint(painter, Icon::Plus, Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(16.0)), TEXT_2);
            painter.text(pos2(rect.left() + 40.0, rect.center().y), egui::Align2::LEFT_CENTER, "New profile", theme::regular(14.0), TEXT_2);
            if resp.clicked() {
                self.dialog_text.clear();
                self.dialog = Dialog::New;
            }
        });
        if let Some(i) = clicked {
            if i != self.sel {
                self.select(i);
            }
        }
    }

    fn profile_editor(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(name) = self.profiles.get(self.sel).map(|x| x.name.clone()) else { return };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                theme::section_label(ui, "EDITING");
                ui.label(RichText::new(&name).font(theme::semibold(22.0)).color(TEXT));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_delete = self.profiles.len() > 1;
                if ui.add_enabled(can_delete, egui::Button::new(RichText::new("Delete").color(theme::ERROR))).clicked() {
                    self.dialog = Dialog::Delete;
                }
                if ui.button("Duplicate").clicked() {
                    self.duplicate();
                }
                if ui.button("Rename").clicked() {
                    self.dialog_text = name.clone();
                    self.dialog = Dialog::Rename;
                }
            });
        });
        ui.add_space(4.0);

        let Some(prof) = self.profiles.get_mut(self.sel) else { return };
        if self.key_buf_id != prof.id {
            self.key_buf_id = prof.id.clone();
            self.key_bufs = [prof.keys.add_waypoint.clone(), prof.keys.waypoint_menu.clone(), prof.keys.hud_editor.clone()];
        }
        let mut ch = hud_preview(ui, prof, p);
        ui.add_space(6.0);

        let key_bufs = &mut self.key_bufs;
        columns2(ui, |cols| {
            theme::card_frame().show(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                theme::section_label(ui, "HUD ELEMENTS");
                ch |= element_row(ui, "ToggleSprint", Some(("Status", &mut prof.toggle_sprint.show_status)), &mut prof.toggle_sprint.enabled, p);
                ch |= element_row(ui, "CPS counter", Some(("Right click", &mut prof.cps.show_right)), &mut prof.cps.enabled, p);
                ch |= element_row(ui, "Coordinates", Some(("Direction", &mut prof.coords.show_direction)), &mut prof.coords.enabled, p);
                ch |= element_row(ui, "Pot counter", None, &mut prof.pot_counter.enabled, p);
                ch |= element_row(ui, "Waypoints", Some(("Distance", &mut prof.waypoints.show_distance)), &mut prof.waypoints.enabled, p);
            });
            let ui = &mut cols[1];
            theme::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::section_label(ui, "HUD LOOK");
                ui.horizontal(|ui| {
                    ui.label("Text color");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&prof.hud.color).color(TEXT_2));
                        let mut rgb = parse_hex(&prof.hud.color);
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            prof.hud.color = format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                            ch = true;
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Text shadow");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ch |= theme::switch(ui, &mut prof.hud.shadow, p).changed();
                    });
                });
            });
            ui.add_space(6.0);
            theme::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::section_label(ui, "KEYS");
                let [a, b, c] = key_bufs;
                ch |= key_field(ui, "Add waypoint", a, &mut prof.keys.add_waypoint);
                ch |= key_field(ui, "Waypoint list", b, &mut prof.keys.waypoint_menu);
                ch |= key_field(ui, "HUD editor", c, &mut prof.keys.hud_editor);
                ui.label(RichText::new("ToggleSprint uses the game's own sprint key.").size(12.0).color(TEXT_3));
            });
        });
        if ch {
            self.save_due = Some(Instant::now());
        }
    }

    // ------------------------------------------------------------ Settings

    fn settings_page(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(12.0);
        columns2(ui, |cols| {
            let (left, right) = cols.split_at_mut(1);
            let (left, right) = (&mut left[0], &mut right[0]);
            theme::card_frame().show(left, |ui| {
                ui.set_width(ui.available_width());
                theme::section_label(ui, "ACCOUNT");
                self.account_section(ui, p);
            });
            left.add_space(6.0);
            theme::card_frame().show(left, |ui| {
                ui.set_width(ui.available_width());
                theme::section_label(ui, "GAME");
                self.game_section(ui);
            });
            theme::card_frame().show(right, |ui| {
                ui.set_width(ui.available_width());
                theme::section_label(ui, "APPEARANCE");
                self.appearance_section(ui);
            });
            right.add_space(6.0);
            theme::card_frame().show(right, |ui| {
                ui.set_width(ui.available_width());
                theme::section_label(ui, "OPTIFINE");
                self.optifine_section(ui, p);
            });
        });
    }

    fn appearance_section(&mut self, ui: &mut Ui) {
        ui.label("Accent");
        let gap = 10.0;
        let w = (ui.available_width() - 3.0 * gap) / 4.0;
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for a in Accent::ALL {
                let sp = a.palette();
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 84.0), Sense::click());
                let painter = ui.painter();
                let on = a == self.accent;
                if on {
                    theme::glow(painter, rect, 14.0, sp.soft_glow, 24.0);
                }
                painter.rect_filled(rect, 14.0, white(if resp.hovered() { 12 } else { 7 }));
                painter.rect_stroke(rect, 14.0, Stroke::new(1.0_f32, white(if on { 80 } else if resp.hovered() { 40 } else { 20 })));
                let c = pos2(rect.center().x, rect.top() + 30.0);
                theme::radial(painter, c, Vec2::splat(26.0), sp.glow);
                painter.circle_filled(c, 16.0, sp.dark);
                theme::radial(painter, c + vec2(-4.0, -5.0), Vec2::splat(15.0), sp.c);
                painter.text(pos2(rect.center().x, rect.bottom() - 18.0), egui::Align2::CENTER_CENTER, a.label(), theme::regular(12.5), TEXT);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    pick = Some(a);
                }
            }
        });
        if let Some(a) = pick {
            self.set_accent(a);
        }
        ui.label(RichText::new("Tints the background glow, active items and the Play button.").size(12.0).color(TEXT_3));
    }

    fn game_section(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Memory");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("MB").color(TEXT_2));
                ui.label(RichText::new(self.settings.memory_mb.to_string()).font(theme::regular(20.0)).color(TEXT));
            });
        });
        let mut mem = self.settings.memory_mb;
        ui.spacing_mut().slider_width = ui.available_width();
        let r = ui.add(egui::Slider::new(&mut mem, MEM_MIN..=MEM_MAX).step_by(MEM_STEP as f64).show_value(false));
        if r.changed() {
            self.settings.memory_mb = mem;
            self.save_settings();
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(MEM_MIN.to_string()).size(12.0).color(TEXT_3));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(MEM_MAX.to_string()).size(12.0).color(TEXT_3));
            });
        });
        ui.label(RichText::new("2048 MB suits 1.7.10 with OptiFine. More rarely helps and can add GC stutter.").size(12.0).color(TEXT_3));
    }

    fn optifine_section(&mut self, ui: &mut Ui, p: &Palette) {
        match self.optifine_jar.clone() {
            Some(n) => {
                egui::Frame::none().fill(egui::Color32::from_black_alpha(60)).stroke(Stroke::new(1.0_f32, white(18))).rounding(12.0).inner_margin(egui::Margin::symmetric(12.0, 10.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                        theme::icons::paint(ui.painter(), Icon::File, r, TEXT_2);
                        ui.add(egui::Label::new(RichText::new(n).size(13.0)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("IMPORTED").size(11.0).color(theme::GREEN));
                        });
                    });
                });
            }
            None => {
                ui.label(RichText::new("No OptiFine jar imported yet.").color(TEXT_2));
            }
        }
        let has_jar = self.optifine_jar.is_some();
        ui.horizontal(|ui| {
            ui.label("Use OptiFine");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut on = self.settings.use_optifine && has_jar;
                if ui.add_enabled_ui(has_jar, |ui| theme::switch(ui, &mut on, p)).inner.changed() {
                    self.settings.use_optifine = on;
                    self.save_settings();
                }
            });
        });
        ui.horizontal(|ui| {
            let label = if has_jar { "Import another jar…" } else { "Import OptiFine…" };
            if ui.button(label).clicked() {
                let pick = rfd::FileDialog::new().add_filter("Java archive", &["jar"]).pick_file();
                if let Some(path) = pick {
                    match prepare::import_optifine(&self.paths, &path) {
                        Ok(dest) => {
                            self.optifine_jar = dest.file_name().map(|n| n.to_string_lossy().into_owned());
                            self.note = Some(format!("Imported {}", self.optifine_jar.clone().unwrap_or_default()));
                            self.error = None;
                        }
                        Err(e) => self.fail(format!("OptiFine import failed: {e:#}")),
                    }
                }
            }
            ui.label(RichText::new("1.7.10 HD U E7, from optifine.net").size(12.0).color(TEXT_3));
        });
    }

    fn account_section(&mut self, ui: &mut Ui, p: &Palette) {
        if let Some(name) = self.ms_name.clone() {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                theme::glow(ui.painter(), r, 12.0, p.soft_glow, 22.0);
                avatar_tile(ui.painter(), r, 12.0, p, &initials(Some(&name)), 16.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(4.0);
                    ui.label(RichText::new(&name).font(theme::regular(17.0)).color(TEXT));
                    ui.label(RichText::new("Microsoft · owns Minecraft: Java Edition").size(13.0).color(TEXT_2));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Sign out").clicked() {
                        self.sign_out();
                    }
                });
            });
            ui.label(
                RichText::new("Your sign-in is stored encrypted for your Windows user and never written to logs.").size(12.5).color(TEXT_3),
            );
            return;
        }
        if self.login.is_some() {
            self.login_panel(ui, p);
            return;
        }
        ui.label(RichText::new("Play needs the Microsoft account that owns Minecraft: Java Edition.").color(TEXT_2));
        let resolved = msauth::resolve_client_id(&self.settings);
        let Some(client) = resolved.filter(|_| !self.client_id_edit) else {
            let can_cancel = self.client_id_edit;
            self.client_id_entry(ui, can_cancel);
            return;
        };
        let changeable = client.source != ClientIdSource::Env;
        if !msauth::valid_client_id(&client.id) {
            ui.colored_label(theme::ERROR, format!("The Microsoft app ID ({}) is not a GUID.", client.source.describe()));
            if changeable && ui.button("Change app ID").clicked() {
                self.client_id_buf.clear();
                self.client_id_edit = true;
            }
            return;
        }
        if accent_button(ui, "Sign in with Microsoft", p).clicked() {
            let ctx = ui.ctx().clone();
            self.start_login(&ctx, client.id.clone());
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("App ID {} ({})", client.id, client.source.describe())).size(12.0).color(TEXT_3));
            if changeable && ui.small_button("Change").clicked() {
                self.client_id_buf = client.id.clone();
                self.client_id_edit = true;
            }
        });
    }

    fn client_id_entry(&mut self, ui: &mut Ui, can_cancel: bool) {
        ui.label(
            RichText::new(
                "Microsoft sign-in needs an app ID: the Application (client) ID of an Azure app registration \
                 that Mojang has approved for Minecraft. Paste it here.",
            )
            .size(13.0)
            .color(TEXT_3),
        );
        let mut save = false;
        ui.horizontal(|ui| {
            let w = ui.available_width() - if can_cancel { 150.0 } else { 70.0 };
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.client_id_buf)
                    .hint_text("00000000-0000-0000-0000-000000000000")
                    .desired_width(w)
                    .char_limit(64),
            );
            let ok = msauth::valid_client_id(self.client_id_buf.trim());
            save = ui.add_enabled(ok, egui::Button::new("Save")).clicked()
                || (ok && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if can_cancel && ui.button("Cancel").clicked() {
                self.client_id_edit = false;
            }
        });
        let typed = self.client_id_buf.trim().to_ascii_lowercase();
        if !typed.is_empty() && !msauth::valid_client_id(&typed) {
            ui.colored_label(theme::ERROR, "Not an app ID: expected a GUID like 00000000-0000-0000-0000-000000000000");
        }
        if save {
            self.settings.ms_client_id = Some(typed);
            self.save_settings();
            self.client_id_edit = false;
            self.client_id_buf.clear();
        }
    }

    fn login_panel(&mut self, ui: &mut Ui, p: &Palette) {
        let Some(login) = &self.login else { return };
        let mut cancel = false;
        let mut copied = None;
        let mut open_error = None;
        match &login.code {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&login.status);
                    cancel = ui.button("Cancel").clicked();
                });
            }
            Some(code) => {
                ui.label(RichText::new("Open the sign-in page, enter this code, and sign in with the Microsoft account that owns Minecraft:").color(TEXT_2));
                egui::Frame::none()
                    .fill(egui::Color32::from_black_alpha(70))
                    .stroke(Stroke::new(1.0_f32, theme::mul_alpha(p.c, 0.5)))
                    .rounding(12.0)
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&code.user_code).monospace().size(30.0).strong().color(TEXT));
                    });
                ui.horizontal(|ui| {
                    if ui.add_enabled(code.uri_trusted, egui::Button::new("Open sign-in page")).clicked() {
                        open_error = msauth::open_in_browser(&code.uri).err();
                    }
                    if ui.button("Copy code").clicked() {
                        ui.ctx().copy_text(code.user_code.clone());
                        copied = Some(());
                    }
                    cancel = ui.button("Cancel").clicked();
                });
                ui.label(RichText::new(format!("Sign-in page: {}", code.uri)).size(12.0).color(TEXT_3));
                if !code.uri_trusted {
                    ui.colored_label(theme::ERROR, "That address is not a Microsoft sign-in page, so the launcher won't open it.");
                }
                let left = code.expires.saturating_duration_since(Instant::now()).as_secs();
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("{} (code expires in {}:{:02})", login.status, left / 60, left % 60)).color(TEXT_2));
                });
            }
        }
        if copied.is_some() {
            self.note = Some("Code copied".into());
        }
        if let Some(e) = open_error {
            self.fail(e);
        }
        if cancel {
            self.cancel_login();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if matches!(self.dialog, Dialog::None) {
            return;
        }
        let title = match self.dialog {
            Dialog::New => "New profile",
            Dialog::Rename => "Rename profile",
            _ => "Delete profile",
        };
        let mut close = false;
        let mut confirm = false;
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_min_width(300.0);
            if matches!(self.dialog, Dialog::Delete) {
                let name = self.profiles.get(self.sel).map(|p| p.name.clone()).unwrap_or_default();
                ui.label(format!("Delete \"{name}\"? This cannot be undone."));
            } else {
                let r = ui.add(egui::TextEdit::singleline(&mut self.dialog_text).char_limit(32).hint_text("Profile name").desired_width(300.0));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirm = true;
                }
            }
            ui.horizontal(|ui| {
                let ok_enabled = matches!(self.dialog, Dialog::Delete) || !self.dialog_text.trim().is_empty();
                if ui.add_enabled(ok_enabled, egui::Button::new("OK")).clicked() {
                    confirm = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if confirm {
            let name = self.dialog_text.trim().to_string();
            match self.dialog {
                Dialog::New if !name.is_empty() => {
                    let id = profile::unique_id(&profile::slugify(&name), &self.ids());
                    self.add_profile(Profile::new(&id, &name));
                }
                Dialog::Rename if !name.is_empty() => {
                    if let Some(p) = self.profiles.get_mut(self.sel) {
                        p.name = name;
                    }
                    self.save_profile_now();
                }
                Dialog::Delete => self.delete_selected(),
                _ => {}
            }
            close = true;
        }
        if close {
            self.dialog = Dialog::None;
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [8.0 / 255.0, 8.0 / 255.0, 10.0 / 255.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.styled_for != Some(self.accent) {
            theme::apply_style(ctx, self.accent);
            self.styled_for = Some(self.accent);
        }
        if !self.logo_tried {
            self.logo_tried = true;
            self.logo = Logo::load(ctx);
            ctx.options_mut(|o| o.zoom_with_keyboard = false); // the zoom follows the window size
        }
        let maximized = is_maximized(ctx);
        let size = fit_zoom(ctx);
        if !maximized {
            self.window_size = Some(size);
        }
        self.poll_job(ctx);
        self.poll_login(ctx);
        if let Some(t) = self.save_due {
            if t.elapsed() >= SAVE_DELAY {
                self.save_profile_now();
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        }
        let p = self.accent.palette();
        let screen = ctx.screen_rect();
        theme::background(&ctx.layer_painter(egui::LayerId::background()), screen, &p);

        egui::SidePanel::left("nav")
            .exact_width(212.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(white(4)).inner_margin(egui::Margin { left: 18.0, right: 18.0, top: 18.0, bottom: 20.0 }))
            .show(ctx, |ui| self.sidebar(ui, &p));
        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(egui::Margin { left: 36.0, right: 20.0, top: 12.0, bottom: 20.0 }))
            .show(ctx, |ui| {
                self.title_bar(ui);
                self.banners(ui);
                match self.tab {
                    Tab::Home => {
                        ui.add_space(0.0);
                        // Only the right padding differs from the other pages, so the content lines up.
                        egui::Frame::none().inner_margin(egui::Margin { right: 16.0, ..Default::default() }).show(ui, |ui| self.home(ui, &p));
                    }
                    Tab::Profiles => {
                        let busy = self.busy();
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.add_enabled_ui(!busy, |ui| {
                                egui::Frame::none().inner_margin(egui::Margin { right: 16.0, ..Default::default() }).show(ui, |ui| self.profiles_page(ui, &p));
                            });
                        });
                    }
                    Tab::Settings => {
                        let busy = self.busy();
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.add_enabled_ui(!busy, |ui| {
                                egui::Frame::none().inner_margin(egui::Margin { right: 16.0, ..Default::default() }).show(ui, |ui| self.settings_page(ui, &p));
                            });
                        });
                    }
                }
            });
        if !maximized {
            // A hairline around the frameless window, and its edges resize it.
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("window-border")))
                .rect_stroke(screen.shrink(0.5), 0.0, Stroke::new(1.0_f32, white(30)));
            resize_edges(ctx);
        }
        self.dialogs(ctx);
        if self.dev_shot.is_some() {
            self.dev_screenshot(ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel_login();
        if self.save_due.is_some() {
            self.save_profile_now();
        }
        if let Some(size) = self.window_size.map(|s| s.map(f32::round)) {
            if self.settings.window_size != Some(size) && self.dev_shot.is_none() {
                self.settings.window_size = Some(size);
                let _ = self.settings.save(&self.paths);
            }
        }
    }
}

fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Scales the UI so the design size fits the window, in 5% steps (each step re-rasterizes the
/// fonts, so a smooth drag does not redo it every frame). Returns the window size in points at
/// 100%.
fn fit_zoom(ctx: &egui::Context) -> [f32; 2] {
    let zoom = ctx.zoom_factor();
    let size = ctx.screen_rect().size() * zoom;
    let fit = (size.x / WINDOW_SIZE[0]).min(size.y / WINDOW_SIZE[1]);
    let target = ((fit * 20.0 + 0.001).floor() / 20.0).clamp(0.5, 4.0);
    if (target - zoom).abs() > 0.001 {
        ctx.set_zoom_factor(target);
    }
    [size.x, size.y]
}

/// The window has no system frame, so its edges and corners are resize handles drawn by hand.
fn resize_edges(ctx: &egui::Context) {
    use egui::ResizeDirection as D;
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.screen_rect();
    let m = RESIZE_MARGIN / ctx.zoom_factor();
    let (left, right) = (pos.x - r.left() < m, r.right() - pos.x < m);
    let (top, bottom) = (pos.y - r.top() < m, r.bottom() - pos.y < m);
    let dir = match (left, right, top, bottom) {
        (true, _, true, _) => D::NorthWest,
        (_, true, true, _) => D::NorthEast,
        (true, _, _, true) => D::SouthWest,
        (_, true, _, true) => D::SouthEast,
        (true, ..) => D::West,
        (_, true, ..) => D::East,
        (_, _, true, _) => D::North,
        (.., true) => D::South,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        D::North | D::South => egui::CursorIcon::ResizeVertical,
        D::East | D::West => egui::CursorIcon::ResizeHorizontal,
        D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
        D::NorthEast | D::SouthWest => egui::CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
    }
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(image.as_raw())?;
    Ok(())
}

// ---------------------------------------------------------------- widgets

/// Two columns whose contents lay out top-down and left-aligned. (`Ui::columns` justifies,
/// which stretches wrapped text.)
fn columns2(ui: &mut Ui, add: impl FnOnce(&mut [Ui])) {
    let gap = 16.0;
    let avail = ui.available_rect_before_wrap();
    let w = (avail.width() - gap) / 2.0;
    let mut cols: Vec<Ui> = (0..2)
        .map(|i| {
            let rect = Rect::from_min_size(pos2(avail.left() + i as f32 * (w + gap), avail.top()), vec2(w, avail.height()));
            ui.new_child(egui::UiBuilder::new().id_salt(("col", i)).max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)))
        })
        .collect();
    add(&mut cols);
    let bottom = cols.iter().map(|c| c.min_rect().bottom()).fold(avail.top(), f32::max);
    ui.allocate_rect(Rect::from_min_max(avail.min, pos2(avail.right(), bottom)), Sense::hover());
}

fn initials(name: Option<&str>) -> String {
    match name {
        Some(n) => n.chars().filter(|c| c.is_ascii_alphanumeric()).take(2).collect::<String>().to_uppercase(),
        None => "?".into(),
    }
}

fn avatar_tile(painter: &egui::Painter, rect: Rect, radius: f32, p: &Palette, text: &str, size: f32) {
    let (c, dark) = (p.c, p.dark);
    theme::h_gradient(painter, rect, radius, |t| {
        let k = t.clamp(0.0, 1.0);
        Color32::from_rgb(
            (c.r() as f32 + (dark.r() as f32 - c.r() as f32) * k) as u8,
            (c.g() as f32 + (dark.g() as f32 - c.g() as f32) * k) as u8,
            (c.b() as f32 + (dark.b() as f32 - c.b() as f32) * k) as u8,
        )
    });
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::semibold(size), p.ink);
}

fn nav_item(ui: &mut Ui, icon: Icon, label: &str, selected: bool, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let painter = ui.painter();
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.12);
    if selected {
        painter.rect_filled(rect, 12.0, white(10));
        painter.rect_stroke(rect, 12.0, Stroke::new(1.0_f32, white(30)));
        let bar = Rect::from_min_max(pos2(rect.left() - 1.0, rect.top() + 11.0), pos2(rect.left() + 2.0, rect.bottom() - 11.0));
        theme::glow(painter, bar, 1.5, theme::mul_alpha(p.glow, 0.6), 8.0);
        painter.rect_filled(bar, 1.5, p.c);
    } else if hover > 0.0 {
        painter.rect_filled(rect, 12.0, white((7.0 * hover) as u8));
    }
    let color = if selected || resp.hovered() { TEXT } else { TEXT_2 };
    theme::icons::paint(painter, icon, Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), Vec2::splat(20.0)), color);
    let font = if selected { theme::semibold(14.0) } else { theme::regular(14.0) };
    painter.text(pos2(rect.left() + 50.0, rect.center().y), egui::Align2::LEFT_CENTER, label, font, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The glass card used for the Home selectors.
fn select_card(painter: &egui::Painter, rect: Rect, hovered: bool) {
    painter.rect_filled(rect, 16.0, white(if hovered { 13 } else { 9 }));
    painter.rect_stroke(rect, 16.0, Stroke::new(1.0_f32, white(if hovered { 48 } else { 26 })));
}

fn version_card(ui: &mut Ui, w: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, 76.0), Sense::hover());
    let painter = ui.painter();
    select_card(painter, rect, false);
    let icon_box = Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - 24.0), Vec2::splat(48.0));
    painter.rect_filled(icon_box, 12.0, white(12));
    painter.rect_stroke(icon_box, 12.0, Stroke::new(1.0_f32, white(20)));
    theme::icons::paint(painter, Icon::Cube, Rect::from_center_size(icon_box.center(), Vec2::splat(22.0)), Color32::from_rgb(212, 212, 218));
    let x = icon_box.right() + 16.0;
    painter.text(pos2(x, rect.center().y - 12.0), egui::Align2::LEFT_CENTER, "Minecraft version", theme::regular(12.0), TEXT_3);
    painter.text(pos2(x, rect.center().y + 11.0), egui::Align2::LEFT_CENTER, "1.7.10", theme::regular(19.0), TEXT);
    let galley = painter.layout_no_wrap("1.8.9 SOON".into(), theme::regular(11.0), TEXT_2);
    let pill = Rect::from_min_size(pos2(rect.right() - 20.0 - galley.size().x - 20.0, rect.center().y - 12.0), vec2(galley.size().x + 20.0, 24.0));
    painter.rect_stroke(pill, 12.0, Stroke::new(1.0_f32, white(36)));
    painter.galley(pill.center() - galley.size() / 2.0, galley, TEXT_2);
}

fn stat(ui: &mut Ui, icon: Icon, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        theme::icons::paint(ui.painter(), icon, r, TEXT_2);
        ui.label(RichText::new(text).size(13.0).color(TEXT_2));
    });
}

/// A Home tile that turns one HUD element on or off. Returns true when it changed.
fn feature_tile(ui: &mut Ui, w: f32, icon: Icon, name: &str, detail: &str, on: &mut bool, p: &Palette) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 100.0), Sense::click());
    let mut changed = false;
    if resp.clicked() {
        *on = !*on;
        changed = true;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 14.0, white(if resp.hovered() { 12 } else { 8 }));
    let edge = if *on { 44 } else { 20 } + if resp.hovered() { 20 } else { 0 };
    painter.rect_stroke(rect, 14.0, Stroke::new(1.0_f32, white(edge)));
    let icon_color = if *on { p.icon } else { TEXT_3 };
    theme::icons::paint(painter, icon, Rect::from_min_size(rect.min + vec2(16.0, 16.0), Vec2::splat(20.0)), icon_color);
    let sw = Rect::from_min_size(pos2(rect.right() - 16.0 - 36.0, rect.top() + 16.0), vec2(36.0, 20.0));
    theme::paint_switch(ui, sw, *on, resp.hovered(), resp.id, p);
    let painter = ui.painter();
    painter.text(pos2(rect.left() + 16.0, rect.bottom() - 38.0), egui::Align2::LEFT_CENTER, name, theme::regular(14.0), TEXT);
    let detail_galley = painter.layout(detail.to_string(), theme::regular(12.0), TEXT_3, rect.width() - 32.0);
    painter.galley(pos2(rect.left() + 16.0, rect.bottom() - 26.0), detail_galley, TEXT_3);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, name));
    changed
}

fn list_row(ui: &mut Ui, name: &str, meta: &str, selected: bool, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 11.0, white(10));
        painter.rect_stroke(rect, 11.0, Stroke::new(1.0_f32, white(36)));
        let bar = Rect::from_min_max(pos2(rect.left() - 1.0, rect.top() + 11.0), pos2(rect.left() + 2.0, rect.bottom() - 11.0));
        painter.rect_filled(bar, 1.5, p.c);
    } else if resp.hovered() {
        painter.rect_filled(rect, 11.0, white(8));
    }
    let color = if selected { TEXT } else { Color32::from_rgb(196, 196, 204) };
    let font = if selected { theme::semibold(14.0) } else { theme::regular(14.0) };
    painter.text(pos2(rect.left() + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, name, font, color);
    painter.text(pos2(rect.right() - 14.0, rect.center().y), egui::Align2::RIGHT_CENTER, meta, theme::regular(11.0), TEXT_3);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A smaller accent button (Sign in).
fn accent_button(ui: &mut Ui, text: &str, p: &Palette) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::semibold(14.0), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 40.0, 42.0), Sense::click());
    let painter = ui.painter();
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.15);
    theme::glow(painter, rect, 12.0, theme::mul_alpha(p.glow, 0.6 + 0.4 * hover), 22.0);
    let (deep, end) = (p.deep, p.btn_end);
    theme::h_gradient(painter, rect, 12.0, |t| {
        let k = (t * 0.8 + 0.2 * hover).clamp(0.0, 1.0);
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
        Color32::from_rgba_premultiplied(l(deep.r(), end.r()), l(deep.g(), end.g()), l(deep.b(), end.b()), l(deep.a(), end.a()))
    });
    painter.rect_stroke(rect, 12.0, Stroke::new(1.0_f32, white(50)));
    painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One HUD element in the Profiles page: name, an optional option chip, and an on/off switch.
fn element_row(ui: &mut Ui, name: &str, option: Option<(&str, &mut bool)>, enabled: &mut bool, p: &Palette) -> bool {
    let mut ch = false;
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.label(RichText::new(name).color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ch |= theme::switch(ui, enabled, p).changed();
            if let Some((label, value)) = option {
                ui.add_enabled_ui(*enabled, |ui| {
                    ch |= chip(ui, label, value, p);
                });
            }
        });
    });
    ch
}

/// A small pill that toggles an option.
fn chip(ui: &mut Ui, text: &str, on: &mut bool, p: &Palette) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::regular(12.0), TEXT);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, 26.0), Sense::click());
    let mut changed = false;
    if resp.clicked() {
        *on = !*on;
        changed = true;
    }
    let painter = ui.painter();
    let enabled = ui.is_enabled();
    if *on && enabled {
        painter.rect_filled(rect, 13.0, p.soft);
        painter.rect_stroke(rect, 13.0, Stroke::new(1.0_f32, theme::mul_alpha(p.c, 0.6)));
    } else {
        painter.rect_stroke(rect, 13.0, Stroke::new(1.0_f32, white(if resp.hovered() { 50 } else { 26 })));
    }
    let color = if !enabled { white(60) } else if *on { TEXT } else { TEXT_3 };
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, text));
    changed
}

/// A preview of the in-game HUD. Elements can be dragged; positions are percentages of the
/// screen, as in the profile file. Returns true when anything moved.
fn hud_preview(ui: &mut Ui, prof: &mut Profile, p: &Palette) -> bool {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, (w * 9.0 / 16.0).round()), Sense::hover());
    // In game the font is 9 of the 240 scaled pixels on a 1080p screen; match that here.
    let font_px = (rect.height() * 9.0 / 240.0 * 0.95).max(8.0);
    let painter = ui.painter().clone();
    theme::glow(&painter, rect, 16.0, Color32::from_black_alpha(120), 40.0);
    theme::glow(&painter, rect, 16.0, theme::mul_alpha(p.soft_glow, 0.35), 24.0);
    let horizon = rect.top() + rect.height() * 0.47;
    let sky = Rect::from_min_max(rect.min, pos2(rect.right(), horizon));
    let ground = Rect::from_min_max(pos2(rect.left(), horizon), rect.max);
    painter.rect_filled(sky, egui::Rounding { nw: 16.0, ne: 16.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(52, 74, 106));
    // Light on the horizon, kept inside the sky.
    let sky_light = painter.with_clip_rect(sky.intersect(painter.clip_rect()));
    theme::radial(&sky_light, pos2(rect.center().x, horizon), vec2(rect.width() * 0.6, rect.height() * 0.45), Color32::from_rgba_unmultiplied(120, 160, 205, 150));
    painter.rect_filled(ground, egui::Rounding { nw: 0.0, ne: 0.0, sw: 16.0, se: 16.0 }, Color32::from_rgb(33, 45, 30));
    let stripes = 12;
    for i in 0..stripes {
        if i % 2 == 0 {
            let x0 = ground.left() + ground.width() * i as f32 / stripes as f32;
            let r = Rect::from_min_max(pos2(x0, horizon), pos2(x0 + ground.width() / stripes as f32, horizon + 6.0));
            painter.rect_filled(r, 0.0, Color32::from_black_alpha(40));
        }
    }
    // Crosshair and hotbar.
    let c = rect.center();
    painter.line_segment([c - vec2(7.0, 0.0), c + vec2(7.0, 0.0)], Stroke::new(2.0_f32, white(200)));
    painter.line_segment([c - vec2(0.0, 7.0), c + vec2(0.0, 7.0)], Stroke::new(2.0_f32, white(200)));
    let hb = Rect::from_center_size(pos2(c.x, rect.bottom() - rect.height() * 0.08), vec2(w * 0.44, w * 0.44 / 9.0));
    painter.rect_filled(hb, 0.0, Color32::from_black_alpha(110));
    painter.rect_stroke(hb, 0.0, Stroke::new(2.0_f32, Color32::from_black_alpha(160)));
    for i in 1..9 {
        let x = hb.left() + hb.width() * i as f32 / 9.0;
        painter.line_segment([pos2(x, hb.top()), pos2(x, hb.bottom())], Stroke::new(1.0_f32, white(30)));
    }
    painter.rect_stroke(Rect::from_min_size(hb.min, vec2(hb.width() / 9.0, hb.height())).expand(1.0), 0.0, Stroke::new(2.0_f32, Color32::WHITE));
    // A waypoint marker in the world, when waypoints are on.
    if prof.waypoints.enabled {
        let m = pos2(rect.left() + w * 0.64, rect.top() + rect.height() * 0.36);
        theme::icons::paint(&painter, Icon::Pin, Rect::from_center_size(m, Vec2::splat(20.0)), p.c);
        let label = if prof.waypoints.show_distance { "Base · 31 m" } else { "Base" };
        painter.text(m + vec2(0.0, 18.0), egui::Align2::CENTER_CENTER, label, theme::semibold(11.0), Color32::WHITE);
    }

    let rgb = parse_hex(&prof.hud.color);
    let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    let shadow = prof.hud.shadow;
    let mut changed = false;
    let elements: [(bool, String, &mut f64, &mut f64); 4] = [
        (
            prof.toggle_sprint.enabled,
            if prof.toggle_sprint.show_status { "[Sprinting (Toggled)]".into() } else { "Sprinting".into() },
            &mut prof.toggle_sprint.x,
            &mut prof.toggle_sprint.y,
        ),
        (prof.cps.enabled, if prof.cps.show_right { "12 | 4 CPS".into() } else { "12 CPS".into() }, &mut prof.cps.x, &mut prof.cps.y),
        (
            prof.coords.enabled,
            if prof.coords.show_direction { "X: 112  Y: 64  Z: -340  NE".into() } else { "X: 112  Y: 64  Z: -340".into() },
            &mut prof.coords.x,
            &mut prof.coords.y,
        ),
        (prof.pot_counter.enabled, "Pots: 14".into(), &mut prof.pot_counter.x, &mut prof.pot_counter.y),
    ];
    for (i, (enabled, text, x, y)) in elements.into_iter().enumerate() {
        if !enabled {
            continue;
        }
        let galley = painter.layout_no_wrap(text, theme::semibold(font_px), color);
        let size = galley.size() + vec2(4.0, 0.0);
        let max_x = ((rect.width() - size.x) / rect.width() * 100.0).max(0.0) as f64;
        let max_y = ((rect.height() - size.y) / rect.height() * 100.0).max(0.0) as f64;
        let pos = |x: f64, y: f64| pos2(rect.left() + (x.min(max_x) as f32) / 100.0 * rect.width(), rect.top() + (y.min(max_y) as f32) / 100.0 * rect.height());
        let el = Rect::from_min_size(pos(*x, *y), size);
        let resp = ui.interact(el, ui.id().with(("hud", i)), Sense::drag());
        if resp.dragged() {
            let d = resp.drag_delta();
            *x = (*x + (d.x / rect.width() * 100.0) as f64).clamp(0.0, max_x);
            *y = (*y + (d.y / rect.height() * 100.0) as f64).clamp(0.0, max_y);
            *x = (*x * 10.0).round() / 10.0;
            *y = (*y * 10.0).round() / 10.0;
            changed = true;
        }
        let el = Rect::from_min_size(pos(*x, *y), size);
        let active = resp.hovered() || resp.dragged();
        if active {
            painter.rect_filled(el, 2.0, white(26));
            painter.rect_stroke(el, 2.0, Stroke::new(1.0_f32, white(200)));
        }
        let text_pos = el.min + vec2(2.0, 0.0);
        if shadow {
            painter.galley_with_override_text_color(text_pos + vec2(1.0, 1.0), galley.clone(), Color32::from_black_alpha(190));
        }
        painter.galley(text_pos, galley, color);
        resp.on_hover_cursor(egui::CursorIcon::Grab);
    }
    // Hint badge.
    let hint = "Drag to move  ·  in game: Right Shift";
    let g = painter.layout_no_wrap(hint.into(), theme::regular(12.0), Color32::from_rgb(228, 228, 232));
    let badge = Rect::from_min_size(pos2(rect.right() - g.size().x - 34.0, rect.top() + 12.0), g.size() + vec2(22.0, 12.0));
    painter.rect_filled(badge, 13.0, Color32::from_rgba_unmultiplied(8, 8, 10, 170));
    painter.rect_stroke(badge, 13.0, Stroke::new(1.0_f32, white(36)));
    painter.galley(badge.min + vec2(11.0, 6.0), g, Color32::WHITE);
    painter.rect_stroke(rect, 16.0, Stroke::new(1.0_f32, white(40)));
    changed
}

fn parse_hex(s: &str) -> [u8; 3] {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).ok().filter(|_| s.trim_start_matches('#').len() == 6);
    let v = v.unwrap_or(0xFFFFFF);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// How a key name reads on screen ("RSHIFT" → "Right Shift").
fn key_label(name: &str) -> String {
    match name {
        "RSHIFT" => "Right Shift".into(),
        "LSHIFT" => "Left Shift".into(),
        "RCONTROL" => "Right Ctrl".into(),
        "LCONTROL" => "Left Ctrl".into(),
        "NONE" => "unbound".into(),
        other => other.to_string(),
    }
}

fn key_field(ui: &mut Ui, label: &str, buf: &mut String, target: &mut String) -> bool {
    let mut changed = false;
    let valid = keys::normalize(buf);
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut te = egui::TextEdit::singleline(buf).desired_width(110.0).char_limit(16).horizontal_align(egui::Align::Center);
            if valid.is_none() {
                te = te.text_color(theme::ERROR);
            }
            if ui.add(te).changed() {
                if let Some(name) = keys::normalize(buf) {
                    if *target != name {
                        *target = name;
                        changed = true;
                    }
                }
            }
        });
    });
    if valid.is_none() {
        ui.label(RichText::new("Unknown key name, not saved. For example B, F6, RSHIFT or NONE.").size(12.0).color(theme::ERROR));
    }
    changed
}
