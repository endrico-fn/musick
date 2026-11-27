# musick

Full-Rust TUI streaming audio player for YouTube Music — keyboard-driven, cookie-free by default, and built for modern Linux terminals (Kitty primary target, with Sixel, Halfblock, and text-only fallbacks).

## Install on Linux (Zero-Compile)

### Quick Installer (Prebuilt Binary + SHA256 Verification)

```sh
curl -fsSL https://raw.githubusercontent.com/endrico-fn/musick/main/scripts/install.sh | sh
```

- Installs the default prebuilt binary (`x86_64` / `aarch64`) to `~/.local/bin/musick`.
- Pass `--fast` for the optional fast-resolver variant (`rustypipe`, GPL-3.0).
- Update any time without recompiling:

```sh
musick update           # download & verify latest release
musick update --check   # check for updates only
musick update --fast    # switch to / update fast-resolver variant
```

### Arch Linux (AUR)

A ready-to-publish [`PKGBUILD`](packaging/aur/PKGBUILD) (`musick-bin`) is provided in `packaging/aur/`.

### Cargo Binstall / From Source

```sh
cargo binstall musick-tui          # prebuilt release via GitHub Releases
# or compile from source:
cargo build --release -p musick-tui
./target/release/musick
```

## Runtime Dependencies

- **Required**: `mpv` (audio playback over IPC), `yt-dlp` (stream resolution & YouTube captions)
- **Recommended**: Kitty terminal (1:1 pixel-centered album artwork via Kitty graphics protocol; Sixel, Halfblock, and `Off` fallback supported)
- **Optional**: `cava` (real-time audio spectrum visualizer)

Verify your environment with:

```sh
musick doctor
```

## Highlights

- **Privacy-first (`no cookies` by default)**: Runs fully anonymous out of the box. Optional OAuth2 TV/Device login or Browser Auth (`SAPISIDHASH`) enables 2-way library/playlist sync and opt-in playback telemetry.
- **Full YouTube Music catalog**: Discover (`Mixes · New Releases · Featured`), fuzzy + semantic search with pagination, prefix filters (`a:`, `al:`, `pl:`, `c:`, `v:`, `s:`), and URL paste support.
- **Rich Artist, Album & Playlist views**: Split Artist Overview with interactive Top Songs & Latest Releases / Fans Also Like panes, full discography pagination, biography viewer (`i`), shuffle (`x`), radio (`r`), inline `/` filtering, and sorting (`o`).
- **Synced Lyrics + Indonesian Translation**: LRCLIB, YouTube Music timed/plain lyrics, and YouTube captions with live offset adjustment (`,` / `.`) and `t` translation toggle.
- **Two-tier Queue & Endless Radio**: Priority `Next Up` queue, gapless transitions, next-track prefetch, undo (`u`), and opt-in session restore across restarts.
- **L1 + L2 Caching**: Bounded in-memory LRU + SQLite/disk caches with configurable TTL, cover budget pruning, and off-thread terminal graphics encoding.

## CLI Subcommands

```sh
musick                              # Launch TUI
musick --log-file /tmp/musick.log   # Launch with debug logging (or set MUSICK_LOG=debug)
musick doctor [--offline]           # Check mpv, yt-dlp, cava, terminal graphics & network
musick update [--check] [--fast]    # Self-update from GitHub Releases (SHA256 verified)
musick auth status                  # Inspect auth & telemetry state
musick auth oauth --client-id <ID> --client-secret <SECRET>
musick auth browser                 # Import DevTools headers, curl snippet, or Cookie via stdin
musick auth logout                  # Wipe credentials, purge remote cache, keep local playlists
musick cache stats                  # Show disk cache usage
musick cache prune                  # Run cache janitor now
musick cache clear                  # Clear caches & rotate visitorData (preserves musick.db)
```

## Keybindings (Vim Mode)

| Key | Action |
|---|---|
| `1`–`5` / `[` `]` | Switch tabs (`1 Player`, `2 Playlists`, `3 Library`, `4 Queue`, `5 Settings`) |
| `j` `k` / `Ctrl-d` `Ctrl-u` / `gg` `G` | Navigate lists, half-page scroll, jump top/bottom |
| `h` `l` / `Tab` `Shift-Tab` | Switch panes or sub-tabs |
| `/` / `g/` | Inline filter (or search on Player) / Jump to global search from anywhere |
| `Enter` / `p` | Open / play context — or direct play |
| `Space` / `>` `<` / `{` `}` | Play/pause, next/prev track, seek ±30s (`←`/`→` ±5s) |
| `+` `-` / `M` / `z` / `Z` | Volume, mute, shuffle, repeat mode |
| `N` / `a` / `P` / `f` / `w` `W` | Play next, append queue, add to playlist, like, save item / follow artist |
| `L` / `t` / `,` `.` / `zz` | Expand lyrics, toggle ID translation, lyrics offset ±0.25s, center lyrics |
| `ga` `gl` / `gA` `gL` | Jump to selected track's Artist/Album — or currently playing Artist/Album |
| `r` / `x` / `i` / `o` | Start Radio, artist shuffle, artist bio modal, cycle sort |
| `c` `R` `dd` `J` `K` `I` `E` `U` | Playlist create, rename, delete/remove, reorder, import, export, upload to YTM |
| `yy` / `Y` / `S` / `gr` / `?` / `q` | Copy item URL, copy collection URL, sync now, refresh Discover, help, quit |
