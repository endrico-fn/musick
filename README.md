# musick

Modern full-Rust TUI streaming audio player for YouTube Music.

## Highlights

- **Privacy-first (`no cookies` by default):** Launches strictly in `Anonymous` mode with zero cookies, zero credentials, and playback history/telemetry disabled.
- **Opt-in 2-Way Sync:** Sign in optionally via **OAuth2 TV/Device Flow** (`musick auth oauth`) or **Browser Auth (`SAPISIDHASH`)** (`musick auth browser`) stored in the OS Keyring (`0o600` file fallback) with Brand Account support. Syncs Playlists, Liked Songs, Saved Albums, Saved Artists, and Saved Community Playlists between TUI and browser.
- **Professional Multi-Tier Cache:** L1 bounded LRU + L2 SQLite WAL `cache.db` + `images/` blob cache with automatic byte/file/TTL budgets and background janitor, strictly separated from user data (`musick.db`). Clearing cache never deletes playlists or library items.
- **YouTube Music Algorithm Parity:** Coherent `visitorData`, Home **`Mixes` | `New Releases`** feeds, infinite radio `continuation`, `rapidfuzz` semantic/lyric search ranking, and opt-in playback telemetry.
- **Audio & Visuals:** Persistent `mpv` JSON IPC (gapless, `dynaudnorm`, cubic lookahead `alimiter` for 101–130% overdrive), hybrid `RustyPipe` fast-path + `yt-dlp --no-cookies` circuit-breaker fallback, Linux MPRIS D-Bus controls, 1:1 terminal cover art (Kitty / Sixel / Halfblock), `cava` spectrum visualizer, synced/plain lyrics + Indonesian translation, and Braille dot loading indicators (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`).

## Workspace Structure

- `crates/musick-core` — Domain models, XDG paths & config, privacy-first auth, SQLite WAL `UserStore`, multi-tier `CacheManager`, native InnerTube client, search ranker, lyrics & translation, image service, and offline-first 2-way `SyncEngine`.
- `crates/musick-audio` — Two-tier `PlayQueue`, `MpvIpcBackend`, hybrid `StreamResolver` (`fast-resolver` feature for RustyPipe), `PlayerEngine`, `CavaRunner`, and Linux `MprisService`.
- `crates/musick-tui` — Ratatui TUI (`musick` binary), collision-free Vim keymap, widgets, 5 root views (`Home`, `Playlists`, `Library`, `Queue`, `Settings`), detail views (`Artist`, `Album`, `PlaylistDetail`), and CLI subcommands.

## Runtime Dependencies

- `mpv` (required for audio playback)
- `yt-dlp` (required for fallback stream resolution and YouTube caption lyrics)
- `cava` (optional, for live audio spectrum visualization)

## Build & Run

```bash
cargo build --release
./target/release/musick
```

To build without GPL-3.0 `rustypipe` (pure MIT/Apache/MPL dependencies using `yt-dlp` only):

```bash
cargo build --release --no-default-features --package musick-audio
```

## CLI Subcommands

```bash
musick                          # Launch TUI
musick auth status              # Inspect auth & telemetry state
musick auth oauth --client-id <ID> --client-secret <SECRET>
musick auth browser             # Import DevTools headers or Cookie string via stdin
musick auth logout              # Wipe credentials, purge remote cache, keep local playlists
musick cache stats              # Show disk cache usage
musick cache prune              # Run cache janitor now
musick cache clear              # Clear caches & rotate visitorData (preserves musick.db)
```

## Collision-Free Vim Keymap

| Scope | Key(s) | Action |
| :--- | :--- | :--- |
| **Tabs** | `1`–`5`, `gt` / `gT` | Switch root tabs (`Home`, `Playlists`, `Library`, `Queue`, `Settings`) |
| **Sub-tabs / Panes** | `Tab` / `Shift+Tab`, `Ctrl+w` + `h/j/k/l` | Switch sub-tab (`Mixes` / `New Releases`, Library, Artist) / focus pane |
| **Global** | `L` (`Shift+L`) / `t` | Toggle expanded Lyrics / Toggle Indonesian translation |
| **Global** | `S` (`Shift+S`) / `/` / `?` / `q` | Sync Now / Search or Filter / Keymap Help / Clean Quit |
| **Playback** | `Space`, `n`/`b`, `>`/`<` (`]`/`[`), `+`/`-`, `s`, `m` | Play-Pause, Next/Prev, Seek ±5s, Volume ±5% (0–130%), Shuffle, Repeat |
| **Navigation** | `h/j/k/l`, `gg`/`G`, `Ctrl+d`/`Ctrl+u`, `Backspace`/`H`, `Esc` | Vim motion, top/bottom, half-page, go back, cancel |
| **Item Actions** | `Enter`/`p`, `N`/`a`, `P`/`f`/`w`, `r`/`o`/`v`/`c`, `yy` | Open-Play, Play Next/Queue, Add Playlist/Like/Save, Radio/Sort/Grid/Size, Copy URL |
| **Editing** | `dd`/`Del`, `J`/`K`, `u`/`D`/`C` | Delete/Remove, Reorder down/up, Undo/Deduplicate/Clear upcoming |
| **Playlists** | `c`, `R`, `I`, `E`, `U` | Create, Rename, Import URL/File, Export JSON+M3U8, Upload local to YTM |

## Storage Layout (XDG)

- `$XDG_CONFIG_HOME/musick/settings.toml` — User settings
- `$XDG_DATA_HOME/musick/musick.db` — User playlists, liked songs, saved collections, sync outbox
- `$XDG_CACHE_HOME/musick/{cache.db,images/,rustypipe/}` — Self-pruning caches
- `$XDG_RUNTIME_DIR/musick/mpv-<pid>.sock` — Private (`0o700`) mpv IPC socket
