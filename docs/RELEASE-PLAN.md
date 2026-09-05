# Release plan — prebuilt executables + Linux release

**Status:** ✅ **M1–M2 built and verified locally, 2026‑09‑05** (macOS universal binary +
ad-hoc sign, both Linux musl targets via `cargo-zigbuild --features vendored`, `.deb` via
`cargo-deb`, `.rpm` via `cargo-generate-rpm`, checksums + minisign signature, all cross-verified)
— **not yet published** (`gh release create` needs an explicit go/no-go, see §11). Found and
fixed two real bugs that only surfaced by actually running the tools: a `libc::ioctl` request-type
mismatch between glibc and musl (`serialtransport.rs`), and `cargo-generate-rpm` v0.21.0's actual
schema for `post_install_script` (a string, not a table) plus its default `ldd`-based
auto-requires needing explicit disabling on a host with no `ldd`. **Feeds:** ROADMAP Phase 4
**Supersedes in detail:** [`RELEASING.md`](RELEASING.md) (which stays as the short pre-release checklist)
**Companions:** [`LINUX-TESTING.md`](LINUX-TESTING.md) (validation) · [`MACOS-NATIVE.md`](MACOS-NATIVE.md) (dropping OrbStack)

Today the only way to get `ktflash` is `git clone && cargo build --release`, which needs a Rust
toolchain, `libusb`, and (on macOS) OrbStack. This plan closes two gaps:

- **A. Executable release** — a user downloads one file, verifies it, and runs it. Ships
  **unsigned/ad-hoc-signed on macOS now**, with notarization planned as a later milestone (§4).
- **B. Linux release** — first-class **AlmaLinux** and **Debian** hosts via plain binaries and
  native packages. **No containers.** See §6 for why.

---

## 0. Constraints this plan must respect

| Constraint | Source | Consequence |
|---|---|---|
| **No GitHub Actions** — releases build locally / on a box you control | `ci.sh`, `RELEASING.md`, ROADMAP | One `release.sh` on a maintainer machine |
| **No containers in the toolchain** | this revision | Cross-compilation via `cargo-zigbuild`, not `cross`/Docker (§3.2) |
| **No firmware images distributed** | LICENSE / README | Packages ship the binary only; `.gitignore` already blocks `*.bin` |
| **Flashing can brick hardware, no readback** | README, Phase 5 | Easier distribution must not make destructive commands easier to hit *by accident*; `--execute --yes` gates stay |
| **Honest status labels** | ROADMAP conventions | Linux-native `flash-cdc` is ⏳ untested on real hardware. Artifacts ship labeled `probe/unlock verified · native write unverified on Linux-native` until [`LINUX-TESTING.md`](LINUX-TESTING.md) P3 signs it off |
| **Supply chain matters more than usual** | it erases hardware | Signed `SHA256SUMS`, committed `Cargo.lock`, `cargo audit`/`cargo deny` mandatory at release time (unlike `ci.sh`, which skips them) |

---

## 1. Current state (verified in-tree)

- `flasher/` — Rust crate `ktflash` v1.1.1; deps `rusb 0.9.4`, `ratatui`, `crossterm`, `serde`,
  `sha2`. Release profile already `strip`/`opt-level="z"`/`lto`.
- `flasher/ci.sh` — clippy `-D warnings`, 70 tests, replay smoke, optional `cargo audit`/`deny`.
- `packaging/99-ktflash.rules` — udev `uaccess` for `31b2:*`, `2972:0102`, `8888:cdc0` + its tty.
- **Nothing exists for:** cross-compilation, packaging, checksums, signing, or an installer.

**Local toolchain gaps** (this machine): `cargo-zigbuild` and `zig` absent; only
`aarch64-apple-darwin` installed as a rustup target. §3 provisions these — all via `brew`/`cargo
install`, no container runtime.

---

## 2. Key technical decision: statically linked musl binaries

The highest-leverage choice. `rusb 0.9.4` exposes a **`vendored`** feature (→
`libusb1-sys/vendored`) that compiles libusb from source. Reading `libusb1-sys 0.7.0`'s
`build.rs`: on Linux it *always* compiles `linux_netlink.c` + `linux_usbfs.c`, and only adds
`linux_udev.c` **if `pkg_config` finds libudev**. Build without `libudev` headers present and
vendored libusb comes out udev-free, enumerating via sysfs/usbfs.

**⇒ `x86_64-unknown-linux-musl` + `aarch64-unknown-linux-musl` with `--features vendored` yields a
fully static, zero-dependency `ktflash`** that runs unmodified on AlmaLinux 8/9/10, Debian 11/12/13,
Ubuntu, Fedora, and Arch.

One artifact solves "no toolchain needed", the Alma-vs-Debian glibc split (Alma 9 = glibc 2.34,
Debian 12 = 2.36), and the "which distro package do I need" question.

**Action:** add to `flasher/Cargo.toml`

```toml
[features]
vendored = ["rusb/vendored"]
```

so the release build is `cargo build --release --features vendored`, and the from-source developer
build against system libusb is unchanged.

**Fallback if musl misbehaves** (e.g. a `crossterm`/`ratatui` terminal-sizing surprise under musl):
build glibc binaries natively **on the AlmaLinux 9 VM** — the oldest glibc in the support set, so
the artifact still runs on Debian 12/13 and Ubuntu 22.04+. Verify the floor with
`objdump -T ktflash | grep GLIBC_ | sort -u`.

> **Gate:** the musl binary must pass `bootdiag --replay fixtures/synthetic-success.json`,
> `compat --template`, and a real `probe` on a Linux host with a dongle, before it is promoted
> over the glibc fallback. Confirm staticness with `file ktflash` → "statically linked" and
> `ldd ktflash` → "not a dynamic executable".

---

## 3. Workstream A — release engineering foundation

Deliverable: `scripts/release.sh` — one command, reproducible, no CI service, no containers.

### 3.1 Gates

1. **Pin the toolchain.** `rust-toolchain.toml` (`channel = "1.98.0"`, components, the four
   release targets). Record `rustc -Vv` in the release notes.
2. **Hard-gate.** `release.sh` refuses unless: working tree clean, tag matches `Cargo.toml`
   version, `ci.sh` passes, **and** `cargo audit` + `cargo deny check` are installed and pass.

### 3.2 Container-free cross-compilation

Two routes, both container-free; use both and compare:

| Route | How | Role |
|---|---|---|
| **`cargo-zigbuild`** (primary) | `brew install zig && cargo install cargo-zigbuild`; `cargo zigbuild --release --features vendored --target x86_64-unknown-linux-musl` | Fast, local, no VM. Zig is a self-contained C cross-compiler shipping musl/glibc headers — it compiles vendored libusb's C too |
| **Native build on the Linux VMs** (authoritative) | `cargo build --release --features vendored` on each VM from [`LINUX-TESTING.md`](LINUX-TESTING.md) | No cross-compile magic to trust; the reference build if zigbuild output is ever suspect |

> **Verify early:** that `cargo-zigbuild` correctly wires `CC_x86_64_unknown_linux_musl` so the
> `cc` crate builds vendored libusb. This is the one unproven link in the primary route. If it
> fails, the native-on-VM route is the fallback and costs only build time.

`cross` is deliberately not used — it requires Docker/Podman.

### 3.3 Artifacts, checksums, signing

- **Naming:** `ktflash-<version>-<target>.tar.gz`, e.g.
  `ktflash-1.2.0-x86_64-unknown-linux-musl.tar.gz`. Each contains `ktflash`, `LICENSE.md`,
  `README.md`, and — for Linux — `99-ktflash.rules`.
- `shasum -a 256 * > SHA256SUMS`, then sign. Recommend **minisign** (one pubkey line in the
  README, no keyserver ceremony); pubkey committed at `packaging/ktflash.pub`.
- **SBOM:** `cargo cyclonedx` → `ktflash-<ver>.cdx.json`.
- **Publish:** `gh release create` (the `gh` CLI is already available) with the caution block and
  the honest support-boundary table.

---

## 4. Workstream B — macOS executable release (unsigned now)

macOS is identify-only today (see [`MACOS-NATIVE.md`](MACOS-NATIVE.md) for the plan to change
that), but it is the first-run experience most users hit.

### 4.1 Build

`aarch64-apple-darwin` + `x86_64-apple-darwin`, then `lipo -create` into a **universal binary** —
one download, no "which Mac do I have?".

### 4.2 Ad-hoc signing is mandatory, not optional

On Apple Silicon, a binary with **no** signature at all is killed by the kernel. Ad-hoc signing is
free, needs no Apple account, and satisfies this:

```sh
codesign --force --sign - ktflash
codesign --verify --verbose ktflash
```

**`lipo` invalidates existing signatures — always ad-hoc sign *after* the `lipo` step.** This is
the most common way a universal-binary release ships broken.

### 4.3 How users run the unsigned build

The distinction that matters: **Gatekeeper quarantine is applied by the downloader, not by the
file.** Browsers and LaunchServices set `com.apple.quarantine`; `curl` does not.

- **Recommended path — `install.sh` via `curl`: no Gatekeeper prompt at all.** Because the
  installer fetches with `curl`, nothing is quarantined and the binary just runs. This should be
  the headline install instruction.
- **Manual path — `curl` the tarball, extract with `tar` in Terminal.** Also unquarantined.
- **Browser-download path — quarantined.** The user sees *"cannot be opened because the developer
  cannot be verified."* Fix, documented verbatim in the release notes:

  ```sh
  xattr -d com.apple.quarantine ./ktflash    # or: xattr -c ./ktflash
  ./ktflash probe
  ```

  If macOS still refuses, **System Settings → Privacy & Security → "Open Anyway"**. Note that on
  macOS 15 (Sequoia) the old Control-click → Open bypass no longer works in all cases; "Open
  Anyway" is the current flow.

Release notes must be explicit that this build is **unsigned by an Apple Developer ID**, say why
(no account yet), and show the exact commands — rather than leaving users to guess, or worse,
training them to blanket-disable Gatekeeper. **Never tell users to run `spctl --master-disable`.**

### 4.4 Notarization — planned, later (M7)

Deferred, and blocked on an Apple Developer account ($99/yr). When it happens:

1. Developer ID Application certificate.
2. `codesign --force --options runtime --timestamp --sign "Developer ID Application: …"`.
3. `xcrun notarytool submit ktflash.zip --wait --keychain-profile …`.
4. **Stapling caveat:** `stapler staple` does **not** work on a bare Mach-O executable — the
   ticket can only be stapled to a `.dmg`, `.pkg`, or `.app` bundle. So full offline notarization
   means **also shipping a signed `.pkg`**; a notarized-but-unstapled raw binary relies on an
   online Gatekeeper ticket lookup and fails on an offline Mac. Decide `.pkg` vs. raw at that
   point.
5. **Then** a Homebrew tap (`ParkWardRR/homebrew-ktflash`). A tap is only worth doing after
   notarization — until then `install.sh` gives a better first-run experience than an unsigned
   formula would.

None of M1–M6 is blocked on this.

---

## 5. Workstream C — Linux host release (Alma + Debian)

### 5.1 Portable tarball (primary)

The static musl binaries from §2 plus `99-ktflash.rules`. Works on every target host, no deps.

### 5.2 Native packages

Packages matter mainly because they install the **udev rules** to the right place automatically —
the step users forget, whose failure mode (`LIBUSB_ERROR_ACCESS`) looks like a broken tool.

| Format | Tool | Notes |
|---|---|---|
| `.deb` (Debian 12/13, Ubuntu) | `cargo-deb` | `amd64` + `arm64` |
| `.rpm` (AlmaLinux 9/10, RHEL, Rocky, Fedora) | `cargo-generate-rpm` | `x86_64` + `aarch64` |

Layout for both: binary → `/usr/bin/ktflash`, rules → `/usr/lib/udev/rules.d/99-ktflash.rules`,
docs → `/usr/share/doc/ktflash/`. Post-install scriptlet:
`udevadm control --reload-rules && udevadm trigger`.

Both wrap the **static** binary, so neither declares a `libusb` dependency and both are portable
across their distro's supported releases. **Both tools are pure Rust and run natively on macOS** —
no container, no Debian/RPM host needed to build packages.

### 5.3 Two udev fixes to land before packaging

Found while reviewing `packaging/99-ktflash.rules` against the real Linux path:

1. **ModemManager will probe the bootloader.** After `unlock` the device enumerates as
   `8888:cdc0` and exposes a CDC-ACM tty. On both Debian and Alma, ModemManager opens new
   `ttyACM*` devices to probe for modems — a live race against `flash-cdc`'s claim of the same
   interface, and a plausible cause of intermittent flash failures. Add:

   ```udev
   SUBSYSTEM=="usb", ATTR{idVendor}=="8888", ATTR{idProduct}=="cdc0", ENV{ID_MM_DEVICE_IGNORE}="1"
   SUBSYSTEM=="tty", ATTRS{idVendor}=="8888", ATTRS{idProduct}=="cdc0", ENV{ID_MM_DEVICE_IGNORE}="1"
   ```

   `ktflash` already calls `set_auto_detach_kernel_driver(true)`
   (`flasher/src/usbtransport.rs:90`, `flasher/src/main.rs:775,822`), so `cdc_acm` itself is
   handled — ModemManager is a separate, currently unhandled actor.

2. **`8888` is a squatted/placeholder VID.** Every `8888` rule must keep its `cdc0` PID qualifier
   (they all do today — don't relax it), and the file should say why.

---

## 6. Why no containers

Dropped from this plan by decision, and the reasoning holds up:

- **Nothing is gained.** The static musl binary already has zero runtime dependencies, so a
  container image would be `FROM scratch` + one file — a heavier way to ship a file that runs
  fine on its own.
- **USB passthrough makes containers *harder*, not easier.** `ktflash unlock` re-enumerates the
  dongle under a new VID:PID, so its `/dev/bus/usb/BBB/DDD` node is replaced. A container pinned
  with `--device` sees the node vanish mid-operation and `flash-cdc` then fails to find the
  bootloader; the workaround (bind-mounting all of `/dev/bus/usb`) plus SELinux
  (`container_use_devices`) and rootless-Podman permission caveats is more to explain than
  `dnf install ./ktflash.rpm`.
- **They wouldn't have helped macOS anyway** — Docker Desktop and `podman machine` run in a VM
  with no USB forwarding, so a container image could never have replaced OrbStack there. The
  native path in [`MACOS-NATIVE.md`](MACOS-NATIVE.md) is the real answer.
- **Container-free cross-compilation exists** (`cargo-zigbuild`, §3.2), so containers aren't
  needed as build isolation either.

Not shipping a registry image also means no registry account, no token custody, and no
multi-arch manifest to maintain.

---

## 7. Workstream D — install UX

1. **`scripts/install.sh`** — `curl -fsSL <raw>/scripts/install.sh | sh`. Detects OS/arch, picks
   the artifact, **verifies the minisign signature and SHA-256 before executing anything**,
   installs to `~/.local/bin` (no sudo by default), and on Linux *offers* to install the udev
   rules behind an explicit `sudo` prompt. Refuses to run if verification fails. Never
   auto-flashes. Doubles as the Gatekeeper-free macOS path (§4.3).
2. **Docs.**
   - `README.md`: lead the Quickstart with **download**, demote *build from source*. Update the
     "Where it runs" table.
   - `docs/LINUX.md`: package install first (`dnf install ./ktflash*.rpm` /
     `apt install ./ktflash*.deb`), source build second, plus ModemManager notes.
   - `docs/FLASHING.md`: the unsigned-binary instructions from §4.3.
   - `docs/RELEASING.md`: shrink to a checklist pointing here.
   - `ROADMAP.md` Phase 4 / `CHANGELOG.md` (`v1.2.0`): update as milestones land.

---

## 8. Sequencing

**M1–M4 need no hardware, no VMs, and no purchases.**

| # | Milestone | Contents | Gate |
|---|---|---|---|
| **M1** | Build foundation | `vendored` feature, `rust-toolchain.toml`, `release.sh`, zigbuild targets, checksums + minisign | ✅ done — `ci.sh` green; both musl binaries verifiably static (`file` confirms) |
| **M2** | Linux artifacts | musl tarballs, `.deb`, `.rpm`, udev fixes (§5.3) | ✅ done — `.deb` contents inspected (binary, udev rules, docs, `postinst` all present, correct paths); `.rpm` lead magic verified; both built from the real static x86_64 musl binary. *Not yet installed on a live guest* — `dpkg`/`rpm` tooling doesn't exist on macOS to test that half |
| **M3** | macOS executable | universal binary, ad-hoc sign after `lipo`, unsigned-run docs | ✅ done — `lipo -info` confirms both arches, `codesign --verify` passes, `probe` runs and correctly identifies real hardware |
| **M4** | Install UX | `install.sh` with verify-before-install, README/docs rework | ⏳ script exists, syntax-checked, but never run end-to-end (needs a published release to download from) |
| **M5** | **v1.2.0 release** | `gh release create`, honest labels | ⏳ **awaiting an explicit go/no-go** — artifacts are built, signed, and verified locally (§0 status line); publishing is a separate, visible decision |
| **M6** | Hardware sign-off | Linux-native `unlock` + `flash-cdc` on real hardware | flips ROADMAP Phase 4.1 to ✅ |
| **M7** | Notarization + tap | Developer ID, `notarytool`, `.pkg` decision, Homebrew tap | **needs Apple Developer account** |
| **M8** | macOS native | see [`MACOS-NATIVE.md`](MACOS-NATIVE.md) — drops OrbStack | tracked separately; not a release blocker |

---

## 9. Risks and open questions

| Risk | Mitigation |
|---|---|
| `cargo-zigbuild` can't build vendored libusb's C | Native-on-VM build is the pre-planned fallback (§3.2); costs build time only |
| musl + `crossterm`/`ratatui` TUI regression | §2 gate; glibc-on-Alma-9 fallback |
| Vendored libusb without udev misses hotplug | `ktflash` enumerates on demand — verify the TUI refresh path doesn't use `libusb_hotplug_*` |
| Unsigned macOS binary scares users off | `install.sh` via `curl` avoids quarantine entirely; exact `xattr` command documented; never recommend disabling Gatekeeper |
| Easier distribution → more accidental bricks | `--execute --yes` gates stay; caution block in release notes and `install.sh` output |
| Signing key custody (minisign) | Offline with the maintainer; pubkey committed; rotation documented |

**Open questions:**

1. **Signing** — minisign (recommendation) vs GPG vs checksums-only?
2. **Notarization budget** — Apple Developer account in scope later? M7 only.
3. **aarch64 Linux** — worth shipping, or x86_64-only for v1.2.0? Depends on whether the VM host
   can run arm64 guests.

---

## 10. File inventory

> **Update, 2026‑09‑05:** the file inventory below has fully landed, and — unlike when this note
> was written — has now actually been run for real: a minisign key exists (§11), and
> `cargo-deb`/`cargo-generate-rpm`/`cargo-zigbuild` all produced real, verified artifacts. See §11
> for what that surfaced. `scripts/release.sh` itself (the one-command wrapper around all of
> this) has still not been executed end-to-end — the steps above were run individually by hand.

```
rust-toolchain.toml                 new   pinned channel + release targets
flasher/Cargo.toml                  edit  [features] vendored = ["rusb/vendored"]
flasher/Cargo.lock                  edit  (regenerated)
scripts/release.sh                  new   the whole matrix, one command, no containers
scripts/install.sh                  new   curl|sh installer, verifies before installing
packaging/99-ktflash.rules          edit  ModemManager ignore rules (§5.3)
packaging/ktflash.pub               new   minisign public key
packaging/deb/                      new   cargo-deb assets/metadata
packaging/rpm/                      new   cargo-generate-rpm assets/metadata
docs/LINUX-TESTING.md               new   VM validation plan
docs/MACOS-NATIVE.md                new   plan to drop OrbStack
docs/LINUX.md                       edit  packages first, ModemManager note
docs/FLASHING.md                    edit  unsigned-binary run instructions
docs/RELEASING.md                   edit  shrink to checklist → points here
README.md                           edit  download-first quickstart
ROADMAP.md / CHANGELOG.md           edit  Phase 4 progress, v1.2.0
```

---

## 11. What actually happened when this plan was executed (2026‑09‑05)

Two bugs found only by running the tools for real, not by reasoning about the plan:

1. **`libc::ioctl`'s request-type mismatch, glibc vs. musl.** `serialtransport.rs`'s `TIOCMBIS`
   constant is declared `libc::c_ulong` for both platforms, which matches what `ioctl` wants on
   glibc — but `cargo zigbuild --target x86_64-unknown-linux-musl` failed to compile with a type
   error, because musl's `libc::ioctl` signature takes `c_int` for the request parameter instead.
   This had never been caught because every prior Linux build (including the real hardware
   validation on the Debian 13 VM) compiled natively against **glibc**, never musl. Fixed with
   `tio::TIOCMBIS as _` at the call site, which lets the cast target whichever type the platform
   actually declares.
2. **`cargo-generate-rpm` v0.21.0's real schema differs from what was drafted.**
   `post_install_script` must be a plain string (optionally a path to a script file), not a
   `{ program, script }` table — the manifest had the latter, guessed by analogy with other
   packaging formats, and the tool rejected it outright (`must be string`). Separately,
   `requires = {}` does **not** stop the tool from shelling out to `ldd` for auto-detected
   dependencies by default — on a host with no `ldd` (macOS, cross-building), this failed
   completely regardless of the manifest. Fixed with `auto-req = "disabled"`.

Both are exactly the kind of thing "we drafted this without running it" plans are supposed to
surface eventually — better now, from a maintainer's build, than from a user's failed install.

**Artifacts produced** (local only, not published): macOS universal binary (ad-hoc signed) and
tarball; `x86_64`/`aarch64` `unknown-linux-musl` static binaries and tarballs; `.deb` (from the
x86_64 musl binary); `.rpm` (same); `SHA256SUMS` + `SHA256SUMS.minisig`. All checksums and the
minisign signature verify. Not yet installed on a live `.deb`/`.rpm`-capable host — no such host
was available at build time, and cross-verifying package *contents* (done: binary path, udev
rule path, docs, maintainer script) is not the same as a live `apt install`/`dnf install`.

**Signing key custody.** A real minisign keypair was generated for this — not a placeholder. The
public key is committed at `packaging/ktflash.pub` and mirrored into `scripts/install.sh`. The
**secret key is encrypted at rest with a randomly generated passphrase** (never an empty
passphrase — an empty-password minisign key is only nominally protected) and lives outside this
repository. **This needs to move to durable secure storage (a password manager or hardware key)
before any real release** — right now both the encrypted key file and its passphrase exist as
plain files on the machine that generated them, which is fine for the local verification done
here but is not real offline custody.
