# protonsync User Manual

German version: [handbuch.md](handbuch.md)

Status: Phase 2 (upload jobs). This manual covers installation, setup, operation and troubleshooting
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

### 3.1 Install the tools (CachyOS/Arch)

```sh
sudo pacman -S --needed base-devel cmake git rustup
rustup default stable
```

### 3.2 Get the source code and build

```sh
git clone https://github.com/kafast/proton-drive-custom-client.git ~/proton-drive-custom-client
cd ~/proton-drive-custom-client
cargo install --locked --path crates/daemon     # installs protonsyncd to ~/.cargo/bin
cargo install --locked --path crates/cli        # installs protonsync to ~/.cargo/bin
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
after it; the repeated hour in autumn triggers only one run. Times missed because the daemon was not
running are not made up.

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
| 7 | Partially failed (at least one pair had errors, or a job run failed) |

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

### 6.4 Trash

- If a file is deleted **in Proton**, protonsync moves the local copy to the local
  trash `.protonsync-trash` in the sync folder.
- If a file is deleted **locally**, protonsync moves it to the Proton trash in Proton.
- `protonsync trash` shows both kinds; `restore` brings entries back.
- After `trash_retention_days` days, entries are deleted for good. The daemon checks this once
  a day; in dry-run mode it only shows what it would delete.

Trash entries created by an upload job's retention rule appear as `job:ID`, e.g.
`protonsync trash restore job:home-assistant 12`.

### 6.5 What is not synchronized?

- Files and folders whose name starts with `.protonsync-` (protonsync's own helper files, e.g. the trash
  and partially written files during a download).
- Symbolic links are not followed.
- Ignore patterns and "on demand" (placeholders) will follow in later phases.

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

### 7.3 Resources

Together, the programs are just under 9 MB (compressed). The container has a memory limit of
256 MB; when idle, the daemon needs much less. Exact measurements will follow.

## 8. Updating, logging out, uninstalling

**Updating (desktop)**

```sh
cd ~/proton-drive-custom-client && git pull
cargo install --locked --path crates/daemon && cargo install --locked --path crates/cli
systemctl --user restart protonsyncd
```

**Updating (container)**: `git pull`, then `docker compose up -d --build`.

**Logging out**: `protonsync logout` deletes the stored session. A new `login` is required afterwards.

**Uninstalling (desktop)**

```sh
protonsync logout
systemctl --user disable --now protonsyncd
rm ~/.config/systemd/user/protonsyncd.service
cargo uninstall protonsync-daemon protonsync-cli
rm -r ~/.config/protonsync ~/.local/share/protonsync     # configuration and state
```

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
