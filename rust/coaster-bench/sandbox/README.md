# Sandbox recipes

Recovered 2026-07-25 from the running sandboxes; the originals were lost. Each
policy was verified by creating a sandbox from it and diffing the effective
policy against the hand-made original: all three matched byte for byte.

| Lane | Image | Policy | Provider |
| --- | --- | --- | --- |
| Claude Code | `Dockerfile` → `localhost/coaster-sandbox` | `policy.yaml` | `claude-sub` |
| opencode / OpenRouter | `opencode.Dockerfile` → `localhost/coaster-or` | `opencode-policy.yaml` | `openrouter` |
| codex | `codex.Dockerfile` → `localhost/codex-arena` | `codex-policy.yaml` | `codex-oauth` |

```bash
podman build -t localhost/coaster-sandbox rust/coaster-bench/sandbox
podman build -t localhost/coaster-or -f rust/coaster-bench/sandbox/opencode.Dockerfile rust/coaster-bench/sandbox
podman build -t localhost/codex-arena -f rust/coaster-bench/sandbox/codex.Dockerfile rust/coaster-bench/sandbox
```

`coaster-bench --fresh-sandbox` creates and deletes a per-run sandbox from the
Claude Code recipe. The other two lanes are still long-lived and predate it.

## Notes

Providers inject their own network block (Anthropic, OpenRouter, OpenAI hosts).
Never copy those into a policy file: `--provider` regenerates them, and a
hand-copied version drifts. Only the game MCP endpoint is authored, plus the
codex exception below.

Attribution uses kernel-resolved binary paths, so a symlinked agent binary is
attributed to its target. `codex.Dockerfile` copies the binary for that reason.

The gateway caps sandbox names at 19 characters. `sandbox create` attaches and
never returns even with `--no-tty`, but the sandbox becomes ready during it and
outlives the client. `sandbox exec` stdin caps at 4 MiB, so use `sandbox upload`
for anything larger. Deletes are eventually consistent.

## Unverified

The images have not been rebuilt from these files yet; they were reconstructed
from `podman history` of the images in use, which is exact for the Claude Code
and opencode lanes.

The codex lane is the weak one. The base image ships codex 0.117 at
`/usr/bin/codex`, while the live `codex-arena` runs a 263 MB build at
`/home/sandbox/bin/codex` that was placed there by hand, leaving no record of
its origin (no tarball, and npm global holds only 0.117). `codex.Dockerfile`
pins `0.145.0-alpha.18` from npm and copies it to the same path, which matches
the policy but has not been built or run. Expect the copy step to need the
package's real binary layout.
