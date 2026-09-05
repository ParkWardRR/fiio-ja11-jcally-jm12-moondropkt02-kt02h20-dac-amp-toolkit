package main

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"
)

// Artifact kinds, matching what release.sh produces.
const (
	ArtifactTarball = "tarball"
	ArtifactDeb     = "deb"
	ArtifactRPM     = "rpm"
	ArtifactSource  = "source"
)

// Host is one VM guest from docs/LINUX-TESTING.md §1.
type Host struct {
	// Short label used in the results table, e.g. "V2 Debian 12".
	Name string `json:"name"`
	// Anything `ssh` accepts: "user@10.0.0.5", or an alias from ~/.ssh/config.
	SSH string `json:"ssh"`
	// Informational; recorded in the report. Auto-detected by `facts` if empty.
	Distro string `json:"distro,omitempty"`
	Arch   string `json:"arch,omitempty"`
	// Which artifact this guest is meant to exercise: tarball | deb | rpm | source.
	Artifact string `json:"artifact"`
	// Does the dongle reach this guest? Only these can run P1-P3.
	HasDongle bool `json:"has_dongle,omitempty"`
	// No logind seat, so udev `uaccess` may not apply and the plugdev fallback matters.
	Headless bool `json:"headless,omitempty"`
	// Skip without removing it from the file.
	Skip bool `json:"skip,omitempty"`
}

// Config is the whole matrix.
type Config struct {
	// Extra flags for every ssh invocation, e.g. ["-o","ConnectTimeout=10"].
	SSHOptions []string `json:"ssh_options,omitempty"`
	// Where to drop artifacts on each guest.
	RemoteDir string `json:"remote_dir,omitempty"`
	Hosts     []Host `json:"hosts"`
}

func (c *Config) remoteDir() string {
	if c.RemoteDir != "" {
		return c.RemoteDir
	}
	return "/tmp/ktflash-test"
}

// active returns the hosts that are not skipped.
func (c *Config) active() []Host {
	var out []Host
	for _, h := range c.Hosts {
		if !h.Skip {
			out = append(out, h)
		}
	}
	return out
}

func loadConfig(path string) (*Config, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var c Config
	dec := json.NewDecoder(strings.NewReader(string(raw)))
	// A typo in a host file should be an error, not a silently ignored guest.
	dec.DisallowUnknownFields()
	if err := dec.Decode(&c); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	if len(c.Hosts) == 0 {
		return nil, fmt.Errorf("%s: no hosts defined", path)
	}
	for i, h := range c.Hosts {
		if h.Name == "" || h.SSH == "" {
			return nil, fmt.Errorf("%s: host %d needs both \"name\" and \"ssh\"", path, i)
		}
		switch h.Artifact {
		case ArtifactTarball, ArtifactDeb, ArtifactRPM, ArtifactSource, "":
		default:
			return nil, fmt.Errorf("%s: host %q has unknown artifact %q (want tarball|deb|rpm|source)",
				path, h.Name, h.Artifact)
		}
	}
	return &c, nil
}

// exampleConfig is written by `vmatrix init`. It mirrors the guest list in
// docs/LINUX-TESTING.md §1 so the two stay recognisably the same matrix.
const exampleConfig = `{
  "remote_dir": "/tmp/ktflash-test",
  "ssh_options": ["-o", "ConnectTimeout=10", "-o", "BatchMode=yes"],
  "hosts": [
    { "name": "V1 Debian 13",    "ssh": "user@debian13",  "artifact": "tarball", "has_dongle": false },
    { "name": "V2 Debian 12",    "ssh": "user@debian12",  "artifact": "deb",     "has_dongle": true  },
    { "name": "V3 AlmaLinux 10", "ssh": "user@alma10",    "artifact": "rpm",     "has_dongle": false },
    { "name": "V4 AlmaLinux 9",  "ssh": "user@alma9",     "artifact": "rpm",     "has_dongle": true  },
    { "name": "V5 Ubuntu 22.04", "ssh": "user@ubuntu2204","artifact": "tarball", "has_dongle": false },
    { "name": "V6 Arch",         "ssh": "user@arch",      "artifact": "tarball", "skip": true },
    { "name": "V4b Alma 9 headless", "ssh": "user@alma9h", "artifact": "rpm", "headless": true, "has_dongle": true }
  ]
}
`
