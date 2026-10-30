# RL Tracker Suite

<p align="center">
  <img src="ui/icon.png" width="96" height="96" alt="RL Tracker Suite Logo">
</p>

<p align="center">
  <strong>Session stats and pre-match lobby radar for Rocket League, in one app. Passive, customizable, built for the Easy Anti-Cheat era.</strong>
</p>

<p align="center">
  <a href="https://github.com/NeyorDEV/rl-tracker-suite/releases/latest">
    <img src="https://img.shields.io/github/v/release/NeyorDEV/rl-tracker-suite?logo=github&color=blue" alt="Latest Release">
  </a>
  <a href="https://github.com/NeyorDEV/rl-tracker-suite/actions/workflows/build.yml">
    <img src="https://github.com/NeyorDEV/rl-tracker-suite/actions/workflows/build.yml/badge.svg" alt="Build Status">
  </a>
  <a href="LICENSE">
    <img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT">
  </a>
  <a href="https://tauri.app/">
    <img src="https://img.shields.io/badge/Tauri-v2-24C8D8.svg?logo=tauri&logoColor=white" alt="Tauri 2">
  </a>
  <img src="https://img.shields.io/badge/Easy_Anti--Cheat-100%25_Safe_(No_Injection)-brightgreen.svg" alt="EAC Safe">
  <img src="https://img.shields.io/badge/Platform-Windows-0078D6.svg?logo=windows&logoColor=white" alt="Platform: Windows">
</p>

---

## Two projects, one app

RL Tracker Suite is the merge of two projects that both read Rocket League's official Stats API:

| Project | What it does | Repository |
|---|---|---|
| **RL Session Stats** | Session tracking: MMR, wins / losses / streak, history, goals, in-game HUD, OBS | [NeyorDEV/rl-session-stats](https://github.com/NeyorDEV/rl-session-stats) |
| **LobbyTracker** | Pre-match radar: real ranks of the lobby, smurf detection, badges, lobby overlay | [NeyorDEV/lobbytracker](https://github.com/NeyorDEV/lobbytracker) |

They shared most of their plumbing (Stats API connection, game and account detection, Tracker Network access, overlay windows, OBS server, settings), so the merge keeps **one** implementation of each instead of two. The app has two tabs, **Session Stats** and **LobbyTracker**; see [Project layout](#project-layout) for what is shared.

The two original repositories remain available as the history of each project. New development happens here.

---

## 100% Safe with Easy Anti-Cheat (No Ban Risk)

Ever since **Easy Anti-Cheat (EAC)** was introduced in Rocket League, traditional BakkesMod plugins and memory injectors are blocked or risk permanent account bans.

**RL Tracker Suite uses ZERO injection, ZERO DLL hooking, and ZERO memory reading.**
It only communicates through Rocket League's **official native local telemetry Stats API** (`TAGame.MatchStatsExporter_TA`) via a local TCP socket on `127.0.0.1:49123`, and reads the game's own `Launch.log` in read-only mode. The overlays are separate transparent windows: nothing is drawn inside the game. You can play online competitive matches normally with EAC fully enabled.

> No tool can promise zero risk, and there is no guarantee from Epic. The design simply avoids every technique anti-cheats target.

---

## Features

### Tab 1 - Session Stats

* **Multiple HUD styles & layouts**: horizontal card, column, vertical card and plain-line layouts with high-resolution rank badges, MMR readout, match delta and win / loss / streak counters.
* **MMR & real-time sync**:
  * Automatic live MMR sync through Rocket League Tracker Network (invisible background window, no manual entry).
  * Accurate official brackets for 1v1, 2v2 and 3v3, with a division progress bar and the exact MMR needed to rank up.
  * Floating MMR delta badges (`+11.2` / `-8.9`) and banners on rank up and division up.
  * Focused playlist selector (1v1, 2v2, 3v3) with automatic playlist switching.
  * Optional automatic sync after each finished match.
* **Multi-session history**: day-by-day browser, interactive MMR curve, match-by-match breakdown, per-playlist statistics and summary cards (record, net MMR, winrate, streak).
* **Session goal tracking** with a progress bar.
* **Deep customization**: every overlay color, opacity (20-100 %), scale (60-200 %), a 6-position screen grid with pixel margins or free drag-and-drop, and per-category stat selection.
* **Native process watcher & autostart**: the overlay sleeps when Rocket League is closed or in the background, silent tray autostart, optional auto-reset on exit.
* **Multi-account**: the account is detected at each game launch (Epic or Steam) and each one keeps its own session.

### Tab 2 - LobbyTracker

* **Live lobby analysis**: every player of the match is looked up on Tracker Network (opponents first) and shown with their real ranks. Profiles are cached in SQLite (24 h by default) so known players appear instantly.
* **Badges, each of them can be turned off** (a disabled badge is neither computed nor sent to the overlay):

  | Badge | Default rule |
  |---|---|
  | **Potentiel smurf** | fewer than 250 total wins and rank at least Champion 2 (thresholds adjustable), or a young account climbing fast (at least 15 recent ranked matches, 70 % wins and +150 MMR), plus a 0-100 *Smurf Index* |
  | **Joueur 1v1** | 1v1 rank at least 2 tiers above the lobby's 2v2 / 3v3 median (10+ duel matches) |
  | **Gardien** | at least 1.6 saves per match and saves at least 2.5 times goals |
  | **Lose streak : N** | 3 or more consecutive losses, shows the count |
  | **Ancien haut rang** | peak at least Champion 2 and current rank at least 2 tiers below the peak |
  | *Opponents only* | hides the teammates |

* **Its own overlay**: four layouts (horizontal cards, column, vertical cards, row), screen position or free drag, margins, scale (50-200 %), opacity, 11 customizable colors, live preview.
* **Automatic hiding**: never, **when the 3-2-1 countdown ends** (Stats API `RoundStarted` event), or after a delay.
* **Shortcut**: <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>L</kbd> (also in the tray menu).
* **OBS**: the same overlay as a browser source, honoring the layout, colors and scale.

---

## Download & Quick Start

Pre-compiled Windows binaries are published on the **[Latest Release](https://github.com/NeyorDEV/rl-tracker-suite/releases/latest)** page:

* **RL Tracker Suite (portable).exe**: standalone executable, no installation required.
* **RL Tracker Suite Setup (.exe)**: standard installer with desktop and Start menu shortcuts.

> *Windows SmartScreen may warn about unsigned open-source binaries on first run. Click **More info** -> **Run anyway**.*

1. **Close Rocket League** before enabling the API *(the game overwrites its configuration files when it exits)*.
2. **Launch RL Tracker Suite**. In the **Session Stats** tab, **Stats API de Rocket League** section, click **Activer la Stats API**. The app finds your Epic Games or Steam installation and sets `PacketSendRate` in `DefaultStatsAPI.ini` (a `.bak` backup is kept). The Stats API is shared by both tabs.
3. **Account**: it is detected automatically when the game starts. You can also type your in-game name and pick a focused playlist in **Joueur & Compte**, then sync your MMR.
4. **In Rocket League**, set **Options -> Video -> Window Mode: Borderless** so transparent overlays can be drawn on top of the game.
5. **Play.** Session stats update after each match; the lobby overlay appears when the players of a match arrive.

> **Pro tip**: press <kbd>Win</kbd> + <kbd>R</kbd>, type `shell:startup` and paste a shortcut to the portable `.exe`. The app starts silently in the tray (~0 % CPU) and is always ready.

### Coming from the previous apps

On first launch, if the new data folder is empty, the settings, history and sessions of the former RL Session Stats app (`%APPDATA%\fr.rlsessionstats.app`) are imported automatically. The old app is left untouched. LobbyTracker had its own settings file and starts with defaults.

---

## Controls & Shortcuts

* <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>O</kbd>: show / hide the session overlay.
* <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>L</kbd>: show / hide the lobby overlay.
* **Tray icon**: open the settings, toggle either overlay, reset the session, or quit.

---

## OBS Studio / Streamlabs Setup

* **Session overlay**: Browser source, URL `http://localhost:5177/overlay`, width `750`, height `180`.
* **Lobby overlay**: Browser source, URL `http://localhost:5177/lobby` (same port).
* **Text files** (session): click **Ouvrir le dossier** in the settings to get `mmr.txt`, `mmr_change.txt`, `rank_full.txt`, `wins.txt`, `losses.txt`, `streak.txt`, `winrate.txt`, etc.

The port can be changed in the OBS section of the Session Stats tab.

---

## Project layout

```
src-tauri/            the Tauri application: Session Stats + the LobbyTracker glue (lobby.rs, trn_web.rs)
crates/
  tracker-core        shared types, LobbyTracker settings
  tracker-parser      Stats API events -> players / kickoff
  tracker-cache       SQLite cache of Tracker Network profiles
  tracker-heuristics  Smurf Index and badges
  tracker-scraper     Tracker Network mapping, priority queue, API-token fallback
  tracker-engine      lobby state, glue between the crates
ui/                   settings.html (tabs) + settings.js (Session Stats) + lt.js (LobbyTracker)
                      overlay.html (session) / lobby-overlay.html (lobby, in game and OBS)
```

What the two tabs share instead of duplicating:

| Need | Shared code |
|---|---|
| Stats API connection and JSON stream decoding | `stats_client.rs` (events are also forwarded to the lobby engine) |
| Enabling the Stats API, Epic / Steam install detection, launch detection | `rl_config.rs` and the "Stats API" section |
| Local account detection at each game launch | the game watcher in `main.rs` |
| Hidden WebView2 access to Tracker Network (Cloudflare) | `trn_web.rs`, used by the MMR sync and by the lobby lookups |
| Local OBS server | `web_server.rs` (two server-sent-event channels) |
| Settings file | `settings.rs` (LobbyTracker settings live in the `lobbytracker` section) |
| Overlay window placement and drag | `show_overlay_window`, `overlay_origin`, `drag_window_by` in `main.rs` |
| Settings page components | the same CSS classes (theme cards, screen grid, sliders, color grid) |

**Cloudflare workaround**: Tracker Network returns 403 to bare HTTP clients, so profiles are read from a hidden WebView2 window that loads the public profile page like a real browser. An official TRN API token can be set as a fallback. Use of Tracker Network data remains subject to their terms of service.

---

## Known limitations

* The lobby needs the Stats API (`PacketSendRate` above 0). Without it, the Session Stats tab has nothing to count and the LobbyTracker tab shows no players.
* Epic, Xbox and PSN accounts are looked up on Tracker Network **by name**, so a player whose name is not known yet stays in "loading". Steam accounts are looked up by ID.
* Tracker Network does not expose account age or demolition statistics, so neither is used.
* Tracker Network blocks clients that send too many requests (HTTP 429). The app only calls it when needed: once when the game starts (your MMR), once per player of a lobby (then cached for 24 h), when you click **Synchroniser**, and once after each finished match. Lookups run one at a time, anonymous names ("Player 3") are skipped, and after a 429 the automatic lookups wait a short time (30 s, then 1, 2, 4, 5 min if it keeps happening) while the Synchroniser button always retries immediately.
* The Tracker Network schema is read tolerantly but is not guaranteed to stay stable.
* The `RoundStarted` kickoff event follows the Stats API documentation; it was tested against a simulated Stats API, not yet against a full real match.

---

## Building from Source

Prerequisites: [Node.js](https://nodejs.org/) (v18+), [Rust](https://rustup.rs/) (stable), Windows C++ Build Tools.

```powershell
git clone https://github.com/NeyorDEV/rl-tracker-suite.git
cd rl-tracker-suite

npm install                  # Tauri CLI
cargo test --workspace       # application + LobbyTracker crates
npx @tauri-apps/cli dev      # development mode
npm run build                # production bundle, .exe files copied to build-output/
```

---

## Privacy Policy

RL Tracker Suite respects your privacy: zero telemetry, zero data collection, everything stays on your machine. Players of your lobby are looked up on Tracker Network from a hidden browser window. See [PRIVACY.md](PRIVACY.md).

---

## License & Credits

Released under the [MIT License](LICENSE).

Built from [RL Session Stats](https://github.com/NeyorDEV/rl-session-stats) and [LobbyTracker](https://github.com/NeyorDEV/lobbytracker) by [NeyorDEV](https://github.com/NeyorDEV).
