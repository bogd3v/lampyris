# Security policy

Lampyris is a tool against manipulation campaigns, so it may itself be targeted:
poisoned inputs, malicious releases, or attempts to make it accuse innocent
accounts.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private vulnerability
reporting ("Security" tab → "Report a vulnerability") on this repository.
Reports may be pseudonymous.

Useful things to include: affected version or commit, steps to reproduce, and
impact (crash, wrong detections, data exposure, supply-chain risk…).

## In scope

- Crashes or resource exhaustion from crafted network input.
- Ways to make the detector systematically flag unrelated accounts.
- Exposure of local data beyond what [docs/METHODOLOGY.md](docs/METHODOLOGY.md)
  describes.
- Build, CI or release supply-chain issues.

## Releases

Release binaries will be built by CI from tagged commits and published with
SHA-256 checksums and a signature. Verify them before running.
