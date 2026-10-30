# Proxmox: setting up a read-only backup of shared folders

> Deutsch: [proxmox-lesebackup.md](proxmox-lesebackup.md)

This guide sets up a small instance on a Proxmox host that runs protonsync without a UI and backs
up selected Proton folders **read-only**. It is based on:

- a **second Proton account** ("reader account") with which the main account shares the folders
  as **"Viewer"** (manual 4.4, ADR 0022). Proton itself then prevents protonsync from changing
  anything, even with a bug in the program or a compromised server;
- sync pairs with `shared = true` and `direction = "backup_only"` (ADR 0019): new versions are
  added, old ones move to `_archiv/`, files deleted in Proton stay local;
- a warning when the disk is almost full (ADR 0023) and notifications via ntfy/email (manual 4.9);
- protonsync as a hardened systemd service with an encrypted login file (manual 7.2).

protonsync is unofficial and not supported by Proton.

## Contents

1. [LXC or VM?](#1-lxc-or-vm)
2. [Prepare Proton](#2-prepare-proton)
3. [Create the container in Proxmox](#3-create-the-container-in-proxmox)
4. [Install protonsync (without building)](#4-install-protonsync-without-building)
5. [Configuration](#5-configuration)
6. [Set up the service and log in](#6-set-up-the-service-and-log-in)
7. [Dry run, then continuous operation](#7-dry-run-then-continuous-operation)
8. [Check that the protection works](#8-check-that-the-protection-works)
9. [Operation: disk space, updating, backing up the backup](#9-operation)
10. [Troubleshooting](#10-troubleshooting)

## 1. LXC or VM?

**Recommendation: an unprivileged LXC container with Debian and protonsync as a systemd service**
(manual 7.2, "variant B"), without Docker.

| | LXC (recommended) | VM |
|---|---|---|
| Resources | about 13 MB RAM idle for protonsync, no kernel of its own | own kernel, at least a few hundred MB RAM |
| FUSE | not needed | not needed |
| Docker | not recommended in LXC by Proxmox | possible (manual 7.1) |
| Isolation | good (unprivileged, own user, hardened unit) | even stronger |

FUSE is only needed for the on-demand drive (`[[mount]]`, desktop only). A backup stores
everything fully locally, so FUSE and any special privileges for the container are not needed. A
VM is only worth it if you want to use Docker anyway or do not trust the container kernel; then
use manual 7.1 with the same configuration as below.

## 2. Prepare Proton

1. **Create a second Proton account** (the free plan is enough, e.g. `backup-reader@proton.me`).
   Set up an authenticator app (TOTP) for it; a security key (FIDO2) as the only second factor is
   not supported.
2. **In the main account**, on the web, share each folder to back up by email with the reader
   account, role **"Viewer"** (not "Editor").
3. **Log in to the web with the reader account** and accept each invitation under "Shared with
   me". The folder names shown there are the values for `remote` later.

The server never needs the main account's password.

## 3. Create the container in Proxmox

In the Proxmox UI, "Create CT" (or `pct create` in the host shell):

| Setting | Value |
|---|---|
| Template | Debian 12 or 13 (`local` → CT Templates → Templates → `debian-12-standard`) |
| Unprivileged container | **yes** |
| Nesting | **yes** (default for new containers; systemd needs it for the unit's hardening) |
| Root disk | 4 GB |
| CPU / RAM / swap | 1 core / 512 MB / 512 MB |
| Network | DHCP or a fixed IP in the home network |

**Storage for the backup:** easiest is a separate mount point from a Proxmox storage
(container → Resources → Add → Mount Point), e.g. 500 GB on `local-zfs`, path `/backup`. Be
generous: `_archiv/` is never emptied automatically. With "Backup" ticked, `vzdump` includes the
mount point.

In the host shell (container number 120 as an example):

```sh
pct create 120 local:vztmpl/TEMPLATE.tar.zst \
  --hostname protonsync --unprivileged 1 --features nesting=1 \
  --cores 1 --memory 512 --swap 512 --rootfs local-zfs:4 \
  --mp0 local-zfs:500,mp=/backup,backup=1 \
  --net0 name=eth0,bridge=vmbr0,ip=dhcp --onboot 1
pct start 120
pct enter 120
```

(Replace `TEMPLATE` with the name from `pveam list local`, e.g. `debian-12-standard_…_amd64`.)

*Alternative:* bind an existing host folder (`--mp0 /tank/backup,mp=/backup`). Because the
container is unprivileged, files in it belong on the host to UID "100000 + UID in the container";
`chown` the folder on the host accordingly (step 4 shows the service user's UID with
`id protonsync`).

## 4. Install protonsync (without building)

Only the two statically linked programs `protonsyncd` and `protonsync` are needed (they run
without further libraries). Sources:

- **After the first release** (none published yet): under
  [Releases](https://github.com/kafast/proton-drive-custom-client/releases) the file
  `protonsync-VERSION-x86_64-linux-musl.tar.gz`. Directly in the container:
  ```sh
  VERSION=0.1.0
  apt install -y curl
  curl -LO "https://github.com/kafast/proton-drive-custom-client/releases/download/v$VERSION/protonsync-$VERSION-x86_64-linux-musl.tar.gz"
  tar -xzf "protonsync-$VERSION-x86_64-linux-musl.tar.gz"
  cd "protonsync-$VERSION-x86_64-linux-musl"
  ```
- **Until then from CI:** on GitHub open "Actions" → "CI", the latest green run of `main`, and
  download `protonsync-x86_64-linux-musl` under "Artifacts" (GitHub login required, the file
  expires after 90 days). Copy it to the Proxmox host and push it into the container:
  ```sh
  # on the Proxmox host
  pct push 120 protonsync-x86_64-linux-musl.zip /root/protonsync.zip
  # in the container
  apt install -y unzip && cd /root && unzip protonsync.zip
  ```
  The run must be from 2026-10-30 or later, otherwise `shared = true` and the disk space warning
  are missing.

Then in the container (as `root`), in the folder with the two programs:

```sh
apt install -y openssl
useradd --system --home-dir /var/lib/protonsync --shell /usr/sbin/nologin protonsync
install -m 0755 protonsyncd protonsync /usr/local/bin/
protonsync --version
install -d -m 0750 -o protonsync /etc/protonsync
sh -c 'umask 077; openssl rand -hex 32 > /etc/protonsync/key'
install -d -m 0750 -o protonsync -g protonsync /backup/proton
id protonsync          # note the UID if a host folder is bound (section 3)
```

`/etc/protonsync/key` is the master key of the encrypted login file. It belongs to `root`; the
service only receives it as a systemd credential. Keep a copy somewhere safe (e.g. a password
manager); without it, a restore only needs a new `protonsync login`, nothing is lost.

## 5. Configuration

Create `/etc/protonsync/config.toml` (`nano /etc/protonsync/config.toml`). All keys with
explanations: `protonsync config example` or manual 4.

```toml
[general]
dry_run = true                 # only plan at first (section 7)
secrets = "file"               # encrypted file instead of a keyring
trash_retention_days = 30
disk_check_minutes = 15        # check disk space (ADR 0023)
disk_low_free_percent = 10     # "almost full"
disk_full_free_percent = 2     # "full"

# One [[pair]] per shared folder. remote = name under "Shared with me",
# optionally with a subfolder ("Family/Photos").
[[pair]]
id = "family"
local = "/backup/proton/family"
remote = "Family"
shared = true
direction = "backup_only"

[[pair]]
id = "documents"
local = "/backup/proton/documents"
remote = "Documents"
shared = true
direction = "backup_only"

# Notifications, e.g. via ntfy (phone app) or email.
[notify]
language = "en"

[notify.webhook]
url = "https://ntfy.sh/a-long-secret-topic"
format = "text"
sync_failures = true
login_required = true
disk_space = true
weekly_report = true           # an overview every Monday at 8:00
```

Notes:

- `shared = true` only allows `backup_only` and `download_only`; the daemon refuses any other pair
  at startup.
- `backup_only` rather than `download_only`, so that deleting or encrypting in the main account
  does not hit the local backup (old versions end up in `_archiv/`, manual 4.3).
- Instead of ntfy, email works too via `[notify.mail]` (manual 4.9). The SMTP password belongs in a
  file with mode 0600 or in a systemd credential, never in the configuration.
- `[web]` stays off. If you want the web interface: manual 7.3. Because of the hardened unit the
  daemon cannot change `/etc/protonsync/config.toml` itself; change settings in the file and
  restart the service.

## 6. Set up the service and log in

The unit `protonsyncd.service` is included in the release archive (then
`install -m 0644 protonsyncd.service /etc/systemd/system/`); the CI artifact lacks it, so download
it from GitHub:

```sh
curl -Lo /etc/systemd/system/protonsyncd.service \
  https://raw.githubusercontent.com/kafast/proton-drive-custom-client/main/packaging/systemd/protonsyncd.service
```

Then add an override:

```sh
systemctl edit protonsyncd
```

In the editor, between the comment lines, enter this so the service may write the backup folder:

```ini
[Service]
ReadWritePaths=/backup/proton
```

Start and log in with the **reader account**:

```sh
systemctl daemon-reload
systemctl enable --now protonsyncd
systemctl status protonsyncd                     # "active (running)"

# Shortcut for the CLI as the service user (add to /root/.bashrc):
alias ps-cli='runuser -u protonsync -- env PROTONSYNC_SOCKET=/run/protonsync/daemon.sock protonsync'

ps-cli login                                     # user name, password, 2FA code of the reader account
ps-cli status
```

Only session keys are stored, encrypted in `/var/lib/protonsync`, never the password. After a
restart the daemon logs in again by itself. If the session expires for good, a "login required"
notification is sent; then run `ps-cli login` again.

## 7. Dry run, then continuous operation

```sh
ps-cli sync                         # only plans: what would be downloaded?
ps-cli sync --pair family --apply   # really run it once
ls -la /backup/proton/family
```

If everything looks right, set `dry_run = false` in `/etc/protonsync/config.toml` and run
`systemctl restart protonsyncd`. From now on the daemon fetches changes by itself: via Proton
events (`event_poll_seconds`, default 60 s) and, as a safety net, a full sync every hour.

View the log: `journalctl -u protonsyncd -f`.

## 8. Check that the protection works

Two points have **not yet been checked** with a real account (trial 3.13 and 3.14). Please go
through them once before relying on the setup:

1. **Viewer cannot delete:** on the web, with the reader account, try to delete or rename a file in
   a shared folder. Proton must refuse. If it works, the folder is shared as "Editor"; fix the
   share.
2. **Changes arrive:** in the main account, change a file in the shared folder. Within about a
   minute the log shows a sync, the new version is in `/backup/proton/…` and the old one in
   `_archiv/`. If it only arrives after up to an hour, the events of the foreign account do not
   reach the reader account and only the hourly sync works (please report it; then consider a
   smaller `full_sync_minutes`).
3. **Disk full warning:** temporarily set `disk_low_free_percent = 99`,
   `disk_full_free_percent = 98`, `disk_check_minutes = 1`, restart the service. The ntfy/email
   message must arrive shortly afterwards. Reset the values and restart.

Feel free to tick off the results in `docs/erprobung.md`.

## 9. Operation

**Disk space.** `_archiv/` grows with every changed file and is never emptied automatically. The
warning comes below 10 % and 2 % free space. A full disk breaks nothing (files are first
downloaded completely, then moved into place); only new versions are missing until there is room
again. To make room: delete old files in `_archiv/` or enlarge the mount point (Proxmox:
container → Resources → Mount Point → Resize).

**Updating.** Fetch the new programs as in section 4, then:

```sh
install -m 0755 protonsyncd protonsync /usr/local/bin/
systemctl restart protonsyncd
```

Only reinstall the unit if it changed (the `systemctl edit` override is kept).

**Backing up the backup.** Since protonsync has write access to `/backup` inside the container,
snapshots outside the container add protection, e.g. regular ZFS snapshots of the mount point on
the host or `vzdump` to another target (Proxmox Backup Server). This keeps a state even if
something goes wrong inside the container itself.

## 10. Troubleshooting

| Problem | Solution |
|---|---|
| Service does not start, `status=226/NAMESPACE` | "Nesting" is off for the container. Container → Options → Features → enable Nesting, restart the container. |
| `status=243/CREDENTIALS` | The key is missing at `/etc/protonsync/key` or not readable by `root`. |
| `Read-only file system` when writing to `/backup/…` | `ReadWritePaths=` is missing or points to another folder (section 6). |
| `Permission denied` in `/backup/…` | The folder does not belong to `protonsync`. With a bound host folder: `chown` on the host to 100000 + UID (section 3). |
| `no folder "…" in "Shared with me"` | Invitation not accepted, name in `remote` spelled differently, or logged in with the wrong account. |
| CLI says the daemon is not reachable | Use `ps-cli` (socket `/run/protonsync/daemon.sock`) and check that the service is running. |
| Other | Manual 9. |
