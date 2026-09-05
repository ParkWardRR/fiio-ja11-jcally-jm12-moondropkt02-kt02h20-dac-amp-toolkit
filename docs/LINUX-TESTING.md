# Linux testing plan — VM validation for the Linux release

**Status:** ✅ **P0–P3 passed on Debian 13** (2026‑09‑05, real hardware, full narrative in
[ROADMAP Appendix D](../ROADMAP.md#20260905--linuxnative-validation-session-narrative-veloce-hyperv-lab)) ·
AlmaLinux 10.2 blocked at the test‑rig level (§2.3) · **Companion to:** [`RELEASE-PLAN.md`](RELEASE-PLAN.md)
**Gates:** ROADMAP Phase 4.1 ("Test `flash-cdc` on real Debian/Arch, no OrbStack") and the
`native write` column of the README support table.

This is the plan for validating `ktflash` on real Linux hosts once a VM host is available. It is
written so that **most of it can run with no dongle at all** — only phases P1–P3 need hardware,
and they are deliberately last.

---

## 1. VM inventory

Six guests. All x86_64 unless the host can also run arm64 guests, in which case add the two marked
*(optional)*.

| # | Guest | Why this one |
|---|---|---|
| V1 | **Debian 13** (trixie) | Newest Debian; current glibc, current udev/ModemManager |
| V2 | **Debian 12** (bookworm) | The long-tail stable most users are on; glibc 2.36 |
| V3 | **AlmaLinux 10** | Newest EL; validates the `.rpm` and SELinux defaults |
| V4 | **AlmaLinux 9** | Oldest glibc in the support set (2.34) — the glibc-fallback build host (`RELEASE-PLAN.md` §2) |
| V5 | **Ubuntu 22.04 LTS** | Widest real-world install base; cheap extra coverage |
| V6 | **Arch** *(optional)* | Rolling; catches "newest everything" breakage. ROADMAP names it |
| V7 | **Debian 12 arm64** *(optional)* | Validates the aarch64 musl artifact |
| V8 | **AlmaLinux 9 arm64** *(optional)* | Validates the aarch64 `.rpm` |

**Provisioning per guest** (minimal — the whole point of the static build is that guests need
nothing):

```sh
# nothing required to RUN the musl artifact. For the source-build control case only:
# Debian/Ubuntu
sudo apt-get install -y build-essential libusb-1.0-0-dev pkg-config usbutils
# Alma/RHEL
sudo dnf install -y gcc libusb1-devel pkgconf-pkg-config usbutils
```

Record for each guest, into the results table: `uname -r`, `ldd --version`,
`systemctl is-active ModemManager`, `getenforce` (EL only), `udevadm --version`, desktop vs.
headless (affects whether `uaccess` grants a seat-local session).

---

## 2. Getting the dongle into a VM — read this first

This is the part most likely to waste a day.

### 2.1 The re-enumeration trap

`ktflash unlock` reboots the dongle into its CDC bootloader, where it comes back with a **different
VID:PID** (`31b2:xxxx` or `2972:0102` → `8888:cdc0`). To the hypervisor this is a *device removal
followed by a new device arrival*.

**⇒ Any USB passthrough rule keyed on VID:PID will drop the dongle at the exact moment it
matters** — `unlock` appears to succeed, then the guest never sees the bootloader and `flash-cdc`
reports "bootloader not present".

**Configure passthrough by physical bus/port path, not by device ID:**

```sh
# QEMU/libvirt — bind the PORT, so any device on it passes through
-device usb-host,hostbus=1,hostport=4
```

```xml
<!-- libvirt equivalent -->
<hostdev mode='subsystem' type='usb'>
  <source><address bus='1' device='0'/></source>  <!-- prefer <address> on the port, not <vendor>/<product> -->
</hostdev>
```

VirtualBox USB filters and VMware `usb.autoConnect` rules have the same failure mode — set them to
match the port, or configure **two** filters (one for the runtime VID, one for `8888:cdc0`) so the
re-enumerated device is re-captured. The two-filter approach is the reliable fallback when
port-based binding isn't available.

**Verify the trap is defused before P2:** in the guest, run `lsusb -t` before and after an
`unlock`, and confirm the device is still present as `8888:cdc0`.

### 2.2 Where the dongle physically lives

| Option | Setup | Notes |
|---|---|---|
| **A — plugged into the VM host** (preferred) | Hypervisor USB passthrough per §2.1 | Simplest; one cable; no network in the loop |
| **B — plugged into the Windows 11 machine** | `usbipd-win` exports it; guests run `usbip attach` | Uses the existing hardware agent's box. Re-enumeration means **re-running `usbip attach` after `unlock`** — script it |
| **C — plugged into the Mac** | ❌ no usable `usbip` client on macOS | Not viable; this is what OrbStack exists for today |

If Option B is used, note that the USB/IP round-trip adds latency to every 1 KB data packet in
`flash-cdc`. Bump timeouts before concluding a flash failed for protocol reasons. **Update:** in
practice, on a Hyper‑V host + guest on the same internal switch, the added latency never caused a
timeout — a full 67‑packet flash completed on the first attempt against the default timeouts. The
warning stands for anything with more network in between (e.g. a genuinely remote host).

### 2.3 Confirmed recipe — Option B via `usbipd-win` (Hyper-V host, Windows 10/11)

Full narrative in [ROADMAP Appendix D](../ROADMAP.md#20260905--linuxnative-validation-session-narrative-veloce-hyperv-lab).
Condensed steps, host side (PowerShell, admin):

```powershell
winget install dorssel.usbipd-win --accept-package-agreements --accept-source-agreements
usbipd list                        # find the dongle's BUSID
usbipd bind --busid <BUSID> --force   # --force needed if a capture filter (e.g. USBPcap) is present
# after every `ktflash unlock`, the dongle re-enumerates under a NEW VID:PID at the SAME busid:
usbipd bind --busid <BUSID> --force   # re-run this, then re-attach on the guest
```

Guest side (Debian/Ubuntu):

```sh
sudo apt-get install -y usbip        # ships the client; Alma/RHEL does NOT (see below)
sudo modprobe vhci-hcd
sudo usbip attach -r <host-internal-switch-ip> -b <BUSID>
sudo usbip port                      # confirm it's attached; re-run attach after every rebind
```

**AlmaLinux / RHEL cannot do this at all**, confirmed by inspection, not just a failed install:
`kernel-devel`'s `drivers/usb/usbip/` ships only `Kconfig`/`Makefile` — Red Hat removes the
driver's actual source — and `vhci-hcd` is absent from `kernel-modules-extra` too. This blocks
*this specific test rig* (dongle reached over the network via `usbipd-win`); it says nothing
about running `ktflash` on a RHEL box with the dongle plugged in directly, which has no USB/IP
dependency at all. If a RHEL/Alma result is needed, get the dongle onto real RHEL hardware (or a
hypervisor with real USB passthrough) rather than fighting this path further.

**Windows quirk hit along the way:** if a flash needs to be retried after the bootloader's
one‑shot handshake is already consumed, the *only* fix is a genuine power cycle. `Disable-
PnpDevice` (`HRESULT 0x80041001`) and `pnputil /disable-device` ("not supported on this OS
product") both failed on Windows 10 Pro — apparently a client‑SKU/WMI limitation, not something
fixable in the request. A physical unplug/replug always worked and preserved the usbipd share
(same busid, no need to re-bind from scratch).

---

## 3. Test phases

Phases are ordered by risk. **Do not start P3 without a known-good firmware image in hand** —
there is no readback ([`CDC-PROTOCOL.md`](CDC-PROTOCOL.md)).

> Scripts for the phases below are in [`scripts/testing/`](../scripts/testing/) —
> `collect-host-facts.sh`, `p0-smoke.sh`, `p2-usb-checks.sh` — with a results template at
> [`docs/results-template.md`](results-template.md). They parse cleanly but have never been run
> on a Linux guest.

### P0 — hardware-free (every guest, no dongle)

Certifies that the artifact *runs* on the distro. This is the bulk of the matrix and can be done
the day the VMs exist.

```sh
# --- install (one per artifact type) ---
tar xzf ktflash-<ver>-x86_64-unknown-linux-musl.tar.gz && ./ktflash --help
sudo apt install ./ktflash_<ver>_amd64.deb        # Debian/Ubuntu
sudo dnf install ./ktflash-<ver>.x86_64.rpm       # Alma

# --- staticness / portability ---
file $(command -v ktflash)          # expect: "statically linked"
ldd  $(command -v ktflash)          # expect: "not a dynamic executable"
objdump -T $(command -v ktflash) | grep GLIBC_ | sort -u   # glibc fallback build only

# --- packaging did its job ---
ls -l /usr/lib/udev/rules.d/99-ktflash.rules
udevadm control --reload-rules && udevadm trigger    # should be a no-op if postinst ran

# --- functional, no hardware ---
ktflash --help
ktflash bootdiag --replay fixtures/synthetic-success.json
ktflash compat --template | ktflash compat --validate /dev/stdin
ktflash image <fw.bin>                     # header parse, if an image is available
ktflash flash-cdc --image <fw.bin>         # DRY RUN — prints the packet plan, writes nothing
ktflash                                    # TUI renders and exits cleanly (needs a tty)
```

**P0 pass criteria:** install clean, binary static, rules present, all commands exit 0, TUI draws
without panics and restores the terminal on quit.

### P1 — read-only USB (needs the dongle)

```sh
lsusb | grep -iE '31b2|2972|8888'
ktflash probe                # as a normal user — proves the udev rules work
ktflash fingerprint          # structured JSON identity
```

**P1 pass criteria:** `probe` succeeds **without sudo**. If it needs sudo, the udev rules or seat
assignment is wrong — record which (headless guests have no seat, so `uaccess` may not apply;
that's the case where the `plugdev` fallback in the rules file matters, and it should be tested
explicitly on at least one headless guest).

### P2 — unlock + bootloader (recoverable, needs the dongle)

`unlock` is recoverable — a power-cycle returns the device to normal mode — but it is the first
step that changes device state, so it gets its own phase.

```sh
ktflash unlock
lsusb | grep 8888:cdc0       # bootloader present?
ls -l /dev/ttyACM*           # the CDC tty; note its owner/permissions
ktflash bootdiag             # proves the bulk pipe is live
# recovery check:
# unplug/replug → device returns as its normal-mode VID:PID
```

**Also check here (the ModemManager question from [`RELEASE-PLAN.md`](RELEASE-PLAN.md) §5.3):**

```sh
systemctl is-active ModemManager
journalctl -u ModemManager --since '2 min ago'   # did it probe /dev/ttyACM*?
udevadm info -q property -n /dev/ttyACM0 | grep -i ID_MM   # is ID_MM_DEVICE_IGNORE set?
```

Run P2 **twice** on at least one guest: once with the ModemManager udev fix installed and once
without, to confirm whether the fix is genuinely needed or merely defensive. Record the answer —
it decides whether the rule ships as a fix or a comment.

### P3 — native flash (destructive, last)

**The gate for ROADMAP Phase 4.1.** Preconditions, all mandatory:

- A known-good firmware image for *this specific dongle*, saved off-device.
- A dongle you are willing to lose.
- P2 passed on this guest, and passthrough survives re-enumeration (§2.1).
- `unlock` immediately before `--execute` — the bootloader state machine is one-shot.

```sh
ktflash flash-cdc --image fw.bin                      # dry run again, on this host
ktflash unlock
ktflash flash-cdc --image fw.bin --execute --yes
# then: replug, confirm the device enumerates in normal mode, and play audio
```

**P3 pass criteria:** flash completes, device re-enumerates in normal mode, **audio plays**. Then
capture before/after USB descriptors and file them per ROADMAP Phase 6.

**If it fails:** the operation journal (`ktflash-<op>.journal.json`) is the recovery input —
`ktflash recover <journal>` prints the safe next step. Attach the journal to the report.

---

## 4. Results matrix

Fill in as phases complete. Nothing ships labeled "verified" without its row.

| Guest | Arch | Artifact | P0 | P1 probe | P2 unlock/bootdiag | P3 flash-cdc |
|---|---|---|:--:|:--:|:--:|:--:|
| V1 Debian 13 | x86_64 | `cargo build` (source control) | ✅ | ✅ (needs `plugdev`, see Phase 4 finding) | ✅ (both transports; serial + libusb) | ✅ **full write, 67/67 packets, 2026‑09‑05** |
| V2 Debian 12 | x86_64 | `.deb` | ⏳ | ⏳ | ⏳ | ⏳ |
| V3 AlmaLinux 10 | x86_64 | `.rpm` | ✅ (build/deps only) | ➖ **not pursuing**: RHEL kernel excludes `vhci-hcd`; this rig can't attach the dongle at all (§2.3) — the `.rpm` still ships, untested on real Alma hardware | ➖ not pursuing | ➖ not pursuing |
| V4 AlmaLinux 9 | x86_64 | `.rpm` | ⏳ | ⏳ | ⏳ | ⏳ |
| V5 Ubuntu 22.04 | x86_64 | musl tarball | ⏳ | ⏳ | ⏳ | ⏳ |
| V6 Arch *(opt)* | x86_64 | musl tarball | ⏳ | ⏳ | ⏳ | ⏳ |
| V7 Debian 12 *(opt)* | aarch64 | musl tarball | ⏳ | ⏳ | ⏳ | ⏳ |
| V8 AlmaLinux 9 *(opt)* | aarch64 | `.rpm` | ⏳ | ⏳ | ⏳ | ⏳ |
| — source build control | x86_64 | `cargo build` | ✅ (same run as V1) | ✅ | ✅ | ✅ |

**V1 detail (2026‑09‑05, Debian 13 on Hyper‑V, dongle via `usbipd-win` from the Windows host):**
93 unit tests + clippy clean before hardware; `probe`/`fingerprint` matched pre‑ and post‑flash
descriptor SHA‑256 exactly; `bootdiag` (non‑advancing) and `bootdiag --send` (KTM handshake) both
confirmed the serial transport live over `/dev/ttyACM0` — the first hardware run of that code
path; a `flash-cdc --execute` attempted right after `--send` failed safely (`KTM` got zero bytes
back, journal correctly said `CancelOrBegin`, nothing erased) because the one‑shot handshake was
already spent — recovered with a physical power‑cycle, then a clean `unlock` → `flash-cdc
--execute --yes` completed end‑to‑end on the first real attempt. Full narrative:
[ROADMAP Appendix D](../ROADMAP.md#20260905--linuxnative-validation-session-narrative-veloce-hyperv-lab).

**One dongle, many guests:** P1–P3 only need to pass on a representative subset — one Debian and
one Alma is enough to call the Linux path proven. P0 should pass everywhere. Plan the passthrough
so the dongle can be moved between guests without re-cabling.

---

## 5. Open questions for the VM host

1. Hypervisor — **Proxmox / libvirt-KVM / VMware / VirtualBox**? Decides the §2.1 passthrough
   syntax.
2. Can it run **arm64 guests** (V7/V8)? Decides whether v1.2.0 ships aarch64 Linux artifacts.
3. Will the dongle be plugged into the **VM host** (Option A) or exported from the **Windows 11
   box** via `usbipd-win` (Option B)?
4. Are guests **headless or desktop**? Headless has no logind seat, which changes whether udev
   `uaccess` grants access — worth having at least one of each.
5. Is there a **sacrificial dongle** for P3, and a known-good image for it?
