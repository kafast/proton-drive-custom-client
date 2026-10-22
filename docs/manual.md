# protonsync User Manual

German version: [handbuch.md](handbuch.md)

Status: phase 6 (all features, not yet tried with a real account). This manual covers installation, setup, operation and troubleshooting
on the desktop (CachyOS/Arch) and on a server (container or VM/LXC, e.g. on Proxmox).

> **Important:** protonsync is an unofficial client and is **not** developed or supported
> by Proton. It is in alpha. Only use it with data that has another backup, and test with
> a dedicated test folder first.

## Contents

1. [How protonsync works](#1-how-protonsync-works)
2. [Requirements](#2-requirements)
3. [Installation on the desktop](#3-installation-on-the-desktop)
4. [Configuration](#4-configuration)
5. [First start step by step](#5-first-start-step-by-step)
6. [Everyday use](#6-everyday-use)
7. [Running on a server](#7-running-on-a-server)
8. [Updating, logging out, uninstalling](#8-updating-logging-out-uninstalling)
9. [Troubleshooting](#9-troubleshooting)
10. [Files and paths](#10-files-and-paths)
11. [Security](#11-security)
12. [For developers: tests and live tests](#12-for-developers-tests-and-live-tests)

---

## 1. How protonsync works

protonsync consists of two programs:

| Program | Purpose |
|---|---|
| `protonsyncd` | The **daemon** (background service). It keeps the login, watches the sync folders, asks Proton for changes and synchronizes. |
| `protonsync` | The **command line** (CLI). It talks to the daemon over a local socket. Without a running daemon it can only run `config example`. |
| `protonsync-tray` | The **tray app** (desktop only, optional): an icon in the system tray with status, conflict dialog, login and notifications. It talks to the daemon over the same socket as the CLI. |

A **sync pair** connects a local folder with a folder in Proton Drive ("My files", shown as "Meine Dateien" in the German Proton interface).
You can create several pairs. Each pair has a direction:

- `two_way` (default): changes on both sides are applied.
- `download_only`: Proton is the source, the local folder is a mirror (e.g. a backup copy on the server).
- `upload_only`: the local folder is the source (e.g. a folder with backups).

When does synchronization happen?

- **Locally**: the daemon notices changes immediately (inotify) and synchronizes after a short quiet period
  (`local_debounce_seconds`, default 3 seconds), so that not every partial change runs on its own.
- **At Proton**: the daemon queries Proton's event list (`event_poll_seconds`, default 60,
  minimum 30). It only synchronizes when Proton reports changes. This follows Proton's
  requirement not to query everything constantly.
- **Safety net**: in addition, a full synchronization runs regularly (`full_sync_minutes`, default 60).

Basic rules that always apply:

- **Nothing is overwritten silently.** If a file was changed on both sides, a conflict
  is created; both versions are kept (see [6.3](#63-conflicts)).
- **Nothing is deleted immediately.** Deletions first go to a trash (local trash or the
  Proton trash) and are only removed for good after the retention period (default 30 days).
- **Dry run first.** After installation, protonsync only plans and shows what it would do.
  Only when you set `dry_run = false` (or run `protonsync sync --apply` once) does it
  actually transfer, rename or delete anything.

## 2. Requirements

**Desktop**

- Linux with systemd (tested against CachyOS/Arch), KDE or GNOME, Wayland or X11.
- A Secret Service for the login data: GNOME Keyring or KWallet (usually already active
  on KDE/GNOME).
- For building: Rust via `rustup`, plus a C compiler and `cmake` (for SQLite and the TLS library).
- For the tray app (optional): Qt 6 with QML (`qt6-base`, `qt6-declarative`, `qt6-svg`; usually
  already installed on KDE). On GNOME, tray icons only appear with the "AppIndicator and
  KStatusNotifierItem Support" extension.
- For the FUSE drive (optional): the `fuse3` package (provides `fusermount3`). Usually already
  installed on CachyOS/Arch, otherwise `sudo pacman -S fuse3`.

**Server**

- Option A: Docker or Podman (with `compose`). The image includes everything.
- Option B: a VM or an LXC container with systemd; the statically linked programs run
  without any further libraries.

**Proton account**

- Username and password, a 2FA app (TOTP) if enabled, and the second password if enabled
  ("mailbox password" in two-password mode).
- Security keys (FIDO2) as the **only** second factor are not supported; in that case also
  set up a 2FA app.

## 3. Installation on the desktop

Two ways: by hand with `cargo` (3.1 to 3.4) or as an Arch package ([3.5](#35-alternative-as-a-package-archcachyos)).
The package is more convenient, because `pacman` takes care of updating and removing.

### 3.1 Install the tools (CachyOS/Arch)

```sh
sudo pacman -S --needed base-devel cmake git rustup fuse3
sudo pacman -S --needed qt6-base qt6-declarative qt6-svg   # only for the tray app
rustup default stable
```

### 3.2 Get the source code and build

```sh
git clone https://github.com/kafast/proton-drive-custom-client.git ~/proton-drive-custom-client
cd ~/proton-drive-custom-client
cargo install --locked --path crates/daemon     # installs protonsyncd to ~/.cargo/bin
cargo install --locked --path crates/cli        # installs protonsync to ~/.cargo/bin
cargo install --locked --path crates/gui-qt     # optional: tray app protonsync-tray
```

`rust-toolchain.toml` makes `rustup` use a matching Rust version automatically.
The first build takes a few minutes. Check:

```sh
protonsyncd --version
protonsync --version
```

If `command not found` appears, `~/.cargo/bin` is missing from your `PATH`. Add the line
`export PATH="$HOME/.cargo/bin:$PATH"` to `~/.bashrc` or `~/.zshrc` and open a new terminal.

### 3.3 Set up the daemon as a user service

```sh
mkdir -p ~/.config/systemd/user
cp packaging/systemd/protonsyncd.user.service ~/.config/systemd/user/protonsyncd.service
systemctl --user daemon-reload
```

Do **not** start it yet: create the configuration first (next section).

### 3.4 Start the tray app (optional)

Once the daemon is running (see [5](#5-first-start-step-by-step)), run `protonsync-tray &` once in
a terminal. Then tick **Start at login** in the tray app's settings; from then on it starts with
every desktop session (entry in `~/.config/autostart/protonsync-tray.desktop`). Usage:
[6.6](#66-tray-app).

### 3.5 Alternative: as a package (Arch/CachyOS)

Instead of 3.1 to 3.4 you can build everything as a package and manage it with `pacman`. The
package is called `protonsync-git`, builds the current state of `main` and contains the daemon,
CLI, tray app, systemd services, the tray app's menu entry and the file manager files
([6.9](#69-file-managers-dolphin-and-nautilus)).

```sh
sudo pacman -S --needed base-devel git
git clone https://github.com/kafast/proton-drive-custom-client.git ~/proton-drive-custom-client
cd ~/proton-drive-custom-client/packaging/arch
makepkg -si          # fetches the source, builds (a few minutes), installs
sudo pacman -S --needed libnotify python-nautilus   # optional: notifications, Nautilus emblems
```

Afterwards the programs are in `/usr/bin`, the user service in
`/usr/lib/systemd/user/protonsyncd.service` (nothing to copy, `systemctl --user enable` is enough,
see [5](#5-first-start-step-by-step)) and the tray app in the application menu as "protonsync".
For running a server without a container the package also brings the system service and the
`protonsync` user ([7.2](#72-option-b-systemd-service-vmlxc-without-a-container)); the paths there
are then `/usr/bin` instead of `/usr/local/bin`.

**Without building yourself (e.g. for a test VM):** CI stores the finished package on every run.
On GitHub open "Actions" → "CI", pick the latest green run of `main` and download
`protonsync-arch-x86_64` under "Artifacts" (you need to be signed in to GitHub; the file is
deleted after 90 days). Then:

```sh
unzip protonsync-arch-x86_64.zip
sudo pacman -U protonsync-git-*.pkg.tar.zst   # also installs fuse3 and Qt 6
```

To build your own checkout instead of GitHub:
`PROTONSYNC_GIT="file://$HOME/proton-drive-custom-client" makepkg -si`. The licenses of the
bundled libraries are in `/usr/share/licenses/protonsync-git/`.

## 4. Configuration

### 4.1 Where is the file?

| Environment | Path |
|---|---|
| Desktop | `~/.config/protonsync/config.toml` (or `$XDG_CONFIG_HOME/protonsync/config.toml`) |
| Container | `/config/config.toml` |
| Server with systemd | `/etc/protonsync/config.toml` (via `--config`) |

You can change the path with the environment variable `PROTONSYNC_CONFIG` or with `protonsyncd --config PFAD`
(`PFAD` means PATH). If the file is missing, the daemon starts with default values and without sync pairs.

Create a commented template with:

```sh
mkdir -p ~/.config/protonsync
protonsync config example > ~/.config/protonsync/config.toml
```

Changes to the file only take effect after restarting the daemon
(`systemctl --user restart protonsyncd`).

### 4.2 Section `[general]`

| Key | Default | Meaning |
|---|---|---|
| `dry_run` | `true` | `true`: automatic synchronizations only plan. `false`: they execute. |
| `trash_retention_days` | `30` | Days that deleted items stay in the trash before they are removed for good. |
| `secrets` | `"keyring"` (desktop build), `"file"` (server build) | Where the login is stored, see [11](#11-security). |
| `event_poll_seconds` | `60` | Interval for querying Proton events in seconds, minimum `30`. |
| `full_sync_minutes` | `60` | Interval of the full safety synchronization in minutes, greater than `0`. |
| `local_debounce_seconds` | `3` | Quiet period after local changes before synchronizing. |
| `data_dir` | `~/.local/share/protonsync` | Data directory (state database, encrypted secrets). Also via `PROTONSYNC_DATA_DIR`. |

### 4.3 Section `[[pair]]` (any number of times)

| Key | Required | Meaning |
|---|---|---|
| `id` | yes | Short name of the pair; allowed characters are `a-z`, `0-9`, `-`, `_`. Used in the CLI. |
| `local` | yes | Local folder as an **absolute** path. Created if it is missing. |
| `remote` | yes | Folder below "My files" in Proton, e.g. `"Dokumente/Steuer"`. Empty (`""`) means "My files" itself. Missing folders are created. |
| `direction` | no | `"two_way"` (default), `"download_only"` or `"upload_only"`. |
| `protect_remote` | no | `true` turns on the **protection** (default `false`): local deletions do not reach Proton, and nothing in Proton is replaced by a local file. See 6.3. |

Rules: IDs must be unique, and local folders must not overlap (no pair may be located
inside another one).

### 4.4 Examples

Desktop with two pairs:

```toml
[general]
dry_run = true

[[pair]]
id = "dokumente"
local = "/home/krischan/Proton/Dokumente"
remote = "Dokumente"

[[pair]]
id = "musik"
local = "/home/krischan/Musik"
remote = "Musik"
direction = "download_only"
```

Server as a backup mirror of the whole of "My files":

```toml
[general]
secrets = "file"
dry_run = false

[[pair]]
id = "spiegel"
local = "/backup/proton"
remote = ""
direction = "download_only"
```

### 4.5 Section `[[job]]`: upload jobs (any number of times)

An upload job regularly copies files from a local folder to Proton, for example Home Assistant
backups. Unlike a sync pair it only works in one direction and cleans up the target folder
according to a retention rule. It works with any folder, including a mounted SMB or NFS share.

| Key | Required | Default | Meaning |
|---|---|---|---|
| `id` | yes | | Short name; allowed are `a-z`, `0-9`, `-`, `_`. |
| `source` | yes | | Source folder as an **absolute** path. Only files directly inside count, no subfolders. |
| `pattern` | no | `"*"` | File name pattern: `*` (any number of characters), `?` (one character), `[abc]`, `[a-z]`, `[!x]`. Example: `"*.tar"`. |
| `target` | yes | | Target folder below "My files", e.g. `"Backups/HomeAssistant"`. Created if missing. "My files" itself is not allowed. |
| `schedule` | yes | | Cron expression (see below) or `"on_new_file"`. |
| `keep_last` | no | everything | This many of the newest matching files stay in the target. |
| `max_age_days` | no | everything | Matching files older than this go to the trash. |
| `verify` | no | `"hash"` | Check after upload: `"hash"` (size and SHA-1 checksum) or `"size"` (size only). |
| `max_attempts` | no | `5` | Attempts on errors before the job gives up and reports the error. |
| `settle_seconds` | no | `60` | A file must be unchanged for this many seconds before it is uploaded, so a half-written backup never ends up in Proton. |

**Schedule.** A cron expression has five fields: minute, hour, day, month, weekday.
Allowed are `*`, lists (`1,15`), ranges (`1-5`), steps (`*/15`), names (`mon`, `jan`) and the
short forms `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`. If both day and weekday are
restricted, it is enough for one of them to match (as in classic cron).

| Expression | Meaning |
|---|---|
| `"0 3 * * *"` | every day at 3:00 |
| `"30 2 * * sun"` | Sundays at 2:30 |
| `"0 */6 * * *"` | every six hours on the hour |
| `"@daily"` | every day at 0:00 |

The time is the computer's **local time**, including daylight saving time. In the container you set
it with the environment variable `TZ` (e.g. `TZ=Europe/Berlin`, see [7.1](#71-option-a-container-dockerpodman));
otherwise UTC applies. If a time falls into the gap of the spring clock change, the job runs right
after it; the repeated hour in autumn triggers only one run. If a time was missed because the daemon was not
running (computer off), the job runs once right after the start; several missed times lead to one
run.

With `"on_new_file"` the job runs when the daemon starts, shortly after changes in the source folder,
and also every five minutes as a safety net, because network shares often do not report changes.
Proton is only contacted when matching files in the source folder have changed.

**What a run does.**

1. Find matching files in the source folder. Files within their settle time (`settle_seconds`) wait.
2. Skip files that are already in the target with the same size (and, with `verify = "hash"`, the
   same checksum). Changed files get a new version in Proton; the old one stays in Proton's
   version history.
3. Check every upload.
4. **Only if all uploads succeeded**, apply the retention rule: expired files go to the Proton trash
   and to protonsync's trash (restorable). Only files in the target that match the pattern are
   affected. The **newest file always stays**, even if it is older than `max_age_days`; that way at
   least one backup remains if the source delivers nothing for a while.

If a run fails, the daemon retries after 1, 2, 4, 8, ... minutes (at most one hour apart) until
`max_attempts` is reached. Then it reports the error in the log and as an event for user interfaces.

With `dry_run = true` jobs only plan as well. `protonsync jobs` shows the result.

Example for Home Assistant backups on an SMB share (mounted on the host at `/mnt/ha-backups`):

```toml
[[job]]
id = "home-assistant"
source = "/mnt/ha-backups"
pattern = "*.tar"
target = "Backups/HomeAssistant"
schedule = "0 4 * * *"       # daily at 4:00, after Home Assistant's nightly backup
keep_last = 14
max_age_days = 90
```

Tip: Do not put a job's target into a folder that a sync pair also syncs in both directions.
Otherwise the pair downloads the backups, and deletions in the sync folder would also move the
backups in Proton to the trash.

### 4.6 Section `[web]`: web interface

Off by default. Keys, password hash and HTTPS are described in [7.3](#73-web-interface-optional).

### 4.7 Sections `[[mount]]` and `[cache]`: FUSE drive (desktop only)

A **drive** shows a Proton folder as a virtual directory. All files and folders appear at once
but take no space: protonsync loads the content only when a program reads the file, and only the
parts that are read (so a video starts right away). Loaded content stays in the cache until it is
freed. Drives and sync pairs are two separate modes; you can use both at the same time, just not
for the same folder.

```toml
[[mount]]
id = "home"                                  # identifier, as for sync pairs
mountpoint = "/home/USER/ProtonDrive"        # absolute path, created if needed
remote = ""                                  # folder in "My files"; empty = everything
keep_local = false                           # default: false = on demand, true = always local

[cache]                                      # applies to all drives
evict_after_days = 30                        # free unused files after this many days
max_size_gb = 10                             # cache size limit (in GB, 1 GB = 10⁹ bytes)
```

| Key | Required | Meaning |
|---|---|---|
| `id` | yes | Identifier of lowercase letters, digits, `-` and `_`; shown in `protonsync mounts` and in the trash as `mount:ID` |
| `mountpoint` | yes | Absolute path where the drive appears (ideally an empty directory). Must not lie inside a sync folder or another drive, nor contain one. |
| `remote` | no | Folder in Proton; default is the root of "My files" |
| `keep_local` | no | `true`: everything is downloaded completely in the background and never freed automatically |
| `evict_after_days` | no | Default 30 |
| `max_size_gb` | no | Default 10. When exceeded, protonsync frees the least recently used files. |

How it behaves:

- **Mounted after login**, unmounted on logout and when the daemon stops. `protonsync mounts`
  shows the state.
- **Writing:** new and changed files are uploaded when closed. If that fails (e.g. offline), they
  stay queued and are retried regularly (every `event_poll_seconds`). If the file changed in Proton
  meanwhile, your version is uploaded next to it as a **conflict copy**; nothing is overwritten.
- **Deleting** in the drive moves the file to the Proton trash (restorable with
  `protonsync trash restore mount:ID NR`).
- **Changes in Proton** (e.g. from your phone) appear after the next event poll.
- **Freeing:** once an hour protonsync frees space according to the policy. Pinned, open or not yet
  uploaded files are never freed. In dry-run mode (`dry_run = true`) it only plans and removes
  nothing.
- The cache lives in `<data>/cache/ID/` (readable only by you, permissions 0700/0600).

The server build (container) has no FUSE: a `[[mount]]` is not mounted there (warning in the log),
but pinning and freeing still work.

### 4.8 Section `[photos]`: photos (Proton Photos)

Proton Photos is a separate area next to "My files"; the phone app puts its photos there.
protonsync can mirror it into a local **photo folder**, by default in one direction only (backup
from the phone to the computer or server):

```toml
[photos]
folder = "/home/USER/Pictures/Proton"     # absolute path, created when needed
direction = "download_only"               # default; or "two_way", "upload_only"
```

| Key | Required | Meaning |
|---|---|---|
| `folder` | yes | Local photo folder. Must neither lie inside a sync folder or drive nor contain one. |
| `direction` | no | `"download_only"` (default): download photos from Proton, upload nothing. `"two_way"`: also upload new local photos and move locally deleted ones to the Proton trash. `"upload_only"`: only upload new local photos. |
| `protect_remote` | no | `true`: photos deleted locally stay in Proton (default `false`, see 6.3). |

How it behaves:

- **Folder layout:** photos are stored as `YYYY/MM/name` by capture time (system time zone or
  `TZ`). If the name already exists in that month, the second photo gets the last 8 characters
  of its id appended, e.g. `IMG_0001 (a1b2c3d4).jpg`. The file's modification time is the
  capture time.
- **Deleted in Proton** (e.g. on the phone): the local copy moves to the local trash
  (`.protonsync-trash` in the photo folder), restorable with `protonsync trash restore photos:library NO`.
- **Nothing twice:** if a photo with the same content already exists in the folder or in Proton,
  it is only linked instead of transferred again.
- **Locally edited photos** are not uploaded (Proton Photos has no new versions) and are reported
  as "skipped". If a photo changes on both sides, both are kept.
- Only media files count (images including RAW, videos). Other files in the photo folder are
  left alone.
- **Cost:** after startup the daemon reads the library once in full, then only Proton events
  (every `event_poll_seconds`); once a day it reads everything again as a safety net.
- In dry-run mode (`dry_run = true`) the photo sync only plans as well.
- If the account has no photo area yet, there is nothing to sync. Uploading only works once an
  official Proton app has created the area.

## 5. First start step by step

Recommendation for the first attempt: create an empty folder in Proton (e.g. `protonsync-test`),
enter it as `remote` and use a new, empty local folder.

1. **Start the daemon**
   ```sh
   systemctl --user enable --now protonsyncd
   systemctl --user status protonsyncd        # should show "active (running)"
   ```
2. **Log in**
   ```sh
   protonsync login
   ```
   You are asked for username, password, the 2FA code if enabled and the mailbox password if enabled.
   Your input is not shown on screen. Afterwards only session keys are stored,
   never the password. After a restart, the daemon logs in again on its own.
3. **Check the state**
   ```sh
   protonsync status
   ```
   Shows the account, the dry-run mode and the sync pairs with their last run.
4. **Trial run**
   ```sh
   protonsync sync
   ```
   Lists what would be uploaded, downloaded, created or moved to the trash,
   without changing anything.
5. **Run it for real once**
   ```sh
   protonsync sync --apply
   ```
6. **Enable continuous operation**: set `dry_run = false` in the configuration, then
   ```sh
   systemctl --user restart protonsyncd
   ```
   From now on the daemon synchronizes on its own.

If the daemon should also run while you are not logged in (e.g. after booting without a login):
`sudo loginctl enable-linger $USER`. Note that the keyring may then be locked;
in that case `secrets = "file"` (see [11](#11-security)) is the better choice.

## 6. Everyday use

### 6.1 Commands

In the commands below, `PAAR` means the pair ID and `NR` means the entry number shown in the list.

| Command | What it does |
|---|---|
| `protonsync status` | Account, mode, sync pairs, last run |
| `protonsync sync` | **Plan** all pairs (change nothing) |
| `protonsync sync --apply` | Synchronize all pairs |
| `protonsync sync --pair dokumente --apply` | Synchronize only one pair |
| `protonsync conflicts` | List open conflicts |
| `protonsync conflicts resolve PAAR NR keep-local` | Resolve a conflict, keep the local version |
| `protonsync conflicts resolve PAAR NR keep-remote` | keep the remote version |
| `protonsync conflicts resolve PAAR NR keep-both` | keep both (the conflict copy stays as a separate file) |
| `protonsync trash` | List the trash of all pairs |
| `protonsync trash restore PAAR NR` | Restore an entry |
| `protonsync trash purge` | Show which expired entries would be deleted |
| `protonsync trash purge --apply` | Delete expired entries for good |
| `protonsync jobs` | Upload jobs with last and next run |
| `protonsync jobs run JOB` | **Plan** a job (change nothing) |
| `protonsync jobs run JOB --apply` | Run a job now |
| `protonsync jobs history JOB [--limit N]` | A job's latest runs (default 10, at most 50) |
| `protonsync mounts` | FUSE drives with state, cache and pinned paths |
| `protonsync pin PATH…` | "Always keep local" for files or folders in a drive (downloads in the background) |
| `protonsync unpin PATH…` | Back to "on demand" (freed later according to the policy) |
| `protonsync file-status PATH…` | State of files: placeholder, downloaded, in sync, conflict … ([6.9](#69-file-managers-dolphin-and-nautilus)) |
| `protonsync free [PATH]` | Show what would be freed (without a path: by the policy; with a path: everything below it) |
| `protonsync free [PATH] --apply` | Actually free the space |
| `protonsync photos` | Photo sync: folder, count, last run |
| `protonsync photos sync [--apply]` | Plan or run the photo sync |
| `protonsync photos duplicates` | Only show duplicates |
| `protonsync photos duplicates trash ID… [--apply]` | Check or trash selected duplicates |
| `protonsync login` / `protonsync logout` | Log in or log out |
| `protonsync config example` | Print an example configuration |
| `protonsync --help`, `protonsync BEFEHL --help` | Help (`BEFEHL` means COMMAND) |

General options:

- `--json`: output as JSON, e.g. for scripts: `protonsync --json status | jq .`
- `--socket PFAD`: talk to a different daemon (`PFAD` means PATH; default see [10](#10-files-and-paths)).
- `protonsync login --username NAME --password-stdin`: reads password, 2FA code and
  mailbox password line by line from standard input, e.g. for automation. Passwords
  never belong on the command line, because they would be visible in the process list there.

The language of the output follows `LC_ALL`, `LC_MESSAGES` or `LANG`
(German, otherwise English). Example: `LANG=en_US.UTF-8 protonsync status`.

### 6.2 Exit codes (for scripts)

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | General error |
| 2 | Wrong usage (unknown option or similar) |
| 3 | Daemon not reachable |
| 4 | Not logged in |
| 5 | Login rejected (wrong password or wrong code) |
| 6 | CLI and daemon versions do not match |
| 7 | Partially failed (at least one pair had errors, or a job run or photo sync failed) |

### 6.3 Conflicts

A conflict occurs when the same file was changed **on both sides** since the last synchronization,
or when it was changed on one side and deleted on the other.

- If both sides changed, the Proton version is kept under the original name, and the local version
  is placed next to it as a **conflict copy**, e.g. `Bericht (Konflikt 1760000000).pdf`. The word
  "Konflikt" is German for "conflict" and is always part of the file name, regardless of the language
  setting; the number is the time as Unix time. So nothing is lost.
- The conflict stays in the queue until you decide (`protonsync conflicts`).
  This also works on the server, where nobody can answer right away.
- When resolving with `keep-local` or `keep-remote`, the discarded version goes to the trash
  (restorable). `keep-both` only closes the conflict; both files remain.
  The next synchronization transfers the result to the other side.

**Protection (`protect_remote = true`):** If you delete something locally, it stays in Proton (it
stays deleted locally; `protonsync sync` shows it as "local change stays local"). If a local file
would replace an existing one in Proton, because you edited it or created it again under the same
name after deleting it, a conflict "changed locally, Proton version protected" occurs: the Proton
version stays under the original name, your version is placed next to it as a conflict copy, and
the conflict copy is uploaded as a new file. Identical content is no conflict. New files and
changes from Proton sync as usual. `keep-local` is refused for these conflicts because the Proton
version is protected; turn the protection off briefly for that. The same key exists in `[photos]`;
there it keeps photos deleted locally from being deleted in Proton.

### 6.4 Trash

- If a file is deleted **in Proton**, protonsync moves the local copy to the local
  trash `.protonsync-trash` in the sync folder.
- If a file is deleted **locally**, protonsync moves it to the Proton trash in Proton.
- `protonsync trash` shows both kinds; `restore` brings entries back.
- After `trash_retention_days` days, entries are deleted for good. The daemon checks this once
  a day; in dry-run mode it only shows what it would delete.

Trash entries created by an upload job's retention rule appear as `job:ID`, e.g.
`protonsync trash restore job:home-assistant 12`. Entries of the photo sync appear as
`photos:library`.

### 6.5 Drive: always keep local and free space

`PATH` is a normal path inside the drive, relative to the current directory or absolute:

```sh
protonsync pin ~/ProtonDrive/Taxes            # always keep the folder and its content local
protonsync unpin ~/ProtonDrive/Taxes/2019     # exception: this subfolder on demand only
protonsync free ~/ProtonDrive/Videos          # dry run: what would be freed?
protonsync free ~/ProtonDrive/Videos --apply  # actually free it
```

The closest rule wins: first the file itself, then its parent folders up to the drive, otherwise
`keep_local`. Freed files stay visible and are downloaded again the next time they are opened.
`free` skips pinned, open and not yet uploaded files and reports their number under "Kept".

### 6.6 Tray app

The icon shows the overall state:

| Icon | Meaning |
|---|---|
| Cloud with a green check mark | Everything up to date |
| Cloud with blue arrows | Syncing right now |
| Cloud with a yellow exclamation mark | Conflicts are waiting for a decision |
| Cloud with a red cross | The last sync or an upload job failed |
| Grey, crossed-out cloud | The daemon is not running or nobody is logged in |

The tooltip shows the state, the account and dry-run mode. A click opens the status window; the
menu (right click) offers **Sync now** (in dry-run mode: **Start dry run**), **Resolve
conflicts**, **Log in**/**Log out**, **Settings** and **Quit tray app**. The tray app does not stop
the daemon; it keeps syncing without it.

- **Overview:** account, sync folders with the result of the last sync (e.g. "3 up, 2 down"),
  drives with the cache in use, each with **Open folder**.
- **Conflicts:** for each conflict **Keep local**, **Keep Proton version** or **Keep both**, like
  `protonsync conflicts resolve`.
- **Settings:** language (same as system, German, English), notifications, autostart. The tray app
  only shows the daemon settings; **Open configuration** opens the file in an editor. Changes take
  effect after `systemctl --user restart protonsyncd`.
- **Log in:** username, password, then if needed 2FA code and mailbox password. The input goes
  straight to the daemon and is not stored.
- **Notifications** appear for new conflicts, an expired login, failed syncs and upload jobs that
  gave up. Clicking one opens the matching view.

The app's own settings live in `~/.config/protonsync/tray.toml`. If the daemon restarts, the tray
app reconnects by itself after a few seconds.

### 6.7 What is not synchronized?

- Files and folders whose name starts with `.protonsync-` (protonsync's own helper files, e.g. the trash
  and partially written files during a download).
- Symbolic links are not followed.
- Ignore patterns will follow in a later phase.

### 6.8 Photos and duplicates

```sh
protonsync photos                      # photo folder, number of photos, last sync
protonsync photos sync                 # dry run: what would be transferred?
protonsync photos sync --apply         # sync now
protonsync photos duplicates           # only show duplicates (with a suggestion which one stays)
protonsync photos duplicates trash ID… # check whether these photos may be deleted
protonsync photos duplicates trash ID… --apply   # move them to the trash
```

The **duplicate finder** is a separate function and never deletes on its own. It finds photos with
the same content (certainly the same photo) and photos with the same name and capture second but
different content (probably the same, e.g. recompressed). For each group it suggests which photo
stays: a favorite before a photo in albums before the one uploaded first. The video part of a live
photo does not count as a duplicate.

Deleting only happens with the ids from the list and with `--apply`. Unknown ids are refused, and
so is a command that would delete every photo of a group. Deleted photos go to the Proton trash and
appear in `protonsync trash` as `photos:library`; the local copy follows to the local trash at the
next sync. After `trash_retention_days` days they are deleted for good.

### 6.9 File managers: Dolphin and Nautilus

For files in a **drive**, the context menu (right click) has a **protonsync** submenu with
**Always keep local**, **On demand only** and **Free up space**. The items call the CLI (`pin`,
`unpin`, `free --apply`); a click is an explicit decision, so there is no dry run here. `free`
still keeps pinned, open and not yet uploaded files. Errors appear as a notification.

**Nautilus** (GNOME Files) also shows **emblems** on files in drives, sync folders and the photo
folder:

| Emblem | Meaning |
|---|---|
| Globe (`emblem-web`) | Online only, downloaded when opened |
| Arrows (`emblem-synchronizing`) | Partly downloaded, waiting for upload or for the next sync |
| Check mark (`emblem-default`) | Available locally or in sync |
| Exclamation mark (`emblem-important`) | Open conflict |

**Dolphin** only gets the menu, no icons on files (that would need a C++ plugin, ADR 0013). There,
`protonsync file-status` shows the state.

The package from [3.5](#35-alternative-as-a-package-archcachyos) already installs these files.
Installing by hand (from the source folder):

```sh
# Shared helper for both menus (must be in PATH)
install -Dm755 packaging/filemanager/protonsync-filemanager ~/.local/bin/protonsync-filemanager

# Dolphin (KDE): service menu, then reopen Dolphin
install -Dm755 packaging/filemanager/dolphin/protonsync.desktop \
  ~/.local/share/kio/servicemenus/protonsync.desktop

# Nautilus (GNOME): needs the python-nautilus package (Arch/CachyOS)
sudo pacman -S --needed python-nautilus
install -Dm644 packaging/filemanager/nautilus/protonsync_nautilus.py \
  ~/.local/share/nautilus-python/extensions/protonsync_nautilus.py
nautilus -q   # quit Nautilus; the extension is active the next time it opens
```

Error notifications need `notify-send` (package `libnotify`). The Nautilus extension asks the
daemon directly through its socket, once per folder, and remembers the answer for five seconds. If
the daemon is not running it simply shows nothing. New states therefore appear with a short delay;
`F5` reloads the folder.

The CLI also shows the state of single files, without asking Proton:

```sh
protonsync file-status ~/ProtonDrive/Taxes/2024.pdf ~/Documents/letter.odt
# /home/k/ProtonDrive/Taxes/2024.pdf: available locally, always local
# /home/k/Documents/letter.odt: in sync
```

Possible states: `online only (placeholder)`, `partly downloaded`, `available locally`,
`waiting for upload` (drive), `in sync`, `pending sync`, `conflict` (sync and photo folders),
`folder` and `not managed`. With `--json` the same values appear as `online_only`, `partial`,
`local`, `pending_upload`, `synced`, `pending`, `conflict`, `folder` and `outside`, plus `in_drive`
and `pinned`.

## 7. Running on a server

### 7.1 Option A: container (Docker/Podman)

The image only contains `protonsyncd` and `protonsync`, is statically linked and runs as
user `nonroot` (UID 65532). Everything you need is in `packaging/container/`.

```sh
git clone https://github.com/kafast/proton-drive-custom-client.git
cd proton-drive-custom-client/packaging/container

# 1. Adjust the configuration (sync pairs). Paths refer to the inside of the container.
$EDITOR config.toml

# 2. Generate the master key for the encrypted login file
openssl rand -hex 32 > protonsync-key
chmod 0400 protonsync-key
sudo chown 65532 protonsync-key

# 3. Make the sync folder on the host writable for the container
sudo mkdir -p /srv/backup && sudo chown 65532 /srv/backup

# 4. Build and start
docker compose up -d --build
docker compose logs -f              # view the log (Ctrl+C only stops the display)

# 5. Log in once
docker compose exec protonsync protonsync login
docker compose exec protonsync protonsync status
```

Notes:

- In `compose.yaml`, the host's `/srv/backup` is mounted as `/backup` in the container. Adjust this to
  your folders (e.g. an SMB/NFS share that is mounted on the host).
- The login is stored encrypted in the volume `protonsync-data`, the key separately as a
  Docker secret. Keep `protonsync-key` in a safe place; without it you have to log in again.
- Further commands always run via `docker compose exec protonsync protonsync …`.
- `TZ` in `compose.yaml` sets the local time for upload job schedules
  (default in the example: `Europe/Berlin`).
- With Podman, the same works with `podman compose`.
- Proxmox: easiest in a small VM or an LXC container with Docker; alternatively
  option B directly in the LXC.

### 7.2 Option B: systemd service (VM/LXC without a container)

You can build the programs yourself (`cargo build --release --no-default-features -p protonsync-daemon -p protonsync-cli`)
or get them from CI: every CI run contains the artifact `protonsync-x86_64-linux-musl` with both
static programs (GitHub → Actions → select a run → Artifacts).

```sh
sudo useradd --system --home-dir /var/lib/protonsync --shell /usr/sbin/nologin protonsync
sudo install -m 0755 protonsyncd protonsync /usr/local/bin/
sudo install -d -m 0750 -o protonsync /etc/protonsync
sudo sh -c 'openssl rand -hex 32 > /etc/protonsync/key && chmod 0400 /etc/protonsync/key'
sudo $EDITOR /etc/protonsync/config.toml        # enter secrets = "file" and the sync pairs
sudo cp packaging/systemd/protonsyncd.service /etc/systemd/system/
sudo systemctl daemon-reload
```

The service is hardened and may not write anything except its data directory. Therefore, allow the sync folders
in the unit (uncomment and adjust the `ReadWritePaths=` line), e.g. with
`sudo systemctl edit protonsyncd`:

```ini
[Service]
ReadWritePaths=/srv/backup
```

Start and log in:

```sh
sudo systemctl enable --now protonsyncd
sudo -u protonsync PROTONSYNC_SOCKET=/run/protonsync/daemon.sock protonsync login
sudo -u protonsync PROTONSYNC_SOCKET=/run/protonsync/daemon.sock protonsync status
journalctl -u protonsyncd -f
```

The key is passed only to the service via `LoadCredential` and is never stored in the data directory.

### 7.3 Web interface (optional)

The daemon can offer a lean web interface: overview, sync (plan or run), deciding conflicts,
restoring from the trash and purging expired entries
(plan first, then run), running upload jobs with their history, syncing photos and checking
duplicates (moved to the trash only after a second confirmation), viewing the settings.
It is **off by default**, needs its own password and works without JavaScript. Language: German
or English (switch at the top right, otherwise from the browser).

What it deliberately **cannot** do: log in to Proton (run `protonsync login` on the command line
once) and change the configuration (edit the file, restart the daemon).

**1. Create the password hash.** The password itself is stored nowhere; only an Argon2id hash goes
into the configuration. It needs at least 12 characters.

```sh
protonsyncd hash-password                          # desktop / systemd
docker compose run --rm protonsync hash-password   # container
```

**2. Add a `[web]` section:**

```toml
[web]
enabled = true
bind = "0.0.0.0:8470"           # default "127.0.0.1:8470": same machine only
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$…"   # output of step 1
allowed_networks = ["192.168.178.0/24"]               # optional, see below
```

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Turn the interface on |
| `bind` | `"127.0.0.1:8470"` | Address and port. `0.0.0.0:8470` (or `[::]:8470`) for the home network. |
| `password_hash` | – (required) | Hash from `protonsyncd hash-password` |
| `allowed_networks` | private networks | Only these networks (CIDR) get any answer, others get `403`. Default: `127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `::1/128`, `fc00::/7`. |
| `tls_cert`, `tls_key` | – | Certificate chain and private key (PEM) for HTTPS; only together. |
| `session_hours` | `12` | How long a login lasts. |

**3. Restart the daemon** and open `http://SERVER:8470/` (or `https://…`) in the browser. The log
reports `Web-Oberfläche bereit` with the address; if it cannot start (port in use, unusable TLS
files), the daemon keeps running without it and logs the error.

**HTTPS.** Without TLS the password and content cross the network unencrypted; the daemon then
warns at startup whenever `bind` is not just `127.0.0.1`. Two options:

- *Own certificate:* set `tls_cert`/`tls_key`. For a home network a self-signed certificate is
  enough (the browser asks once):
  ```sh
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 825 \
    -subj "/CN=protonsync" -addext "subjectAltName=DNS:server.fritz.box,IP:192.168.178.10" \
    -keyout key.pem -out cert.pem
  chmod 0600 key.pem
  ```
  The daemon user must be able to read both files (container: UID 65532, mount the files e.g.
  under `/config`).
- *Reverse proxy* (Caddy, nginx, Traefik) with HTTPS in front: keep `bind = "127.0.0.1:8470"` and
  point the proxy at that address. The interface then only sees the proxy's address;
  `X-Forwarded-For` is deliberately not evaluated. Network-based access control then belongs in
  the proxy.

**Container.** Inside the container set `bind = "0.0.0.0:8470"` (`127.0.0.1` would only be
reachable inside the container itself) and uncomment the `ports` line in `compose.yaml`:
`"127.0.0.1:8470:8470"` for a proxy on the same host, `"192.168.178.10:8470:8470"` for the home
network. Without an IP before the first colon the port would be open on **all** host addresses.

**Protection at a glance:** login cookie `HttpOnly`, `SameSite=Strict`, `Secure` with TLS; every
action needs a CSRF token; after 5 wrong passwords the address is locked for 5 minutes; strict
security headers (including a Content Security Policy, no embedding in other sites). Sessions
live in memory only: after a daemon restart you log in again.

### 7.4 Resources

Measured on 2026-10-16 with version 0.1.0 (not logged in, one sync pair):

| What | Value |
|---|---|
| Daemon memory when idle (RSS) | about 13 MB, 6 threads |
| CPU when idle | not measurable (0.0 %) |
| `protonsyncd` (desktop, glibc, with FUSE and Secret Service, stripped) | about 30 MB |
| `protonsync` (CLI, stripped) | about 2 MB |
| Static server binaries (musl) together, compressed | just under 12 MB |
| Server container when idle (`docker stats`, without file cache) | under 2 MB |
| Container image | about 34 MB |
| Arch package `protonsync-git` | about 12 MB |

CI measures the container's idle usage again on every run (step "Leerlauf-Speicher messen") and
also prints the image size. `docker stats` does not count the program file itself; the desktop
value (RSS) includes it. When logged in, the tree of remote files in memory comes on top; it
grows with the number of files. A measurement with a real account is still to be done. The
256 MB memory limit in `compose.yaml` leaves plenty of room for it.

## 8. Updating, logging out, uninstalling

**Updating (desktop)**

```sh
cd ~/proton-drive-custom-client && git pull
cargo install --locked --path crates/daemon && cargo install --locked --path crates/cli
cargo install --locked --path crates/gui-qt     # if the tray app is installed
systemctl --user restart protonsyncd
```

**Updating (package)**: run `makepkg -si` again in the `packaging/arch` folder, then
`systemctl --user restart protonsyncd`.

**Updating (container)**: `git pull`, then `docker compose up -d --build`.

**Logging out**: `protonsync logout` deletes the stored session. A new `login` is required afterwards.

**Uninstalling (desktop)**

```sh
protonsync logout
systemctl --user disable --now protonsyncd
rm ~/.config/systemd/user/protonsyncd.service
cargo uninstall protonsync-daemon protonsync-cli
cargo uninstall protonsync-gui-qt                         # if installed
rm -f ~/.config/autostart/protonsync-tray.desktop
rm -r ~/.config/protonsync ~/.local/share/protonsync     # configuration and state
```

With the package, instead of the `cargo` lines and the `rm …/protonsyncd.service`:
`sudo pacman -Rns protonsync-git`.

Your files in the sync folders and in Proton stay unchanged. You can delete the
`.protonsync-trash` folder in a sync folder by hand afterwards.

## 9. Troubleshooting

### 9.1 Logs

| Environment | Command |
|---|---|
| Desktop | `journalctl --user -u protonsyncd -f` |
| Server (systemd) | `journalctl -u protonsyncd -f` |
| Container | `docker compose logs -f` |

More details: set the environment variable `RUST_LOG=debug`, e.g. with
`systemctl --user edit protonsyncd`:

```ini
[Service]
Environment=RUST_LOG=debug
```

Passwords and keys never appear in the log.

### 9.2 Common problems

The messages are shown in English; with a German locale the CLI prints the German text given in parentheses.

| Message / behavior | Cause and solution |
|---|---|
| `Daemon not reachable` (`Daemon nicht erreichbar`, exit 3) | The daemon is not running: `systemctl --user status protonsyncd`. Or CLI and daemon use different sockets: check `PROTONSYNC_SOCKET`. |
| `Not logged in` (`Nicht angemeldet`, exit 4) | Run `protonsync login`. If this comes back after some time, the session at Proton has expired or was ended (e.g. by a password change). |
| `Login rejected` (`Anmeldung abgelehnt`, exit 5) | Password or 2FA code wrong, or the code had already expired. Try again. |
| A security key is requested | Accounts with only a FIDO2 key are not supported; also set up a 2FA app. |
| Nothing happens | Is `dry_run = true`? Then it only plans. `protonsync status` shows the mode. |
| A job runs at the wrong time | The time zone is missing (UTC applies). Set `TZ=Europe/Berlin` in the container; at startup the log names the time zone in use. |
| A job reports "not found" for the source folder | The share is not mounted, or not bound into the container. An unmounted, empty mount point, on the other hand, reports no error, just no new files. |
| A job uploads a file again and again | The file is rewritten with every backup (same name, different content). This is intended: Proton gets a new version each time. |
| `secrets = "keyring" is not available in this build` (`secrets = "keyring" ist in diesem Build nicht verfügbar`) | Server build without keyring: set `secrets = "file"` in the configuration. |
| Error about the master key | With `secrets = "file"`, a key must be present: `PROTONSYNC_KEY_FILE` (file with 64 hex characters, permissions at most 0600) or the systemd credential `protonsync-key`. |
| Keyring asks for permission / is locked | Unlock it at desktop login; when running without a desktop session, use `secrets = "file"`. |
| `Permission denied` in the container | The sync folder or key file is not owned by UID 65532: `sudo chown 65532 …`. |
| `Invalid configuration …` (`Ungültige Konfiguration …`) | The message names the error (e.g. relative path, duplicate `id`, unknown key). Fix the file, restart the daemon. |
| Web interface: `403 Forbidden` | Your address is not in `allowed_networks`. Behind Docker or a proxy the interface sees the address of the bridge or proxy. |
| Web interface: login locked | 5 wrong passwords: wait 5 minutes. Forgotten password: run `protonsyncd hash-password` again, replace `password_hash`, restart the daemon. |
| Web interface not reachable | Is `bind` still `127.0.0.1`? In the container, `0.0.0.0` and a `ports` entry are needed. The log at startup says whether and where it is listening. |
| Drive: `Transport endpoint is not connected` | The daemon was killed. On the next start it cleans up the old mount itself; to fix it right away: `fusermount3 -u ~/ProtonDrive`. |
| Drive is not mounted ("connected, not mounted") | The log names the reason. Common: `fuse3` is missing, or something is already mounted at that path. Pinning and freeing still work. |
| Drive is empty | Not logged in: `protonsync mounts` shows "not connected". Run `protonsync login`. |
| A file in the drive opens slowly | The content is downloaded on first read. Use `protonsync pin` for frequently used folders. |
| Tray icon does not appear (GNOME) | Install and enable the "AppIndicator and KStatusNotifierItem Support" extension. |
| Tray app says "Daemon not reachable" | Start the daemon (`systemctl --user start protonsyncd`). If it runs with its own `PROTONSYNC_SOCKET`, the tray app must see the same value. |
| `protonsync-tray` does not start: Qt platform or QML module missing | Install the Qt packages from [3.1](#31-install-the-tools-cachyosarch); `protonsync-tray --check` in a terminal shows the exact message. |
| Socket already in use | A second daemon is already running. Only start one daemon per user. |

## 10. Files and paths

| What | Desktop | Server (systemd) | Container |
|---|---|---|---|
| Configuration | `~/.config/protonsync/config.toml` | `/etc/protonsync/config.toml` | `/config/config.toml` |
| Data directory | `~/.local/share/protonsync` | `/var/lib/protonsync` | `/data` |
| State database | `<Daten>/state.sqlite` | same | same |
| Encrypted login (`secrets = "file"`) | `<Daten>/secrets/` | same | same |
| Socket | `$XDG_RUNTIME_DIR/protonsync/daemon.sock` | `/run/protonsync/daemon.sock` | `/data/run/daemon.sock` |
| Local trash | `<Sync-Ordner>/.protonsync-trash` | same | same |
| Drive cache | `<data>/cache/<ID>/` | – | – |
| Tray app settings | `~/.config/protonsync/tray.toml` | – | – |
| Dolphin menu | `~/.local/share/kio/servicemenus/protonsync.desktop` | – | – |
| Nautilus extension | `~/.local/share/nautilus-python/extensions/protonsync_nautilus.py` | – | – |

`<Daten>` stands for the data directory, `<Sync-Ordner>` for the local folder of a sync pair.

Environment variables:

| Variable | Meaning |
|---|---|
| `PROTONSYNC_CONFIG` | Path of the configuration file |
| `PROTONSYNC_DATA_DIR` | Data directory |
| `PROTONSYNC_SOCKET` | Path of the socket (daemon and CLI) |
| `PROTONSYNC_KEY_FILE` | File with the master key (64 hex characters, permissions ≤ 0600) |
| `PROTONSYNC_KEY` | Master key directly (only if nothing else works) |
| `CREDENTIALS_DIRECTORY` | set by systemd; `protonsync-key` in it takes precedence |
| `RUST_LOG` | Log level (`error`, `warn`, `info`, `debug`, `trace`) |
| `LANG`, `LC_ALL`, `LC_MESSAGES` | Language of the CLI output |

## 11. Security

- **What is stored?** Only session tokens and the key passphrases derived from them,
  never the password. This data is still as valuable as the password: whoever has it can access
  your Proton Drive.
- **Desktop (`secrets = "keyring"`)**: stored in the Secret Service (GNOME Keyring/KWallet) under the
  service name `protonsync`.
- **Server (`secrets = "file"`)**: stored in the data directory, encrypted with AES-256-GCM
  (file permissions 0600, directory 0700). The master key comes separately from a
  systemd credential, a key file or an environment variable. Do not put the key and the
  data directory into the same backup.
- **Web interface**: off by default. If you turn it on, use HTTPS (own certificate or reverse proxy) as soon as it is reachable from other machines, and a long password. Never expose it to the internet directly; use a VPN (e.g. WireGuard) for access from outside.
- **Access to the daemon**: the socket has permissions 0600; only the user the daemon runs as
  can use it.
- **Identification towards Proton**: protonsync honestly identifies itself as the third-party app
  `external-drive-protonsync@0.1.0-alpha`. Proton can block individual versions.
- **End-to-end encryption**: encryption and decryption happen locally, as with the official
  apps; Proton sees neither contents nor file names.

## 12. For developers: tests and live tests

```sh
cargo test                                   # all tests, no account, no network
cargo test -p protonsync-application         # a single crate
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p protonsync-fuse --test mount -- --ignored   # real FUSE drive (needs fuse3)
QT_QPA_PLATFORM=offscreen cargo run -p protonsync-gui-qt -- --check   # load the tray app without a screen
python3 -m unittest discover -s packaging/filemanager/tests   # file manager files
```

Live tests against a real account only run on request and only in a dedicated test folder in Proton
(default `protonsync-live-tests`, created if missing). Each test works in a fresh subfolder.

```sh
export PROTONSYNC_LIVE_USERNAME='name@proton.me'
read -rs PROTONSYNC_LIVE_PASSWORD && export PROTONSYNC_LIVE_PASSWORD
# optional: PROTONSYNC_LIVE_DATA_PASSWORD (mailbox password), PROTONSYNC_LIVE_TOTP (current code),
#           PROTONSYNC_LIVE_FOLDER (different test folder)
cargo test -p protonsync-adapter-proton --test live_contract -- --ignored --test-threads=1
```

`read -rs` reads the password without displaying it, so that it does not end up in the shell history.
The 2FA code is only valid for 30 seconds; set it right before the call.
