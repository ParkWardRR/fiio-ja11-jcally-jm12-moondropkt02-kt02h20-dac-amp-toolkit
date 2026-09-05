// vmatrix — run the ktflash Linux test matrix across VM guests over SSH.
//
//	vmatrix init                                 write a starter hosts.json
//	vmatrix facts    -hosts hosts.json           collect per-guest facts
//	vmatrix deploy   -hosts hosts.json -dist DIR copy the right artifacts to each guest
//	vmatrix install  -hosts hosts.json           install them (deb/rpm/tarball as configured)
//	vmatrix p0       -hosts hosts.json           hardware-free phase, every guest
//	vmatrix p1       -hosts hosts.json           read-only USB, guests with the dongle
//	vmatrix run      -hosts hosts.json -dist DIR everything above, in order
//	vmatrix report   -hosts hosts.json -out F.md render results (implied by `run`)
//
// WHAT IT DOES NOT DO, deliberately: P2 (unlock) and P3 (flash). P2 changes device state and
// P3 destroys firmware with no backup path, so neither should ever run because someone typed a
// command that looked like it only ran tests. They stay manual —
// staging/linux-testing/p2-usb-checks.sh.
//
// STATUS: UNTESTED against real guests. Compiles and `go vet`s clean; never run over SSH.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"
)

const usage = `vmatrix — ktflash Linux test matrix runner

USAGE:
  vmatrix <command> [flags]

COMMANDS:
  init      write a starter hosts.json
  facts     collect per-guest facts (distro, glibc, ModemManager, SELinux, seat)
  deploy    copy each guest's artifact to it
  install   install the deployed artifact
  p0        hardware-free checks on every guest
  p1        read-only USB checks on guests with the dongle
  run       facts -> deploy -> install -> p0 -> p1 -> report
  report    render the results markdown from the last run in this process

FLAGS:
  -hosts FILE     host config (default hosts.json)
  -dist DIR       directory of release artifacts (for deploy)
  -out FILE       where to write the report (default linux-test-results.md)
  -parallel N     max concurrent guests (default 4)
  -timeout D      per-command timeout (default 5m)
  -version V      version label for the report

P2 (unlock) and P3 (flash) are intentionally not automated. See the package comment.
`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usage)
		os.Exit(2)
	}
	cmd := os.Args[1]

	fs := flag.NewFlagSet(cmd, flag.ExitOnError)
	hostsPath := fs.String("hosts", "hosts.json", "host config file")
	distDir := fs.String("dist", "", "directory of release artifacts")
	outPath := fs.String("out", "linux-test-results.md", "report output path")
	parallel := fs.Int("parallel", 4, "max concurrent guests")
	timeout := fs.Duration("timeout", 5*time.Minute, "per-command timeout")
	version := fs.String("version", "unknown", "version label for the report")
	fs.Usage = func() { fmt.Fprint(os.Stderr, usage) }
	_ = fs.Parse(os.Args[2:])

	if cmd == "init" {
		if _, err := os.Stat(*hostsPath); err == nil {
			fatalf("%s already exists — refusing to overwrite", *hostsPath)
		}
		if err := os.WriteFile(*hostsPath, []byte(exampleConfig), 0o644); err != nil {
			fatalf("write %s: %v", *hostsPath, err)
		}
		fmt.Printf("wrote %s — edit the ssh targets, then: vmatrix facts -hosts %s\n",
			*hostsPath, *hostsPath)
		return
	}

	cfg, err := loadConfig(*hostsPath)
	if err != nil {
		fatalf("%v", err)
	}
	hosts := cfg.active()
	if len(hosts) == 0 {
		fatalf("every host in %s is skipped", *hostsPath)
	}

	// Ctrl-C must actually stop the fan-out, not leave ssh children running.
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()

	report := NewReport(*version)

	runPhase := func(name string, sel func(Host) bool, body func(Host) Result) {
		var chosen []Host
		for _, h := range hosts {
			if sel == nil || sel(h) {
				chosen = append(chosen, h)
			}
		}
		if len(chosen) == 0 {
			fmt.Printf("\n== %s == (no eligible guests)\n", name)
			return
		}
		fmt.Printf("\n== %s == (%d guests, %d at a time)\n", name, len(chosen), *parallel)
		results := fanOut(chosen, *parallel, body)
		for _, res := range results {
			report.Add(name, res)
			status := "ok"
			if !res.OK() {
				status = "FAIL"
				if res.Err != nil {
					status = "ERROR: " + res.Err.Error()
				}
			}
			fmt.Printf("  %-24s %-6s %6.1fs\n", res.Host.Name, status, res.Duration.Seconds())
			if !res.OK() {
				for _, line := range strings.Split(strings.TrimSpace(res.Stderr), "\n") {
					if line != "" {
						fmt.Printf("      %s\n", line)
					}
				}
			}
		}
	}

	withTimeout := func(fn func(context.Context, Host) Result) func(Host) Result {
		return func(h Host) Result {
			c, cancel := context.WithTimeout(ctx, *timeout)
			defer cancel()
			return fn(c, h)
		}
	}

	doFacts := func() {
		runPhase("facts", nil, withTimeout(func(c context.Context, h Host) Result {
			res := runSSH(c, cfg, h, factsScript)
			if res.OK() {
				report.AddFacts(h.Name, parseFacts(res.Stdout))
			}
			return res
		}))
	}

	doDeploy := func() {
		if *distDir == "" {
			fatalf("-dist DIR is required for deploy")
		}
		runPhase("deploy", func(h Host) bool { return h.Artifact != ArtifactSource },
			withTimeout(func(c context.Context, h Host) Result {
				files, err := artifactsFor(*distDir, h)
				if err != nil {
					return Result{Host: h, Err: err}
				}
				mk := runSSH(c, cfg, h, fmt.Sprintf("mkdir -p %q", cfg.remoteDir()))
				if !mk.OK() {
					return mk
				}
				for _, f := range files {
					if r := copyFile(c, cfg, h, f, cfg.remoteDir()); !r.OK() {
						return r
					}
				}
				return Result{Host: h, Stdout: fmt.Sprintf("copied %d files", len(files))}
			}))
	}

	doInstall := func() {
		runPhase("install", nil, withTimeout(func(c context.Context, h Host) Result {
			return runSSH(c, cfg, h, installScript(cfg, h))
		}))
	}

	doP0 := func() {
		runPhase("p0", nil, withTimeout(func(c context.Context, h Host) Result {
			return runSSH(c, cfg, h, p0Script(cfg))
		}))
	}

	doP1 := func() {
		// Only guests that can actually see the dongle. Running P1 elsewhere would produce
		// failures that mean nothing and drown the ones that do.
		runPhase("p1", func(h Host) bool { return h.HasDongle },
			withTimeout(func(c context.Context, h Host) Result {
				return runSSH(c, cfg, h, p1Script)
			}))
	}

	writeReport := func() {
		md := report.Markdown(hosts)
		if err := os.WriteFile(*outPath, []byte(md), 0o644); err != nil {
			fatalf("write %s: %v", *outPath, err)
		}
		fmt.Printf("\nwrote %s\n", *outPath)
		fmt.Println("Remember: P2 (unlock) and P3 (flash) are manual — see LINUX-TESTING.md §3.")
	}

	switch cmd {
	case "facts":
		doFacts()
		writeReport()
	case "deploy":
		doDeploy()
	case "install":
		doInstall()
	case "p0":
		doP0()
		writeReport()
	case "p1":
		doP1()
		writeReport()
	case "report":
		writeReport()
	case "run":
		doFacts()
		doDeploy()
		doInstall()
		doFacts() // again: linkage/version are only knowable once ktflash is installed
		doP0()
		doP1()
		writeReport()
	default:
		fmt.Fprintf(os.Stderr, "unknown command %q\n\n%s", cmd, usage)
		os.Exit(2)
	}
}

func fatalf(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "vmatrix: "+format+"\n", args...)
	os.Exit(1)
}
