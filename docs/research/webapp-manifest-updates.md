# Updating installed web apps from their manifests

**Date:** 2026-11-20

`app update` fetches the recorded manifest and previews its changes. `--apply` verifies the
installed identity, updates values that still equal the last import, and refreshes the launcher.
A local setting or locally edited icon stays in place. If retained scope and proposed start
page are incompatible, both stay as they were.

New installs record the source and imported values in an internal file beside `app.yaml`.
Older installs can discover the manifest from their start page. Their first apply records the
baseline with the local configuration intact. Fetching is unauthenticated, as for `app add`.

A new icon is written under a content-based filename before `app.yaml` refers to it. The updater
stages the next baseline against the digest of the proposed configuration. An interrupted write
can therefore use the baseline that matches the configuration on disk. A second apply refreshes
the launcher if a previous launcher write failed. Top-level YAML edits are checked against the
parsed document before a write.

On the development machine, a temporary loopback site exercised CLI installation, preview
without writes, apply, launcher refresh, repeated apply, preservation of local edits, enrollment
of an older install and refusal of a changed manifest ID. The GNOME Wayland `install` self-test
checked that installation from a tab saves the manifest URL and identity. Authenticated manifests
and live websites were outside this check.

```bash
scripts/test unit webapp::update::tests
scripts/test unit manifest::tests
scripts/test selftest install
```
