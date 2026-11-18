# NEXUS Workbench · ai-m-sync-tool

> A unified desktop workbench for **SSH management**, **AI coding-session sync**, and **note sync** — all backed by a single private Git repository.

[中文文档](README.zh-CN.md) · [Screenshots](#screenshots) · [User Manual](#user-manual)

![Dashboard](docs/images/01-dashboard.png)

---

## Why

Developers juggle too many scattered tools: SSH configs, AI IDE chat histories, notes, and cloud backups. NEXUS Workbench brings them into one desktop app with a single sync backbone:

- One private Git repo stores **SSH config + notes + AI session data**
- One-click push/pull, plus scheduled background sync
- Works offline-first; Git is the only dependency for sync

## Features

| Module | What it does |
|---|---|
| **SSH Management** | Manage hosts in `assh.yml` (aliases, jump hosts, templates, wildcards); auto-generates standard `~/.ssh/config`; test connectivity; edit host info; per-host password vault — synced (encrypted) when a master password is set |
| **SSH-only push / restore** | `⇡ Push to repo` encrypts & pushes only `assh.yml` + password vault; `⇣ Restore from cloud` fetches the ciphertext, decrypts and rebuilds `~/.ssh/config` — notes/AI data untouched |
| **Built-in Terminal** | Multi-tab xterm.js terminal backed by a real PTY; auto-login with saved passwords; sessions survive page switching |
| **Electerm Integration** | Open any host in Electerm with credentials passed via CLI; auto-detects the binary (detection never launches the app); download hints if missing |
| **AI Session Sync** | Syncs ZCode, Claude Code, Cursor, Trae, and DeepSeek Harness (dsh) session data into the unified repo |
| **Notes Sync** | Local Markdown notes with rich-text/Markdown dual-mode editor; delete-and-push |
| **Youdao Note Pull** | Pulls notes from Youdao Note (Cookie-based, pull-only) and converts them to Markdown |
| **Unified Sync** | One-click push/pull of everything; global force-restore; scheduled sync |
| **First-run Onboarding** | Recover / Fresh modes with a live progress bar and a 5-second countdown into the main UI; strict Git URL validation; mandatory connectivity check; automatic network retries |
| **Scheduled Sync** | Built-in cron installer (every N minutes) + in-app timer fallback |
| **Theming** | 5 built-in themes: Neon, Forest, Ocean, Mono, Light-Green; custom app name |

## Screenshots

### SSH Management
Host list with jump-host markers, regex hosts, and per-host actions.

![SSH](docs/images/02-ssh.png)

### AI Session Sync
Auto-detects installed AI tools and syncs their session data.

![AI sync](docs/images/03-ai-sync.png)

### Notes
Local Markdown notes with a Markdown / rich-text dual editor.

![Notes](docs/images/04-notes.png)

### Settings & Themes
5 color themes, custom workbench name, Git repo config, cron settings.

![Settings](docs/images/05-settings.png)

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Tauri desktop app (Rust + React + TypeScript)          │
│                                                         │
│  Frontend (React 19 + Tailwind v4 + xterm.js)           │
│    Dashboard · SSH · AI Sync · Notes · Settings         │
│                                                         │
│  Rust backend (50+ commands)                            │
│    │                                                    │
│    ├── assh  ────►  ~/.ssh/assh.yml  ──►  ~/.ssh/config │
│    ├── PTY (portable-pty) ──► system ssh ──► terminal   │
│    ├── ai-session-sync.sh ──► ai-sessions/              │
│    ├── notes ──► notes/*.md                             │
│    ├── secrets ──► secrets.json (host passwords, 600)   │
│    ├── crypto ──► per-file AES-256-GCM                  │
│    └── git ──► unified repo (push / pull / restore /    │
│                 onboarding)                             │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
         Private Git repo (e.g. GitHub `m-data-sync`)
         ├── assh.yml            SSH config (plain mode)
         ├── encrypted/          all ciphertext (encrypted mode)
         │   ├── assh.yml.enc
         │   ├── secrets.json.enc   host password vault
         │   ├── notes/*.enc
         │   └── ai-sessions/**.enc
         └── .enc_salt           KDF salt
```

**Sync strategy**

| Action | Semantics |
|---|---|
| Push (Settings) | Encrypt everything → commit → pull(rebase, ours) → push |
| **SSH-only push** | Encrypt & push only `assh.yml` + password vault; notes/AI untouched |
| Pull (Settings) | Merge from remote, local uncommitted changes win |
| Force restore (Settings) | `reset --hard origin/main` — remote overwrites everything |
| **SSH-only restore** | Fetch `encrypted/assh.yml.enc` → decrypt with master password → rebuild `~/.ssh/config` |
| Onboarding · Recover | Connect repo → pull everything (progress) → decrypt → rebuild SSH config |
| Onboarding · Fresh | Connect an **empty** repo → encrypt local data → initial push (force, cloud verified empty) |

> **Full-content encryption is opt-in.** Set a master password in Settings and every synced file (`assh.yml`, `notes/`, `ai-sessions/`, plus the host-password vault `secrets.json`) is encrypted per-file (AES-256-GCM, 100k-round SHA-256 KDF) into `encrypted/**/*.enc` before pushing; pulling decrypts them back in place (the password vault is written to the local app data dir, mode 600). SSH **private keys** are never synced.

---

## Getting Started

### Prerequisites

- Node.js ≥ 20 + pnpm
- Rust toolchain (stable)
- Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf` (Ubuntu 24.04 also needs `libayatana-appindicator3-dev`)
- Optional: `assh` (SSH config compiler), `sshpass` (password auto-login), `xdotool` + `x11-utils` (bring Electerm to front on Linux)

### Install & run

```bash
git clone git@github.com:kistorm/ai-m-sync-tool.git
cd ai-m-sync-tool

pnpm install
pnpm tauri dev          # development
pnpm tauri build        # production bundle (.deb / AppImage / dmg / exe)
```

### First-run setup (3-minute guide, just follow the steps)

Launching the app for the first time opens the onboarding screen:

![Onboarding - choose mode](docs/images/06-onboarding.png)

#### Scenario A: first time (Fresh mode) — encrypt & upload this machine's data

> Use when: you already have SSH configs / notes / AI sessions on this machine and want them backed up to the cloud.

| Step | Action | Notes |
|---|---|---|
| 1 | Click **“⇡ Fresh”** | Opens the form |
| 2 | Fill in the **Git repo URL** | First create a **private empty repo** on GitHub, then enter `git@github.com:you/your-repo.git` (this exact format — anything else is rejected) |
| 3 | Click **“Check connectivity”** | Wait for the green ✓ (retries automatically, network glitches handled) |
| 4 | Set the **master password** (twice) | At least 6 chars, **remember it** — it encrypts everything in the cloud |
| 5 | Click **“⇡ Encrypt & push”** | Watch the progress bar (encrypt → commit → upload %) |
| 6 | Wait for the **5-second countdown** | Auto-enters the main UI — done ✅ |

![Onboarding - Fresh form](docs/images/07-onboarding-fresh.png)

#### Scenario B: new machine / reinstall (Recover mode) — pull everything back from the cloud

> Use when: the tool is installed on a new machine and you want SSH hosts (incl. passwords), notes and AI sessions back.

| Step | Action | Notes |
|---|---|---|
| 1 | Click **“⇣ Recover”** | Opens the form |
| 2 | Enter the **same repo URL** | Exactly the one used when uploading |
| 3 | Click **“Check connectivity”** | Green ✓ |
| 4 | Enter the **original master password** → click **“⇣ Start recovery”** | Progress: pull → decrypt → generate SSH config |
| 5 | Countdown ends, main UI opens | SSH hosts/passwords, notes and AI sessions are all back ✅ |

> 💡 Wrong password? You'll see “incorrect master password” and the password field gets focused — the content was already pulled on the first click, so fixing the password and retrying takes **seconds, no re-download**.
>
> 💡 Forgot the master password? Set a new one in Settings → click “⇡ Full push” to re-encrypt the cloud copy with it.

#### Safeguards shared by both modes

- **Strict Git URL validation**: only `git@host:owner/repo.git` is accepted — `https://`, `ssh://` and local paths are rejected in real time as you type (saving the config enforces the same rule)
- **Mandatory connectivity check**: after entering the URL you must click “Check connectivity” (`git ls-remote`) before continuing; editing the URL clears the previous check result
- **Live progress panel**: stage description + percentage + progress bar + elapsed time; git's object-transfer progress is parsed for push/pull
- **Automatic network retries**: connectivity check, pull and push all retry 3 times (3 s apart); the progress panel shows “retry N”
- **5-second countdown** into the main UI on success (with a “Enter now” button)
- **assh bootstrap config**: when `~/.ssh/assh.yml` is missing, a minimal transport-only config is generated automatically (just the git-sync channel, no user hosts) so the recovery flow itself can reach the network

> Changing the master password later: edit it in Settings, then immediately run a **full push** so the cloud copy is re-encrypted with the new password. Otherwise pulls will fail to decrypt.

> Changing the master password later: edit it in Settings, then immediately run a **full push** so the cloud copy is re-encrypted with the new password. Otherwise pulls will fail to decrypt.

---

## User Manual

### SSH Management

- **Add host**: `+ Add host` → alias / address / user / port / auth (key or password) / trust host
- **Edit**: select a host → `Edit connection` — alias, address, user, port, trust; unlisted fields (`inherit`, `identityfile`) are preserved
- **Password**: `View / change password`; when a master password is set the vault syncs (encrypted) and passwords are viewable after recovery
- **Test**: `Test connectivity` — uses `sshpass` when a password exists
- **⇡ Push to repo**: encrypts & pushes only `assh.yml` + the password vault (notes/AI untouched); merges remote first (local-preferred) and retries 3 times on failure
- **⇣ Restore from cloud**: fetches `encrypted/assh.yml.enc`, decrypts it with the master password (plain repos supported too), then rebuilds `~/.ssh/config` and refreshes the host list; notes/AI data untouched
- **Regenerate config**: writes standard `~/.ssh/config` from `assh.yml` (also runs automatically after add/edit/delete)
- **Templates & wildcards**: edit `assh.yml` directly for `inherit`, gateways, and regex hosts
- **Password safety**: the vault `secrets.json` is mode 600 on disk; with encryption enabled it syncs as a single `encrypted/secrets.json.enc` blob — plaintext never enters the repo; SSH **private keys** are never synced

### Built-in Terminal

- Select a host → `New terminal`; click `+ New` in the tab bar to open more (same host can open multiple tabs)
- Tabs keep their sessions alive when you switch pages; use `Minimize` to hide the workspace — the status bar shows `▸ Terminal N` to restore
- Password hosts auto-login; otherwise ssh prompts normally inside the terminal
- Full-screen apps (vim, htop) work — the PTY is resized with the panel

### AI Session Sync

- Auto-detects installed tools: **ZCode**, **Claude Code**, **Cursor**, **Trae**, **DeepSeek Harness (dsh)**
- `Push` collects sessions into `ai-sessions/` inside the unified repo and commits/pushes
- `Pull` fetches the repo, then restores data into each tool's directory
- SQLite databases are snapshotted via the online backup API (safe while apps run); SQLite restore is skipped when the app is running (use `--force` in the script to override)
- Cross-platform paths: Linux uses `~/.config`, macOS `~/Library/Application Support`, Windows `%APPDATA%`

### Notes

- Create/edit/delete Markdown notes; **Markdown** and **rich-text** modes (auto-converted back to Markdown on save)
- Delete triggers an automatic push so the deletion propagates to other machines
- **Pull from Youdao Note**: paste browser cookies, pull-only; same-name notes are overwritten by newer cloud versions
- **Clear all notes**: permanent local deletion (`.git` preserved)

### Sync & Scheduling

- **Settings → Sync repo**: push / pull / force restore / init repo
- **Scheduled sync**: set interval (minutes) and click `Install/Update` — writes a crontab entry running `auto-sync.sh` (SSH config + notes + AI sessions)
- **In-app timer** runs when crontab is not installed
- **Sync log**: view the tail of the background sync log

### System & Update (Settings page)

- **System info**: the Settings page gains a "System & Update" block showing the OS type, OS version, current login account, and app version
- **Check update**: click `Check update` to compare the version tags in the repo (e.g. `v0.2.0`) against the current version via `git ls-remote --tags`; when a newer version exists, a `⇣ Download vX` button appears and opens the GitHub Releases page to download the installer for your platform. No token required (private repos use the local SSH key)
- **Network accelerator recommendation**: a new "Network accelerator" block recommends the xx-yun (xx-yun.com) proxy/acceleration service — click it to open the promo link; GitHub access is more stable from mainland-China networks

### Packages & Release

- Pushing a `v*` tag (e.g. `v0.2.1`) makes GitHub Actions build and publish a Release on Windows / macOS (both architectures) / Ubuntu automatically
- Artifacts: Windows `.msi` / `.exe`, macOS `.dmg`, Ubuntu `.deb` / `.AppImage`, plus Source code (zip / tar.gz) archives
- Upgrade path: Settings → `Check update` → `⇣ Download vX` → install over the existing app

**Which macOS build do I need?**

| Your Mac | Download |
|---|---|
| Intel (pre-2020 models, "About This Mac" shows an Intel processor) | `nexus-workbench_x.y.z_x64.dmg` |
| Apple Silicon (M1/M2/M3/M4, "About This Mac" shows an Apple M-series chip) | `nexus-workbench_x.y.z_aarch64.dmg` |

Requires macOS 12 Monterey or later. **If the first launch says “the app is damaged” or “cannot verify the developer”**, that is the expected Gatekeeper prompt for a non-notarized app (not real corruption). Use either fix:

```bash
# Option 1: clear the quarantine flag, then open normally
xattr -cr "/Applications/nexus-workbench.app"

# Option 2: right-click the app in Finder → Open → Open again
```

### Themes & Name

- 5 themes: Neon, Forest, Ocean, Mono, Light-Green
- Custom workbench name (shown top-left and in the window title)
- Preferences persist in `localStorage`

---

## Configuration Reference

`~/.local/share/<app-id>/config.json` (Linux; platform path varies):

| Key | Description |
|---|---|
| `git_repo` | Private Git repo URL (strict `git@host:owner/repo.git` format) |
| `ssh_key` | SSH private key path for Git |
| `encrypt_password` | Master password (when set, everything incl. the password vault syncs encrypted) |
| `onboarded` | Whether first-run onboarding is done (false shows the onboarding screen) |
| `sync_interval_min` | Scheduled sync interval (minutes) |
| `conflict_strategy` | `newest_wins` |
| `notes_dir` | Notes directory (defaults inside the sync repo) |
| `ai_sync_dir` | Directory containing `ai-session-sync/sync.sh` |
| `ssh_dir` | SSH config directory (`~/.ssh`) |
| `electerm_cmd` | Electerm binary path |
| `auto_sync_enabled` | In-app timer switch |

## Development

```bash
# Backend tests (42 tests: unit + integration)
cd src-tauri && cargo test

# Frontend type check / build
pnpm exec tsc --noEmit
pnpm exec vite build
```

Project layout:

```
src/                 React frontend
  views/             Dashboard, SSH, AI, Notes, Settings, Onboarding
  components/        Terminal, UI kit
  store.tsx          App-level state (terminals, long tasks)
  api.ts             Tauri command bindings
src-tauri/src/       Rust backend
  core.rs            Config, activity log, password vault (secrets.json), command helpers
  ssh.rs             assh.yml CRUD, connectivity, Electerm, SSH-only push/restore
  terminal.rs        PTY-based terminal sessions
  ai.rs              AI tool detection + session sync
  notes.rs           Notes CRUD + git sync
  sync.rs            Unified repo push/pull/restore + full-content encryption + strict URL validation
  onboard.rs         First-run onboarding (recover/fresh, progress, retries)
  crypto.rs          AES-256-GCM encryption (100k-round SHA-256 KDF)
  cron.rs            Cron installer + auto-sync script
  youdao.rs          Youdao Note pull
  tests.rs           42 unit/integration tests
docs/images/         Screenshots
```

## FAQ

**Q: Recovery says “incorrect master password”?**
0 successfully-decrypted files is treated as a wrong password. The cloud content was already pulled on the first click — fix the password and click “Start recovery” again; it finishes in seconds without re-downloading.

**Q: Fresh-mode push fails with `! [rejected] main -> main (fetch first)`?**
GitHub repos can never be truly ref-less (clearing one leaves an empty-tree commit). Fresh mode verifies the remote tree has no files and then pushes with `--force`. If it is still rejected, new content appeared on the remote — make sure the cloud copy is really disposable before overwriting.

**Q: Can recovery still reach the network without `assh.yml`?**
Yes. When `~/.ssh/assh.yml` is missing, the app generates a minimal bootstrap config (just the git-sync transport, e.g. github.com → ssh.github.com:443) so the recovery flow itself can connect; it is overwritten by the full config decrypted from the cloud afterwards.

**Q: Why doesn't my GitHub host open a terminal?**
GitHub's SSH endpoint (`git@ssh.github.com`) is a Git service — it does not allocate a PTY. The app detects Git-service hosts (GitHub/GitLab/Bitbucket/Gitee/Codeberg/sr.ht) and disables terminal entry points.

**Q: Do SSH passwords sync across machines?**
Yes — encrypted. When a master password is set, the host-password vault `secrets.json` syncs along with everything else as `encrypted/secrets.json.enc`; after recovery on a new machine you can view passwords again (written to the local app data dir, mode 600). SSH **private keys** are never synced; without a master password the vault is not synced either.

**Q: "Pull" didn't restore a file I deleted locally.**
Regular pull is local-preferred. Use **Settings → Force restore** (`reset --hard`) or **SSH → Restore from cloud** for a remote-overwrites-local restore.

**Q: Electerm opened but the window is behind other windows.**
Electerm is single-instance; the app activates its window automatically (Linux requires `xdotool` + `xwininfo`).

**Q: Youdao Note integration?**
Official OpenAPI is closed to new applicants. This app uses Cookie-based pull (pull-only, no writes back).

**Q: Does “Check update” work with a private repo?**
Yes. The check reads the repo's tags with your local SSH key (`git ls-remote --tags`) — there is no need to make the repo public; when downloading installers, just make sure the browser is logged in to GitHub so the private Release is accessible.

## License

MIT
