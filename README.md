```
ooooo                                       
`888'                                       
 888          .ooooo.   .oooo.   ooo. .oo.  
 888         d88' `88b `P  )88b  `888P"Y88b 
 888         888ooo888  .oP"888   888   888 
 888       o 888    .o d8(  888   888   888 
o888ooooood8 `Y8bod8P' `Y888""8o o888o o888o

```

A lightweight alternative to Lunar Client for 1.7–1.8 potion PvP. You sign in with the Microsoft account that owns Minecraft, pick a settings profile in the launcher and press Play. The launcher writes the profile into the game folder, starts Minecraft 1.7.10 on Java 8 with the client (and OptiFine if you imported it), then closes, so nothing else runs while you play.

The client adds ToggleSprint, a CPS counter, coordinates, waypoints and a pot counter. It works on any normal server, because the server needs nothing installed.

| Folder | What it is |
|---|---|
| `client/` | The in-game client (Java 8). It hooks into vanilla 1.7.10 with Mixin, with no Forge. |
| `launcher/` | The launcher (Rust, egui). It handles profiles, downloads, the launch itself, and OptiFine import. |
| `docs/launch-contract.md` | The data folder layout, profile format and launch command that both sides follow. |

## In game

| Key | Does |
|---|---|
| Sprint key (Left Ctrl by default) | Turns ToggleSprint on or off |
| `B` | Adds a waypoint where you stand |
| `M` | Opens the waypoint list (add, delete) |
| `Right Shift` | Opens the HUD editor: drag elements, then Done or Esc saves them to the profile |

You can change the keys per profile in the launcher.

## Building

You need JDK 17 or newer to run Gradle (JDK 21 works) and Rust with the `stable-x86_64-pc-windows-gnu` toolchain. Gradle downloads its own Java 8 for compiling.

Client:

```powershell
cd client; .\gradlew.bat clientJar
```

This produces `client\build\libs\lean-client-0.1.0.jar`. The first build downloads vanilla 1.7.10 and the MCP 1.7.10 mappings.

Launcher:

```powershell
cd launcher; cargo build --release
```

`launcher\.cargo\config.toml` is machine-local and not committed. It puts the build output in `%LOCALAPPDATA%\lean-build\launcher-target` so several GB of Rust output stay out of OneDrive. Keep `Cargo.lock`: some crates are pinned on purpose, because newer versions need MinGW's `dlltool`, which the GNU toolchain doesn't include.

## Running

Put `lean-client.jar` next to `lean-launcher.exe`, or point `LEAN_CLIENT_JAR` at the built jar, then start the launcher. Play needs a signed-in Microsoft account that owns Minecraft (see below); there is no offline account. The first Play downloads about 260 MB into `%APPDATA%\.lean`: Java 8, Minecraft 1.7.10, libraries and assets.

Launcher command-line flags, for testing without the window: `--prepare`, `--print-command`, `--play`, `--login`, `--logout`, plus `--profile <id>` and `--optifine` / `--no-optifine`. `--prepare` works without an account. Run them from Git Bash, or with `Start-Process -Wait -NoNewWindow` in PowerShell. The exe is a windowed app, so PowerShell and cmd don't wait for it. Logs go to `%APPDATA%\.lean\logs\`.

## Microsoft login

In Settings > Account, press Sign in with Microsoft. The launcher shows a code; you enter it at microsoft.com/link, and it signs you in to Minecraft with that account. The sign-in tokens are kept in `%APPDATA%\.lean\accounts.json`, encrypted so that only your Windows user can read them, and the Minecraft token is renewed before it expires. Tokens never go into the logs. The game's logging config also drops the line where 1.7.10 prints the token, and it keeps Mojang's Log4Shell fix, because 1.7.10 uses an old log4j.

The launcher needs its own Microsoft app ID, and **Mojang has to approve that ID** before Minecraft accepts its sign-ins. Until then, sign-in stops with "isn't approved for Minecraft sign-in yet". The launcher has no offline account: Mojang's form says it won't approve an app that lets anyone play without signing in and owning the game, whatever the server's settings. To set it up:

1. Sign in at [portal.azure.com](https://portal.azure.com) with a Microsoft account. A personal account can't register apps until it has a directory. If Azure says so, sign up for a free Azure account, which creates one; it may ask for a phone number and a card to verify you. The app registration itself doesn't need a paid subscription.
2. Go to Microsoft Entra ID > App registrations > New registration. Name it, for example "Lean Launcher"; the name can't contain Mojang, Minecraft, Microsoft, Live, Xbox, Discord or Hypixel. For supported account types, choose **Accounts in any organizational directory and personal Microsoft accounts**. Leave the redirect URI empty, then press Register.
3. Under Authentication, set **Allow public client flows** to Yes (under Advanced settings, or on the Settings tab in the newer layout), then Save.
4. From the Overview page, copy the **Application (client) ID** and the **Directory (tenant) ID**.
5. Apply at [aka.ms/mce-reviewappid](https://aka.ms/mce-reviewappid) as a "New AppID for Approval". Use the email of the Microsoft account that owns the app in Azure, because Mojang checks it against Azure. Give the same app name, both IDs, a website or repository page about the launcher, and a short justification. Mojang reviews requests weekly; sending the form again doesn't speed it up.
6. Paste the client ID under Settings > Account. Alternatively, set `msClientId` in `launcher.json` or the `LEAN_MS_CLIENT_ID` environment variable.

Don't use another launcher's client ID instead; that breaks Microsoft's terms.

Until the app ID is approved, Play can't start the game. For client work, run `.\gradlew.bat runClient` in `client\` instead; it starts a local dev copy of the game, without the launcher. Add `-Poptifine=<path to OptiFine jar>` to load OptiFine.

## Dev checks

- `.\gradlew.bat runClient -Pautotest=true`, or `LEAN_DEV_AUTOTEST=1` for a game started by the launcher, opens a flat test world. It sets up something for every feature, saves screenshots to `<game folder>\screenshots\autotest-*.png`, and quits.
- `LEAN_DEV_SCREENSHOT=<file.png>` makes the launcher save its Home, Profiles and Settings pages as images (`<file>.png`, `<file>.profiles.png`, `<file>.settings.png`), then close.
- `LEAN_DEV_WINDOW_SIZE=<w>x<h>` opens the window at that size (points), e.g. to screenshot the scaled layout.

## Not done yet

- **A real Microsoft sign-in.** Login is built, but it can only be tried end to end once Mojang approves the app ID (see above).
- **OptiFine.** It can't be bundled. Each player downloads OptiFine 1.7.10 HD U E7 from optifine.net and imports it in the launcher's Settings tab.
- **1.8.9.** Not built yet.
