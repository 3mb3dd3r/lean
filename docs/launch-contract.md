# Launch contract

The launcher (`launcher/`, Rust) and the in-game client (`client/`, Java) both depend on this file. Change both sides together.

Target: Minecraft **1.7.10**, Java **8**, optional OptiFine **1.7.10 HD U E7**.

## Data folder

Everything lives in `%APPDATA%\.lean\`. The launcher creates it. Before the project was called Lean the folder was `.medal`, with `game\medal` for the client's files; the launcher moves an old folder to the new names once, on its first start.

| Path | Written by | Contents |
|---|---|---|
| `launcher.json` | launcher | Launcher settings: selected profile id, memory in MB, OptiFine on/off, `msClientId` (Microsoft app ID) |
| `accounts.json` | launcher | The signed-in Microsoft account: name, UUID and tokens. Tokens are encrypted for the current Windows user (DPAPI) |
| `log_configs\log4j2-lean.xml` | launcher | The game's logging config, rewritten before each launch (see below) |
| `profiles\<id>.json` | launcher | One file per profile, in the profile format below |
| `runtime\java8\` | launcher | Java 8 runtime, laid out so `runtime\java8\bin\javaw.exe` exists |
| `versions\1.7.10\1.7.10.json` and `1.7.10.jar` | launcher | Mojang's version JSON and the vanilla client jar |
| `libraries\` | launcher | Libraries, Maven layout (`group\path\artifact\version\artifact-version.jar`) |
| `natives\1.7.10\` | launcher | Native DLLs extracted from the natives jars |
| `assets\indexes\1.7.10.json`, `assets\objects\` | launcher | Asset index and objects (`objects\<first 2 hash chars>\<hash>`) |
| `client\lean-client.jar` | launcher | The in-game client jar, copied in before each launch |
| `optifine\<file>.jar` | launcher | OptiFine jar the player imported (optional) |
| `game\` | game | Game directory (`--gameDir`): options.txt, saves, resourcepacks, screenshots, logs |
| `game\lean\profile.json` | both | Active profile. The launcher writes it right before launch; the client may write it back |
| `game\lean\waypoints.json` | client | Waypoints, grouped per server or singleplayer world |
| `logs\launcher.log`, `logs\game-output.log` | launcher | Launcher log, and the game's stdout/stderr |

**Profile round trip:** when the launcher starts, if `game\lean\profile.json` exists and its `id` matches a stored profile, the launcher replaces its stored copy with that file. The client saves HUD positions and the ToggleSprint state there.

## Profile format

```json
{
  "schema": 1,
  "id": "default",
  "name": "Default",
  "hud": { "color": "FFFFFF", "shadow": true },
  "toggleSprint": { "enabled": true, "showStatus": true, "toggled": false, "x": 0.5, "y": 1.0 },
  "cps":          { "enabled": true, "showRight": true, "x": 0.5, "y": 5.0 },
  "coords":       { "enabled": true, "showDirection": true, "x": 0.5, "y": 9.5 },
  "potCounter":   { "enabled": true, "x": 0.5, "y": 18.0 },
  "waypoints":    { "enabled": true, "showDistance": true },
  "keys": { "addWaypoint": "B", "waypointMenu": "M", "hudEditor": "RSHIFT" }
}
```

- `id`: lowercase letters, digits and `-`; it is also the file name in `profiles\`.
- `x`, `y`: where the element's top-left corner sits, as a percentage (0 to 100) of the scaled screen width and height. The client keeps elements fully on screen.
- `hud.color`: RGB hex without `#`.
- Keys use LWJGL 2 `Keyboard` names: `A`-`Z`, `0`-`9`, `F1`-`F12`, `LSHIFT`, `RSHIFT`, `LCONTROL`, `RCONTROL`, `LMENU`, `RMENU`, `TAB`, `GRAVE`, and so on. `NONE` means unbound. ToggleSprint uses the game's own sprint key.
- A missing field means its default (the values above). Both sides must keep fields they don't know about when they rewrite a profile.

## Launch command

```
<data>\runtime\java8\bin\javaw.exe
  -Xms512M -Xmx<memory>M
  -XX:+UnlockExperimentalVMOptions -XX:+UseG1GC -XX:G1NewSizePercent=20
  -XX:G1ReservePercent=20 -XX:MaxGCPauseMillis=50 -XX:G1HeapRegionSize=32M
  -Djava.library.path=<data>\natives\1.7.10
  -Dlog4j.configurationFile=<data>\log_configs\log4j2-lean.xml
  -cp <classpath>
  net.minecraft.launchwrapper.Launch
  --username <name> --version 1.7.10 --gameDir <data>\game
  --assetsDir <data>\assets --assetIndex 1.7.10
  --uuid <uuid, no dashes> --accessToken <Minecraft access token>
  --userProperties {} --userType msa
  --tweakClass lean.client.launch.LeanTweaker
```

Working directory: `<data>\game`.

The launcher only starts the game for a signed-in Microsoft account that owns Minecraft, and fills in its real name, UUID and Minecraft access token. There is no offline account, because Mojang only approves launcher app IDs that check sign-in and ownership before any play. The access token must never be written to a log: the launcher's own log shows `<redacted>`, and the logging config does two jobs. It keeps Mojang's Log4Shell fix for 1.7, dropping any message that contains `${...}`, because 1.7.10 ships log4j 2.0-beta9. It also drops the `(Session ID is token:<token>:<uuid>)` line that 1.7.10 prints at startup. The config's source is `launcher/assets/log4j2-lean.xml`.

Classpath, in this order, joined with `;`:

1. Every library in `1.7.10.json` that applies to Windows and has a normal artifact, except `com.google.guava:guava:15.0`, which is replaced by `com.google.guava:guava:17.0` (Mixin needs it). Natives-only entries are extracted, not put on the classpath.
2. `net.minecraft:launchwrapper:1.12` from `https://libraries.minecraft.net/`.
3. `org.ow2.asm:asm-all:5.0.3` from `https://repo1.maven.org/maven2/`.
4. `<data>\client\lean-client.jar`.
5. The OptiFine jar, only when OptiFine is turned on.
6. `<data>\versions\1.7.10\1.7.10.jar`, last.

`LeanTweaker` returns the game arguments, registers OptiFine's class transformer when OptiFine is on the classpath, then starts Mixin. Do not also pass `--tweakClass optifine.OptiFineTweaker`: its arguments would be added twice.
