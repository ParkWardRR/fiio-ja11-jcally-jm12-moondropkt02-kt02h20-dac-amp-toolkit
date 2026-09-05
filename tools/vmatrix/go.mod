// vmatrix — run the ktflash Linux test matrix across VM guests over SSH.
//
// DEPENDENCY-FREE ON PURPOSE. Standard library only: no golang.org/x/crypto/ssh, no TOML
// parser, nothing vendored. It shells out to the system `ssh`/`scp`, which means it inherits
// the operator's existing SSH config, agent, jump hosts and known_hosts rather than
// reimplementing any of that — and `go build` works offline with zero third-party code in a
// repo that cares about supply chain (docs/RELEASE-PLAN.md §0).

module github.com/ParkWardRR/fiio-ja11-jcally-jm12-moondropkt02-kt02h20-dac-amp-toolkit/tools/vmatrix

go 1.22
