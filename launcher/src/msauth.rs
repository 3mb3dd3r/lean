//! Microsoft account sign-in for Minecraft: OAuth device code flow on the Microsoft identity
//! platform (personal accounts), then Xbox Live, XSTS and Minecraft services.
//!
//! Tokens never reach the log, stdout or an error message. They live in [`Secret`], whose
//! `Debug` prints `<redacted>`; messages are built from HTTP status and error codes; any text a
//! server sends back goes through [`scrub`] first; request and response bodies are never logged.

use crate::log;
use crate::settings::Settings;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::ACCEPT;
use serde_json::{json, Value};
use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const SCOPE: &str = "XboxLive.signin offline_access";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Compiled-in Azure app (client) ID, used when neither `LEAN_MS_CLIENT_ID` nor launcher.json
/// `msClientId` is set. Empty until this project's own app registration has Mojang's approval.
/// Never put another application's ID here (the official launcher's, say): Microsoft's terms forbid it.
pub const BUILTIN_CLIENT_ID: &str = "";
pub const CLIENT_ID_ENV: &str = "LEAN_MS_CLIENT_ID";

/// Renew the Minecraft token when it expires within this many seconds.
pub const RENEW_MARGIN_SECS: i64 = 600;

pub const NO_CLIENT_ID: &str = "Microsoft sign-in needs an app ID: the Application (client) ID of an Azure app \
registration that Mojang has approved for Minecraft. Paste it under Settings > Account, or set LEAN_MS_CLIENT_ID.";
pub const NOT_APPROVED: &str =
    "Your Microsoft app ID isn't approved for Minecraft sign-in yet. Mojang has to approve it first.";
pub const NO_GAME: &str = "This Microsoft account doesn't own Minecraft: Java Edition.";
pub const SIGN_IN_AGAIN: &str =
    "Your Microsoft sign-in has expired or was revoked. Sign in again under Settings > Account (or run with --login).";
const DECLINED: &str = "The sign-in was declined on the Microsoft page. Start it again if that was a mistake.";
const CODE_EXPIRED: &str = "The sign-in code expired before it was used. Start the sign-in again.";
const BAD_CODE: &str = "Microsoft didn't recognize the sign-in code. Start the sign-in again.";

// ---------------------------------------------------------------- secrets and errors

/// A token. `Debug` prints `<redacted>` and there is no `Display`, so it cannot slip into a log
/// line or a message by accident; [`Secret::expose`] is the only way to read it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The player cancelled the sign-in.
    Cancelled,
    /// The stored refresh token was refused (expired or revoked): the player has to sign in again.
    SignInAgain(String),
    /// A service could not be reached or is overloaded; trying again later may work.
    Network(String),
    Failed(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::Cancelled => f.write_str("Sign-in cancelled."),
            AuthError::SignInAgain(m) | AuthError::Network(m) | AuthError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for AuthError {}

fn failed(m: impl Into<String>) -> AuthError {
    AuthError::Failed(m.into())
}

// ---------------------------------------------------------------- app (client) ID

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientIdSource {
    Env,
    Settings,
    BuiltIn,
}

impl ClientIdSource {
    pub fn describe(self) -> &'static str {
        match self {
            ClientIdSource::Env => "from LEAN_MS_CLIENT_ID",
            ClientIdSource::Settings => "from launcher.json",
            ClientIdSource::BuiltIn => "built in",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientId {
    pub id: String,
    pub source: ClientIdSource,
}

/// First non-blank of: the environment variable, launcher.json `msClientId`, the built-in ID.
pub fn resolve_client_id_from(env: Option<&str>, settings: Option<&str>, builtin: &str) -> Option<ClientId> {
    [(env, ClientIdSource::Env), (settings, ClientIdSource::Settings), (Some(builtin), ClientIdSource::BuiltIn)]
        .into_iter()
        .find_map(|(value, source)| {
            let id = value?.trim();
            (!id.is_empty()).then(|| ClientId { id: id.to_string(), source })
        })
}

pub fn resolve_client_id(settings: &Settings) -> Option<ClientId> {
    let env = std::env::var(CLIENT_ID_ENV).ok();
    resolve_client_id_from(env.as_deref(), settings.ms_client_id.as_deref(), BUILTIN_CLIENT_ID)
}

/// The app ID to sign in with, format-checked. The value itself is not echoed on error.
pub fn client_id(settings: &Settings) -> Result<ClientId, AuthError> {
    let c = resolve_client_id(settings).ok_or_else(|| failed(NO_CLIENT_ID))?;
    if !valid_client_id(&c.id) {
        return Err(failed(format!(
            "The Microsoft app ID ({}) is not a GUID like 00000000-0000-0000-0000-000000000000.",
            c.source.describe()
        )));
    }
    Ok(c)
}

/// Basic format check: a GUID, hex digits in groups of 8-4-4-4-12.
pub fn valid_client_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

// ---------------------------------------------------------------- sign-in page

const SIGN_IN_DOMAINS: &[&str] = &["microsoft.com", "live.com", "microsoftonline.com"];

/// The sign-in page address, normalized, when it is `https://` on microsoft.com, live.com or
/// microsoftonline.com (or a subdomain) with no user name and no port; otherwise None.
pub fn checked_verification_uri(uri: &str) -> Option<String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c);
    if uri.is_empty() || uri.len() > 300 || !uri.chars().all(allowed) {
        return None;
    }
    let url = reqwest::Url::parse(uri).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let trusted = SIGN_IN_DOMAINS
        .iter()
        .any(|d| host == *d || host.strip_suffix(d).is_some_and(|rest| rest.ends_with('.')));
    trusted.then(|| url.to_string())
}

/// Opens a checked sign-in page in the default browser (`rundll32 url.dll,FileProtocolHandler`).
pub fn open_in_browser(uri: &str) -> Result<(), String> {
    let url = checked_verification_uri(uri)
        .ok_or_else(|| "That is not a Microsoft sign-in address, so it was not opened.".to_string())?;
    let rundll = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("rundll32.exe"))
        .unwrap_or_else(|| PathBuf::from("rundll32.exe"));
    Command::new(rundll)
        .arg("url.dll,FileProtocolHandler")
        .arg(&url)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not open the browser: {e}"))
}

// ---------------------------------------------------------------- response parsing (no network)

#[derive(Debug)]
pub struct DeviceCode {
    pub user_code: String,
    device_code: Secret,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug)]
pub struct MsTokens {
    pub access_token: Secret,
    pub refresh_token: Secret,
}

#[derive(Debug)]
pub enum Poll {
    Pending,
    SlowDown,
    Done(MsTokens),
}

#[derive(Debug)]
pub struct XboxToken {
    pub token: Secret,
    pub uhs: String,
}

#[derive(Debug)]
pub struct McToken {
    pub access_token: Secret,
    pub expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McProfile {
    /// No dashes, lowercase.
    pub id: String,
    pub name: String,
}

fn parse_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or(Value::Null)
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn number(v: &Value, key: &str) -> Option<u64> {
    match v.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Server text made safe for a message: first line, cut before the trace details, long
/// token-like words replaced, at most 200 characters.
fn scrub(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    let line = line.split(" Trace ID:").next().unwrap_or(line);
    let words: Vec<&str> = line
        .split_whitespace()
        .map(|w| if w.len() >= 40 && !(w.starts_with("https://") && w.len() < 100) { "<hidden>" } else { w })
        .collect();
    let out = words.join(" ");
    if out.chars().count() > 200 {
        out.chars().take(200).collect::<String>() + "..."
    } else {
        out
    }
}

/// `AADSTS<digits>` at the start of a Microsoft error description.
fn aadsts_code(description: &str) -> Option<&str> {
    let start = description.find("AADSTS")?;
    let rest = &description[start..];
    let end = rest[6..].find(|c: char| !c.is_ascii_digit()).map_or(rest.len(), |i| i + 6);
    (end > 6).then(|| &rest[..end])
}

struct OAuthError {
    status: u16,
    error: String,
    description: String,
}

fn oauth_error(status: u16, body: &str) -> OAuthError {
    let v = parse_json(body);
    OAuthError {
        status,
        error: text(&v, "error").unwrap_or_default().to_string(),
        description: text(&v, "error_description").unwrap_or_default().to_string(),
    }
}

/// Plain message for an error from the device code or token endpoint.
fn oauth_failure(e: &OAuthError) -> AuthError {
    let code = aadsts_code(&e.description);
    let mut tag = format!("HTTP {}", e.status);
    for part in [Some(e.error.as_str()).filter(|s| !s.is_empty()), code].into_iter().flatten() {
        tag.push_str(", ");
        tag.push_str(part);
    }
    if code == Some("AADSTS7000218") {
        return failed(format!(
            "The Azure app has to allow public client flows: turn on \"Allow public client flows\" under \
             Authentication in the Azure portal ({tag})."
        ));
    }
    if matches!(e.error.as_str(), "unauthorized_client" | "invalid_client") || code == Some("AADSTS700016") {
        return failed(format!(
            "Microsoft doesn't recognize this app ID, or the app isn't set up for personal Microsoft accounts ({tag}). \
             Check the Application (client) ID of the Azure app registration, and that it allows personal Microsoft accounts."
        ));
    }
    if e.error == "invalid_scope" {
        return failed(format!("Microsoft refused the Xbox Live permission for this app ({tag})."));
    }
    if e.status == 429 || e.status >= 500 {
        return AuthError::Network(format!("Microsoft sign-in is unavailable right now ({tag}). Try again in a minute."));
    }
    match scrub(&e.description) {
        d if d.is_empty() => failed(format!("Microsoft sign-in failed ({tag}).")),
        d => failed(format!("Microsoft sign-in failed ({tag}): {d}")),
    }
}

/// Status errors from Xbox Live and Minecraft services, which send little or no body.
fn service_error(service: &str, status: u16) -> AuthError {
    match status {
        429 => AuthError::Network(format!("{service} is limiting sign-ins (HTTP 429). Wait a minute and try again.")),
        s if s >= 500 => AuthError::Network(format!("{service} is unavailable right now (HTTP {s}). Try again later.")),
        s => failed(format!("{service} refused the sign-in (HTTP {s}).")),
    }
}

pub fn parse_device_code(status: u16, body: &str) -> Result<DeviceCode, AuthError> {
    if status != 200 {
        return Err(oauth_failure(&oauth_error(status, body)));
    }
    let v = parse_json(body);
    let user_code = text(&v, "user_code").map(str::trim).filter(|c| c.len() <= 32);
    match (user_code, text(&v, "device_code"), text(&v, "verification_uri")) {
        (Some(user_code), Some(device_code), Some(uri)) => Ok(DeviceCode {
            user_code: user_code.to_string(),
            device_code: Secret::new(device_code),
            verification_uri: uri.to_string(),
            expires_in: number(&v, "expires_in").unwrap_or(900),
            interval: number(&v, "interval").unwrap_or(5),
        }),
        _ => Err(failed("Microsoft sent an unexpected answer to the sign-in request (no code).")),
    }
}

fn parse_tokens(body: &str) -> Result<MsTokens, AuthError> {
    let v = parse_json(body);
    let access = text(&v, "access_token").ok_or_else(|| failed("Microsoft's answer had no access token."))?;
    let refresh = text(&v, "refresh_token")
        .ok_or_else(|| failed("Microsoft's answer had no refresh token (the offline_access permission is missing)."))?;
    Ok(MsTokens { access_token: Secret::new(access), refresh_token: Secret::new(refresh) })
}

/// One answer from the token endpoint while waiting for the player (step 2).
pub fn parse_poll(status: u16, body: &str) -> Result<Poll, AuthError> {
    if status == 200 {
        return parse_tokens(body).map(Poll::Done);
    }
    let e = oauth_error(status, body);
    match e.error.as_str() {
        "authorization_pending" => Ok(Poll::Pending),
        "slow_down" => Ok(Poll::SlowDown),
        "authorization_declined" | "access_denied" => Err(failed(DECLINED)),
        "expired_token" => Err(failed(CODE_EXPIRED)),
        "bad_verification_code" => Err(failed(BAD_CODE)),
        _ => Err(oauth_failure(&e)),
    }
}

/// The token endpoint's answer to a refresh (step 3).
pub fn parse_refresh(status: u16, body: &str) -> Result<MsTokens, AuthError> {
    if status == 200 {
        return parse_tokens(body);
    }
    let e = oauth_error(status, body);
    match e.error.as_str() {
        "invalid_grant" | "interaction_required" | "consent_required" | "login_required" => {
            Err(AuthError::SignInAgain(SIGN_IN_AGAIN.into()))
        }
        _ => Err(oauth_failure(&e)),
    }
}

fn parse_xbox_token(body: &str, service: &str) -> Result<XboxToken, AuthError> {
    let v = parse_json(body);
    let uhs = v.pointer("/DisplayClaims/xui/0/uhs").and_then(Value::as_str).filter(|s| !s.is_empty());
    match (text(&v, "Token"), uhs) {
        (Some(token), Some(uhs)) => Ok(XboxToken { token: Secret::new(token), uhs: uhs.to_string() }),
        _ => Err(failed(format!("{service} sent an unexpected answer (no token)."))),
    }
}

/// Xbox Live user authentication (step 4).
pub fn parse_xbl(status: u16, body: &str) -> Result<XboxToken, AuthError> {
    if status == 200 {
        return parse_xbox_token(body, "Xbox Live");
    }
    match number(&parse_json(body), "XErr") {
        Some(x) => Err(failed(format!("Xbox Live refused the sign-in (HTTP {status}, XErr {x})."))),
        None => Err(service_error("Xbox Live", status)),
    }
}

/// Plain message for an XSTS `XErr` code.
pub fn xsts_message(code: u64) -> String {
    match code {
        2148916233 => "This Microsoft account has no Xbox profile yet. Sign in once at xbox.com to create one, then try again.".into(),
        2148916235 => "Xbox Live isn't available in this account's country or region, so it can't sign in to Minecraft.".into(),
        2148916236 | 2148916237 => "This account needs adult verification on the Xbox website before it can sign in.".into(),
        2148916238 => "This is a child account. An adult has to add it to a Microsoft family (account.microsoft.com/family) before it can sign in.".into(),
        other => format!("Xbox Live refused the sign-in (XErr {other})."),
    }
}

/// XSTS authorization (step 5).
pub fn parse_xsts(status: u16, body: &str) -> Result<XboxToken, AuthError> {
    if status == 200 {
        return parse_xbox_token(body, "Xbox Live (XSTS)");
    }
    match number(&parse_json(body), "XErr") {
        Some(code) => Err(failed(xsts_message(code))),
        None => Err(service_error("Xbox Live (XSTS)", status)),
    }
}

/// Minecraft login with the XSTS token (step 6).
pub fn parse_mc_login(status: u16, body: &str) -> Result<McToken, AuthError> {
    match status {
        200 => {
            let v = parse_json(body);
            let token = text(&v, "access_token").ok_or_else(|| failed("Minecraft sent an unexpected answer (no token)."))?;
            Ok(McToken { access_token: Secret::new(token), expires_in: number(&v, "expires_in").unwrap_or(86_400) })
        }
        403 => Err(failed(NOT_APPROVED)),
        s => Err(service_error("Minecraft", s)),
    }
}

fn valid_uuid(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_profile_name(s: &str) -> bool {
    (1..=16).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The Minecraft profile (step 7).
pub fn parse_profile(status: u16, body: &str) -> Result<McProfile, AuthError> {
    match status {
        200 => {}
        404 => return Err(failed(NO_GAME)),
        s => return Err(service_error("The Minecraft profile service", s)),
    }
    let v = parse_json(body);
    let id = text(&v, "id").map(|s| s.replace('-', "").to_ascii_lowercase());
    match (id, text(&v, "name")) {
        (Some(id), Some(name)) if valid_uuid(&id) && valid_profile_name(name) => Ok(McProfile { id, name: name.to_string() }),
        _ => Err(failed("Minecraft sent an unexpected profile.")),
    }
}

// ---------------------------------------------------------------- HTTP

struct Api {
    client: Client,
}

impl Api {
    fn new() -> Result<Self, AuthError> {
        Client::builder()
            .user_agent(concat!("lean-launcher/", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .map(|client| Self { client })
            .map_err(|e| failed(format!("Could not set up HTTPS: {}", e.without_url())))
    }

    /// Status and body. The body is only ever parsed, never logged.
    fn send(&self, service: &str, req: RequestBuilder) -> Result<(u16, String), AuthError> {
        let resp = req.header(ACCEPT, "application/json").send().map_err(|e| network(service, e))?;
        let status = resp.status().as_u16();
        let body = resp.text().map_err(|e| network(service, e))?;
        Ok((status, body))
    }

    fn device_code(&self, client_id: &str) -> Result<DeviceCode, AuthError> {
        let form = [("client_id", client_id), ("scope", SCOPE)];
        let (status, body) = self.send("Microsoft sign-in", self.client.post(DEVICE_CODE_URL).form(&form))?;
        parse_device_code(status, &body)
    }

    fn poll(&self, client_id: &str, dc: &DeviceCode) -> Result<Poll, AuthError> {
        let form = [("grant_type", DEVICE_CODE_GRANT), ("client_id", client_id), ("device_code", dc.device_code.expose())];
        let (status, body) = self.send("Microsoft sign-in", self.client.post(TOKEN_URL).form(&form))?;
        parse_poll(status, &body)
    }

    fn refresh(&self, client_id: &str, refresh_token: &Secret) -> Result<MsTokens, AuthError> {
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
            ("client_id", client_id),
            ("scope", SCOPE),
        ];
        let (status, body) = self.send("Microsoft sign-in", self.client.post(TOKEN_URL).form(&form))?;
        parse_refresh(status, &body)
    }

    fn xbox_live(&self, ms_access_token: &Secret) -> Result<XboxToken, AuthError> {
        let body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={}", ms_access_token.expose()),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        });
        let (status, body) = self.send("Xbox Live", self.client.post(XBL_URL).json(&body))?;
        parse_xbl(status, &body)
    }

    fn xsts(&self, xbl: &XboxToken) -> Result<XboxToken, AuthError> {
        let body = json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token.expose()] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        });
        let (status, body) = self.send("Xbox Live (XSTS)", self.client.post(XSTS_URL).json(&body))?;
        parse_xsts(status, &body)
    }

    fn minecraft_login(&self, xsts: &XboxToken) -> Result<McToken, AuthError> {
        let body = json!({ "identityToken": format!("XBL3.0 x={};{}", xsts.uhs, xsts.token.expose()) });
        let (status, body) = self.send("Minecraft services", self.client.post(MC_LOGIN_URL).json(&body))?;
        parse_mc_login(status, &body)
    }

    fn minecraft_profile(&self, mc_token: &Secret) -> Result<McProfile, AuthError> {
        let req = self.client.get(MC_PROFILE_URL).bearer_auth(mc_token.expose());
        let (status, body) = self.send("The Minecraft profile service", req)?;
        parse_profile(status, &body)
    }
}

/// Transport failure. The URL is dropped (`service` names it); the causes (DNS, TLS, ...)
/// carry no request data.
fn network(service: &str, e: reqwest::Error) -> AuthError {
    let kind = if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "could not connect"
    } else {
        "connection failed"
    };
    let e = e.without_url();
    let mut causes = Vec::new();
    let mut source = std::error::Error::source(&e);
    while let Some(s) = source {
        causes.push(s.to_string());
        source = s.source();
    }
    let detail = if causes.is_empty() { String::new() } else { format!(": {}", causes.join(": ")) };
    AuthError::Network(format!("Could not reach {service} ({kind}{detail}). Check the internet connection and try again."))
}

// ---------------------------------------------------------------- flows

/// A finished sign-in, ready to store.
#[derive(Clone, Debug)]
pub struct Session {
    pub name: String,
    /// No dashes.
    pub uuid: String,
    pub refresh_token: Secret,
    pub mc_token: Secret,
    /// Unix seconds.
    pub mc_expires: i64,
    /// The app ID that issued the refresh token; refreshing needs the same one.
    pub client_id: String,
}

pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// True when a Minecraft token expiring at `expires` has 10 minutes or less left at `now`.
pub fn needs_renewal(expires: i64, now: i64) -> bool {
    expires - now <= RENEW_MARGIN_SECS
}

/// What the sign-in shows while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginEvent {
    /// Show this code and page to the player.
    Code { user_code: String, verification_uri: String, expires_in: u64 },
    Status(&'static str),
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), AuthError> {
    if cancel.load(Ordering::Relaxed) {
        Err(AuthError::Cancelled)
    } else {
        Ok(())
    }
}

fn sleep_or_cancel(d: Duration, cancel: &AtomicBool) -> Result<(), AuthError> {
    let end = Instant::now() + d;
    loop {
        check_cancel(cancel)?;
        let now = Instant::now();
        if now >= end {
            return Ok(());
        }
        std::thread::sleep((end - now).min(Duration::from_millis(100)));
    }
}

/// Full sign-in (steps 1 to 7). `cancel` is checked between polls and between steps.
pub fn sign_in(client_id: &str, cancel: &AtomicBool, on_event: &mut dyn FnMut(LoginEvent)) -> Result<Session, AuthError> {
    let api = Api::new()?;
    log::info(format!("Microsoft sign-in: requesting a code (app ID {client_id})"));
    let dc = api.device_code(client_id)?;
    on_event(LoginEvent::Code {
        user_code: dc.user_code.clone(),
        verification_uri: dc.verification_uri.clone(),
        expires_in: dc.expires_in,
    });
    on_event(LoginEvent::Status("Waiting for you to sign in on the Microsoft page"));
    log::info("Microsoft sign-in: waiting for the player to enter the code");
    let ms = wait_for_tokens(&api, client_id, &dc, cancel)?;
    log::info("Microsoft sign-in: Microsoft account accepted");
    minecraft_session(&api, client_id, ms, cancel, on_event)
}

fn wait_for_tokens(api: &Api, client_id: &str, dc: &DeviceCode, cancel: &AtomicBool) -> Result<MsTokens, AuthError> {
    let deadline = Instant::now() + Duration::from_secs(dc.expires_in.clamp(30, 3600));
    let mut interval = dc.interval.clamp(1, 60);
    let mut network_failures = 0;
    loop {
        sleep_or_cancel(Duration::from_secs(interval), cancel)?;
        if Instant::now() >= deadline {
            return Err(failed(CODE_EXPIRED));
        }
        match api.poll(client_id, dc) {
            Ok(Poll::Done(tokens)) => return Ok(tokens),
            Ok(Poll::Pending) => network_failures = 0,
            Ok(Poll::SlowDown) => interval += 5,
            Err(AuthError::Network(m)) if network_failures < 3 => {
                network_failures += 1;
                log::warn(format!("Microsoft sign-in: {m} Retrying."));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Steps 4 to 7: Xbox Live, XSTS, Minecraft token, profile.
fn minecraft_session(
    api: &Api,
    client_id: &str,
    ms: MsTokens,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(LoginEvent),
) -> Result<Session, AuthError> {
    check_cancel(cancel)?;
    on_event(LoginEvent::Status("Signing in to Xbox Live"));
    log::info("Microsoft sign-in: Xbox Live");
    let xbl = api.xbox_live(&ms.access_token)?;
    check_cancel(cancel)?;
    let xsts = api.xsts(&xbl)?;
    check_cancel(cancel)?;
    on_event(LoginEvent::Status("Signing in to Minecraft"));
    log::info("Microsoft sign-in: Minecraft");
    let mc = api.minecraft_login(&xsts)?;
    check_cancel(cancel)?;
    on_event(LoginEvent::Status("Reading the Minecraft profile"));
    let profile = api.minecraft_profile(&mc.access_token)?;
    Ok(Session {
        name: profile.name,
        uuid: profile.id,
        refresh_token: ms.refresh_token,
        mc_token: mc.access_token,
        mc_expires: now_secs() + mc.expires_in.min(7 * 86_400) as i64,
        client_id: client_id.to_string(),
    })
}

/// Step 3, then 4 to 7: a new Minecraft token from a stored refresh token. `on_refresh_token`
/// receives the new refresh token as soon as Microsoft issues it, so it can be stored before
/// the remaining steps run.
pub fn renew(client_id: &str, refresh_token: &Secret, on_refresh_token: &mut dyn FnMut(&Secret)) -> Result<Session, AuthError> {
    let api = Api::new()?;
    log::info("Microsoft sign-in: renewing the Minecraft token");
    let ms = api.refresh(client_id, refresh_token)?;
    on_refresh_token(&ms.refresh_token);
    minecraft_session(&api, client_id, ms, &AtomicBool::new(false), &mut |_: LoginEvent| {})
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    const LONG_TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.c2lnbmF0dXJlLXRoYXQtaXMtbG9uZw";

    fn err(r: Result<impl fmt::Debug, AuthError>) -> AuthError {
        r.expect_err("expected an error")
    }

    #[test]
    fn device_code_response() {
        let body = r#"{"user_code":"ABCD1234","device_code":"DAQABAAEAAAD--secret-device-code",
            "verification_uri":"https://www.microsoft.com/link","expires_in":900,"interval":5,
            "message":"To sign in, use a web browser to open the page https://www.microsoft.com/link and enter the code ABCD1234 to authenticate."}"#;
        let dc = parse_device_code(200, body).unwrap();
        assert_eq!(dc.user_code, "ABCD1234");
        assert_eq!(dc.device_code.expose(), "DAQABAAEAAAD--secret-device-code");
        assert_eq!(dc.verification_uri, "https://www.microsoft.com/link");
        assert_eq!((dc.expires_in, dc.interval), (900, 5));
        assert!(!format!("{dc:?}").contains("secret-device-code"), "Debug must hide the device code");

        let defaults = parse_device_code(200, r#"{"user_code":"X","device_code":"d","verification_uri":"https://microsoft.com/devicelogin"}"#).unwrap();
        assert_eq!((defaults.expires_in, defaults.interval), (900, 5));
        assert!(matches!(err(parse_device_code(200, r#"{"user_code":"X"}"#)), AuthError::Failed(_)));
        assert!(matches!(err(parse_device_code(200, "not json")), AuthError::Failed(_)));
    }

    #[test]
    fn unknown_app_id_messages() {
        let consumers = r#"{"error":"unauthorized_client","error_description":"The client does not exist or is not enabled for consumers. If you are the application developer, configure a new application through the App Registrations in the Azure Portal at https://go.microsoft.com/fwlink/?linkid=2083908."}"#;
        let m = err(parse_device_code(400, consumers)).to_string();
        assert!(m.starts_with("Microsoft doesn't recognize this app ID"), "{m}");
        assert!(m.contains("HTTP 400, unauthorized_client"), "{m}");

        let aad = r#"{"error":"invalid_request","error_description":"AADSTS700016: Application with identifier 'x' was not found in the directory. Trace ID: 1 Correlation ID: 2"}"#;
        let m = err(parse_device_code(400, aad)).to_string();
        assert!(m.starts_with("Microsoft doesn't recognize this app ID"), "{m}");
        assert!(m.contains("AADSTS700016"), "{m}");

        let public = r#"{"error":"invalid_client","error_description":"AADSTS7000218: The request body must contain the following parameter: 'client_assertion' or 'client_secret'."}"#;
        let m = err(parse_poll(401, public)).to_string();
        assert!(m.contains("Allow public client flows"), "{m}");

        let scope = r#"{"error":"invalid_scope","error_description":"AADSTS70011: The provided value for scope is not valid."}"#;
        assert!(err(parse_device_code(400, scope)).to_string().contains("Xbox Live permission"));
        assert!(matches!(err(parse_device_code(503, "")), AuthError::Network(_)));
        assert_eq!(err(parse_device_code(400, "")).to_string(), "Microsoft sign-in failed (HTTP 400).");
    }

    #[test]
    fn poll_outcomes() {
        let e = |code: &str| format!(r#"{{"error":"{code}","error_description":"AADSTS70016: OAuth 2.0 device flow error."}}"#);
        assert!(matches!(parse_poll(400, &e("authorization_pending")), Ok(Poll::Pending)));
        assert!(matches!(parse_poll(400, &e("slow_down")), Ok(Poll::SlowDown)));
        assert_eq!(err(parse_poll(400, &e("authorization_declined"))).to_string(), DECLINED);
        assert_eq!(err(parse_poll(400, &e("access_denied"))).to_string(), DECLINED);
        assert_eq!(err(parse_poll(400, &e("expired_token"))).to_string(), CODE_EXPIRED);
        assert_eq!(err(parse_poll(400, &e("bad_verification_code"))).to_string(), BAD_CODE);

        let ok = r#"{"token_type":"Bearer","scope":"XboxLive.signin offline_access","expires_in":3600,
            "access_token":"EwAAA-ms-access","refresh_token":"M.C5_BAY.0.U.-refresh!*$"}"#;
        match parse_poll(200, ok).unwrap() {
            Poll::Done(t) => {
                assert_eq!(t.access_token.expose(), "EwAAA-ms-access");
                assert_eq!(t.refresh_token.expose(), "M.C5_BAY.0.U.-refresh!*$");
                assert_eq!(format!("{t:?}"), "MsTokens { access_token: <redacted>, refresh_token: <redacted> }");
            }
            other => panic!("unexpected {other:?}"),
        }
        let no_refresh = r#"{"access_token":"a","expires_in":3600}"#;
        assert!(err(parse_poll(200, no_refresh)).to_string().contains("offline_access"));
    }

    #[test]
    fn refresh_outcomes() {
        let t = parse_refresh(200, r#"{"access_token":"new-a","refresh_token":"new-r"}"#).unwrap();
        assert_eq!((t.access_token.expose(), t.refresh_token.expose()), ("new-a", "new-r"));
        let grant = r#"{"error":"invalid_grant","error_description":"AADSTS70000: The provided value for the input parameter 'refresh_token' is not valid."}"#;
        assert_eq!(err(parse_refresh(400, grant)), AuthError::SignInAgain(SIGN_IN_AGAIN.into()));
        assert!(matches!(err(parse_refresh(400, r#"{"error":"unauthorized_client"}"#)), AuthError::Failed(_)));
        assert!(matches!(err(parse_refresh(500, "")), AuthError::Network(_)));
    }

    #[test]
    fn server_text_is_scrubbed() {
        let body = format!(r#"{{"error":"weird_error","error_description":"Bad request for {LONG_TOKEN} here. Trace ID: abc Correlation ID: def"}}"#);
        let m = err(parse_poll(400, &body)).to_string();
        assert!(!m.contains(LONG_TOKEN), "{m}");
        assert!(m.contains("<hidden>") && !m.contains("Trace ID"), "{m}");
        assert!(m.starts_with("Microsoft sign-in failed (HTTP 400, weird_error): Bad request for <hidden> here."), "{m}");
        assert!(scrub(&"word ".repeat(100)).chars().count() <= 203);
        assert_eq!(scrub("see https://go.microsoft.com/fwlink/?linkid=2083908."), "see https://go.microsoft.com/fwlink/?linkid=2083908.");
    }

    #[test]
    fn xbox_live_responses() {
        let ok = r#"{"IssueInstant":"2026-01-01T00:00:00Z","NotAfter":"2026-01-15T00:00:00Z","Token":"xbl-token",
            "DisplayClaims":{"xui":[{"uhs":"1234567890"}]}}"#;
        let x = parse_xbl(200, ok).unwrap();
        assert_eq!((x.token.expose(), x.uhs.as_str()), ("xbl-token", "1234567890"));
        assert!(matches!(err(parse_xbl(200, r#"{"Token":"t"}"#)), AuthError::Failed(_)));
        assert_eq!(err(parse_xbl(401, "")).to_string(), "Xbox Live refused the sign-in (HTTP 401).");
        assert_eq!(err(parse_xbl(400, r#"{"XErr":2148916227}"#)).to_string(), "Xbox Live refused the sign-in (HTTP 400, XErr 2148916227).");
        assert!(matches!(err(parse_xbl(503, "")), AuthError::Network(_)));
    }

    #[test]
    fn xsts_responses_and_xerr_messages() {
        let ok = r#"{"Token":"xsts-token","DisplayClaims":{"xui":[{"uhs":"777"}]}}"#;
        let x = parse_xsts(200, ok).unwrap();
        assert_eq!((x.token.expose(), x.uhs.as_str()), ("xsts-token", "777"));

        let xerr = |code: u64| err(parse_xsts(401, &format!(r#"{{"Identity":"0","XErr":{code},"Message":"","Redirect":"https://start.ui.xboxlive.com/CreateAccount"}}"#))).to_string();
        assert!(xerr(2148916233).contains("no Xbox profile") && xerr(2148916233).contains("xbox.com"));
        assert!(xerr(2148916235).contains("isn't available in this account's country"));
        assert!(xerr(2148916236).contains("adult verification"));
        assert!(xerr(2148916237).contains("adult verification"));
        assert!(xerr(2148916238).contains("child account") && xerr(2148916238).contains("family"));
        assert_eq!(xerr(2148916239), "Xbox Live refused the sign-in (XErr 2148916239).");
        // Some proxies send the code as a string.
        assert!(err(parse_xsts(401, r#"{"XErr":"2148916233"}"#)).to_string().contains("no Xbox profile"));
        assert_eq!(err(parse_xsts(401, "")).to_string(), "Xbox Live (XSTS) refused the sign-in (HTTP 401).");
    }

    #[test]
    fn minecraft_login_responses() {
        let ok = r#"{"username":"0f0f0f0f-aaaa-bbbb-cccc-000000000000","roles":[],"access_token":"mc-token","token_type":"Bearer","expires_in":86400}"#;
        let t = parse_mc_login(200, ok).unwrap();
        assert_eq!((t.access_token.expose(), t.expires_in), ("mc-token", 86_400));
        assert_eq!(err(parse_mc_login(403, "")).to_string(), NOT_APPROVED);
        assert_eq!(
            NOT_APPROVED,
            "Your Microsoft app ID isn't approved for Minecraft sign-in yet. Mojang has to approve it first."
        );
        assert!(matches!(err(parse_mc_login(429, "")), AuthError::Network(_)));
        assert_eq!(err(parse_mc_login(400, "{}")).to_string(), "Minecraft refused the sign-in (HTTP 400).");
    }

    #[test]
    fn profile_responses() {
        let ok = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","skins":[],"capes":[]}"#;
        assert_eq!(
            parse_profile(200, ok).unwrap(),
            McProfile { id: "069a79f444e94726a5befca90e38aaf5".into(), name: "Notch".into() }
        );
        let dashed = r#"{"id":"069A79F4-44E9-4726-A5BE-FCA90E38AAF5","name":"Notch"}"#;
        assert_eq!(parse_profile(200, dashed).unwrap().id, "069a79f444e94726a5befca90e38aaf5");
        let not_found = r#"{"path":"/minecraft/profile","errorType":"NOT_FOUND","error":"NOT_FOUND"}"#;
        assert_eq!(err(parse_profile(404, not_found)).to_string(), NO_GAME);
        assert_eq!(NO_GAME, "This Microsoft account doesn't own Minecraft: Java Edition.");
        assert!(matches!(err(parse_profile(200, r#"{"id":"nothex","name":"Notch"}"#)), AuthError::Failed(_)));
        assert!(matches!(err(parse_profile(200, r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"bad name"}"#)), AuthError::Failed(_)));
        assert!(matches!(err(parse_profile(401, "")), AuthError::Failed(_)));
    }

    #[test]
    fn client_id_resolution_order() {
        let (a, b, c) = (
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222",
            "33333333-3333-3333-3333-333333333333",
        );
        let r = |env, set, built| resolve_client_id_from(env, set, built).map(|c| (c.id, c.source));
        assert_eq!(r(Some(a), Some(b), c), Some((a.to_string(), ClientIdSource::Env)));
        assert_eq!(r(None, Some(b), c), Some((b.to_string(), ClientIdSource::Settings)));
        assert_eq!(r(Some("  "), Some(b), c), Some((b.to_string(), ClientIdSource::Settings)));
        assert_eq!(r(None, None, c), Some((c.to_string(), ClientIdSource::BuiltIn)));
        assert_eq!(r(Some(""), Some(""), c), Some((c.to_string(), ClientIdSource::BuiltIn)));
        let padded = format!(" {b}\n");
        assert_eq!(r(None, Some(padded.as_str()), ""), Some((b.to_string(), ClientIdSource::Settings)));
        assert_eq!(r(None, None, ""), None);
        assert_eq!(BUILTIN_CLIENT_ID, "", "no app ID is registered yet");
    }

    #[test]
    fn client_id_format() {
        assert!(valid_client_id("00000000-0000-0000-0000-000000000000"));
        assert!(valid_client_id("9f86d081-884C-4d63-a1b2-0123456789ab"));
        assert!(!valid_client_id("9f86d081884c4d63a1b20123456789ab"));
        assert!(!valid_client_id("{9f86d081-884c-4d63-a1b2-0123456789ab}"));
        assert!(!valid_client_id("9f86d081-884c-4d63-a1b2-0123456789ag"));
        assert!(!valid_client_id("9f86d081-884c-4d63-a1b2-0123456789a"));
        assert!(!valid_client_id(""));
    }

    #[test]
    fn verification_uri_check() {
        for ok in [
            "https://www.microsoft.com/link",
            "https://microsoft.com/devicelogin",
            "https://login.live.com/oauth20_remoteconnect.srf",
            "https://login.microsoftonline.com/common/oauth2/deviceauth",
            "https://WWW.Microsoft.com:443/link",
        ] {
            assert!(checked_verification_uri(ok).is_some(), "{ok}");
        }
        assert_eq!(checked_verification_uri("https://www.microsoft.com/link").unwrap(), "https://www.microsoft.com/link");
        for bad in [
            "http://www.microsoft.com/link",
            "https://evil.com/link",
            "https://microsoft.com.evil.com/link",
            "https://evilmicrosoft.com/link",
            "https://notlive.com/",
            "https://user@microsoft.com/link",
            "https://microsoft.com:8443/link",
            "https://microsoft.com/link\" & calc",
            "https://microsoft.com/a b",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "https://127.0.0.1/",
            "",
        ] {
            assert!(checked_verification_uri(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn renewal_margin() {
        let now = 1_000_000;
        assert!(!needs_renewal(now + 601, now));
        assert!(needs_renewal(now + 600, now));
        assert!(needs_renewal(now - 5, now));
        assert!(!needs_renewal(now + 86_400, now));
    }

    #[test]
    fn secrets_and_sessions_never_print() {
        let s = Session {
            name: "Steve".into(),
            uuid: "0".repeat(32),
            refresh_token: Secret::new(LONG_TOKEN),
            mc_token: Secret::new("mc-secret-token"),
            mc_expires: 1,
            client_id: "c".into(),
        };
        let shown = format!("{s:?} {:?}", Secret::new("abc"));
        assert!(!shown.contains(LONG_TOKEN) && !shown.contains("mc-secret-token") && !shown.contains("abc"));
        assert!(shown.contains("<redacted>"));
    }

    #[test]
    fn cancel_stops_the_wait() {
        let cancel = AtomicBool::new(true);
        assert_eq!(sleep_or_cancel(Duration::from_secs(30), &cancel), Err(AuthError::Cancelled));
        assert_eq!(sleep_or_cancel(Duration::from_millis(1), &AtomicBool::new(false)), Ok(()));
    }
}
