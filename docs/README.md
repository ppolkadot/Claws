# Documentation

Use these documents for the current project:

- [Development status](CURRENT_DEVELOPMENT_STATUS.md): packaged version, remaining work and verification limits.
- [Developer handoff](HANDOFF.md): architecture, safety contracts, build/test commands and manual acceptance.
- [User guide](../README.md): installation, features, settings and troubleshooting.
- [Product brief](../PRODUCT.md) and [design system](../DESIGN.md): product scope and visual rules.
- [1.0.9 release notes](releases/1.0.9.md): installer, checksum, build source and packaging evidence (not yet published).
- [1.0.8 release notes](releases/1.0.8.md): installer, checksum, build source and packaging evidence.
- [Spotify verification](spotify-track-matching-verification.md): feature contract, current checks and dated live evidence.
- [Speed diagnosis](performance/speed-diagnosis.md) and [UI measurements](performance/queue-ui.md): measured results with raw data under `performance/`.

## Historical references

`releases/` preserves per-version artifact records. Earlier versions, including the historical 1.1.0 candidate, are not the current build.

The dated reliability/review reports and `superpowers/plans/` preserve implementation decisions and original reproductions. Their old version numbers, test counts and unchecked steps describe the recorded snapshot, not current defects or instructions to redo completed work. Use the current status and owning source before acting on them. The Spotify design remains under `superpowers/specs/`.

Keep current state in the status page, reusable engineering guidance in the handoff, and immutable build evidence in release records. Do not append session transcripts, duplicate test histories or superseded release summaries to the handoff.
