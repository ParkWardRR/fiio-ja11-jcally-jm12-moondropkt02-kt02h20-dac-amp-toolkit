package main

import (
	"bytes"
	"context"
	"fmt"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"
)

// Result is one command run against one host.
type Result struct {
	Host     Host
	Phase    string
	Stdout   string
	Stderr   string
	ExitCode int
	Err      error
	Duration time.Duration
}

func (r Result) OK() bool { return r.Err == nil && r.ExitCode == 0 }

// Mark renders the results-table cell.
func (r Result) Mark() string {
	switch {
	case r.Err != nil:
		return "❌"
	case r.ExitCode == 0:
		return "✅"
	default:
		return "❌"
	}
}

// runSSH executes a command on a host via the system ssh.
//
// Shelling out rather than using x/crypto/ssh is deliberate: the operator's ~/.ssh/config,
// agent, jump hosts and known_hosts all just work, and there is no third-party crypto in a
// repo that ships a tool for erasing firmware.
func runSSH(ctx context.Context, cfg *Config, h Host, script string) Result {
	start := time.Now()
	args := append([]string{}, cfg.SSHOptions...)
	args = append(args, h.SSH, "bash -s")

	cmd := exec.CommandContext(ctx, "ssh", args...)
	cmd.Stdin = strings.NewReader(script)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err := cmd.Run()
	res := Result{
		Host: h, Stdout: stdout.String(), Stderr: stderr.String(),
		Duration: time.Since(start),
	}
	var exitErr *exec.ExitError
	if err != nil {
		if ok := asExitError(err, &exitErr); ok {
			res.ExitCode = exitErr.ExitCode()
		} else {
			res.Err = err
		}
	}
	return res
}

// asExitError is errors.As specialised, kept tiny to avoid an import for one call.
func asExitError(err error, target **exec.ExitError) bool {
	for err != nil {
		if e, ok := err.(*exec.ExitError); ok {
			*target = e
			return true
		}
		u, ok := err.(interface{ Unwrap() error })
		if !ok {
			return false
		}
		err = u.Unwrap()
	}
	return false
}

// copyFile pushes a local file to a host with scp.
func copyFile(ctx context.Context, cfg *Config, h Host, local, remoteDir string) Result {
	start := time.Now()
	args := append([]string{}, cfg.SSHOptions...)
	args = append(args, local, h.SSH+":"+remoteDir+"/")

	cmd := exec.CommandContext(ctx, "scp", args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := cmd.Run()

	res := Result{
		Host: h, Phase: "deploy", Stdout: stdout.String(), Stderr: stderr.String(),
		Duration: time.Since(start),
	}
	var exitErr *exec.ExitError
	if err != nil {
		if asExitError(err, &exitErr) {
			res.ExitCode = exitErr.ExitCode()
		} else {
			res.Err = err
		}
	}
	return res
}

// fanOut runs body against every host, at most `parallel` at a time.
//
// Guests are independent, and a serial run across eight VMs wastes most of an afternoon —
// but unbounded parallelism against one hypervisor host is how you make it swap.
func fanOut(hosts []Host, parallel int, body func(Host) Result) []Result {
	if parallel < 1 {
		parallel = 1
	}
	sem := make(chan struct{}, parallel)
	out := make([]Result, len(hosts))
	var wg sync.WaitGroup

	for i, h := range hosts {
		wg.Add(1)
		go func(i int, h Host) {
			defer wg.Done()
			sem <- struct{}{}
			defer func() { <-sem }()
			out[i] = body(h)
		}(i, h)
	}
	wg.Wait()
	return out
}

// ---------------------------------------------------------------- scripts

// factsScript mirrors staging/linux-testing/collect-host-facts.sh, trimmed to the fields the
// report needs and made parseable (key=value) rather than pretty.
const factsScript = `
set -u
emit(){ printf '%s=%s\n' "$1" "$2"; }
. /etc/os-release 2>/dev/null || true
emit distro "${PRETTY_NAME:-unknown}"
emit id "${ID:-unknown}"
emit version_id "${VERSION_ID:-unknown}"
emit arch "$(uname -m)"
emit kernel "$(uname -r)"
emit glibc "$(ldd --version 2>/dev/null | head -n1 | sed 's/.*) //' || echo unknown)"
emit modemmanager "$(systemctl is-active ModemManager 2>/dev/null || echo absent)"
emit selinux "$(getenforce 2>/dev/null || echo absent)"
emit seat "$(loginctl show-session "$(loginctl 2>/dev/null | awk 'NR==2{print $1}')" -p Seat --value 2>/dev/null || echo none)"
emit ktflash "$(command -v ktflash 2>/dev/null || echo none)"
if command -v ktflash >/dev/null 2>&1; then
  emit linkage "$(file -L "$(command -v ktflash)" 2>/dev/null | grep -o 'statically linked\|dynamically linked' || echo unknown)"
  emit version "$(ktflash --help 2>&1 | head -n1)"
fi
emit rules "$(ls /etc/udev/rules.d/99-ktflash.rules /usr/lib/udev/rules.d/99-ktflash.rules 2>/dev/null | head -n1 || echo none)"
emit mm_ignore "$(grep -l ID_MM_DEVICE_IGNORE /etc/udev/rules.d/99-ktflash.rules /usr/lib/udev/rules.d/99-ktflash.rules 2>/dev/null | head -n1 || echo no)"
emit dongle "$(lsusb 2>/dev/null | grep -icE '31b2|2972|8888' || echo 0)"
`

// installScript installs whichever artifact this guest is meant to exercise.
//
// It deliberately does NOT use sudo for the tarball path: proving ktflash works unprivileged is
// half the point of the udev rules.
func installScript(cfg *Config, h Host) string {
	dir := cfg.remoteDir()
	switch h.Artifact {
	case ArtifactDeb:
		return fmt.Sprintf(`set -eu
cd %q
deb=$(ls ktflash*.deb 2>/dev/null | head -n1)
[ -n "$deb" ] || { echo "no .deb deployed" >&2; exit 2; }
sudo apt-get install -y "./$deb"
command -v ktflash`, dir)
	case ArtifactRPM:
		return fmt.Sprintf(`set -eu
cd %q
rpm=$(ls ktflash*.rpm 2>/dev/null | head -n1)
[ -n "$rpm" ] || { echo "no .rpm deployed" >&2; exit 2; }
sudo dnf install -y "./$rpm"
command -v ktflash`, dir)
	case ArtifactSource:
		return fmt.Sprintf(`set -eu
cd %q
echo "source builds are set up by hand — see docs/LINUX.md" >&2
command -v ktflash`, dir)
	default: // tarball
		return fmt.Sprintf(`set -eu
cd %q
tar=$(ls ktflash-*.tar.gz 2>/dev/null | head -n1)
[ -n "$tar" ] || { echo "no tarball deployed" >&2; exit 2; }
rm -rf extracted && mkdir extracted && tar -xzf "$tar" -C extracted --strip-components=1
mkdir -p "$HOME/.local/bin"
install -m 0755 extracted/ktflash "$HOME/.local/bin/ktflash"
echo "installed to $HOME/.local/bin/ktflash (udev rules NOT installed — that is a separate, sudo step)"`, dir)
	}
}

// p0Script is the hardware-free phase from docs/LINUX-TESTING.md §3.
//
// Kept inline rather than uploading p0-smoke.sh so a guest needs nothing but bash — one less
// thing to have deployed correctly before the test can tell you anything.
func p0Script(cfg *Config) string {
	return fmt.Sprintf(`
set -u
export PATH="$HOME/.local/bin:$PATH"
cd %q 2>/dev/null || true
fail=0
kt="$(command -v ktflash || true)"
[ -n "$kt" ] || { echo "FAIL: ktflash not on PATH"; exit 1; }
echo "binary: $kt"

desc="$(file -L "$kt" 2>/dev/null || echo unknown)"
echo "file: $desc"
case "$desc" in
  *"statically linked"*) echo "PASS: statically linked" ;;
  *"dynamically linked"*)
      echo "NOTE: dynamically linked (expected only for the glibc fallback build)"
      objdump -T "$kt" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -3 | sed 's/^/  glibc floor: /' ;;
  *) echo "NOTE: could not determine linkage" ;;
esac

run(){ # run <label> <expect 0|1> <cmd...>
  label="$1"; want="$2"; shift 2
  if "$@" >/dev/null 2>&1; then rc=0; else rc=$?; fi
  if [ "$want" = 0 ] && [ "$rc" = 0 ]; then echo "PASS: $label"
  elif [ "$want" != 0 ] && [ "$rc" != 0 ]; then echo "PASS: $label (correctly refused)"
  else echo "FAIL: $label (exit $rc)"; fail=$((fail+1)); fi
}

run "--help" 0 "$kt" --help
run "compat --template" 0 "$kt" compat --template

tmp="$(mktemp)"
if "$kt" compat --template > "$tmp" 2>/dev/null; then
  run "compat --validate round-trip" 0 "$kt" compat --validate "$tmp"
fi
rm -f "$tmp"

rules="$(ls /etc/udev/rules.d/99-ktflash.rules /usr/lib/udev/rules.d/99-ktflash.rules 2>/dev/null | head -n1 || true)"
if [ -n "$rules" ]; then
  echo "PASS: udev rules at $rules"
  if grep -q ID_MM_DEVICE_IGNORE "$rules"; then
    echo "PASS: rules include the ModemManager ignore entries"
  else
    echo "FAIL: rules are the OLD version (no ID_MM_DEVICE_IGNORE)"; fail=$((fail+1))
  fi
else
  echo "NOTE: udev rules not installed (expected for a bare tarball install)"
fi

echo "---"
if [ "$fail" = 0 ]; then echo "P0 PASS"; else echo "P0 FAIL ($fail)"; fi
exit "$fail"
`, cfg.remoteDir())
}

// p1Script is the read-only USB phase. The important assertion is "without sudo".
const p1Script = `
set -u
export PATH="$HOME/.local/bin:$PATH"
fail=0
echo "lsusb:"; lsusb 2>/dev/null | grep -iE '31b2|2972|8888' || echo "  (no dongle visible)"

if ktflash probe >/dev/null 2>&1; then
  echo "PASS: probe works as a normal user"
else
  echo "FAIL: probe failed unprivileged"
  if sudo -n ktflash probe >/dev/null 2>&1; then
    echo "  ...but works under sudo => the udev rules or the logind seat is the problem"
  fi
  fail=$((fail+1))
fi

ktflash fingerprint >/dev/null 2>&1 && echo "PASS: fingerprint" || { echo "FAIL: fingerprint"; fail=$((fail+1)); }
exit "$fail"
`

var factsLine = regexp.MustCompile(`^([a-z_]+)=(.*)$`)

func parseFacts(stdout string) map[string]string {
	out := map[string]string{}
	for _, line := range strings.Split(stdout, "\n") {
		if m := factsLine.FindStringSubmatch(strings.TrimSpace(line)); m != nil {
			out[m[1]] = m[2]
		}
	}
	return out
}

// artifactsFor picks which local files belong on a given guest, so a Debian box does not have
// four RPMs copied to it.
func artifactsFor(dir string, h Host) ([]string, error) {
	var patterns []string
	switch h.Artifact {
	case ArtifactDeb:
		patterns = []string{"ktflash*.deb"}
	case ArtifactRPM:
		patterns = []string{"ktflash*.rpm"}
	case ArtifactSource:
		return nil, nil
	default:
		arch := h.Arch
		if arch == "" {
			arch = "x86_64"
		}
		// musl tarball for this architecture, plus the checksums so the guest can verify.
		patterns = []string{"ktflash-*-" + muslTarget(arch) + ".tar.gz"}
	}
	patterns = append(patterns, "SHA256SUMS", "SHA256SUMS.minisig")

	var files []string
	for _, p := range patterns {
		matches, err := filepath.Glob(filepath.Join(dir, p))
		if err != nil {
			return nil, err
		}
		files = append(files, matches...)
	}
	sort.Strings(files)
	if len(files) == 0 {
		return nil, fmt.Errorf("no artifacts in %s matching %v", dir, patterns)
	}
	return files, nil
}

func muslTarget(arch string) string {
	switch arch {
	case "aarch64", "arm64":
		return "aarch64-unknown-linux-musl"
	default:
		return "x86_64-unknown-linux-musl"
	}
}
