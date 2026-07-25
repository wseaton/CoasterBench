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

## Verification status

Claude Code and opencode lanes: **rebuilt and verified** 2026-07-25. Building the
base with cache reproduced the in-use image id exactly (`fb083e1932da`), so the
recovered instructions match the original build. A `--no-cache` rebuild also
works, and with the pinned versions produces claude-code 2.1.218 and opencode
1.18.4, matching what produced the existing results. Unpinned it drifts (2.1.220,
1.18.5), which is why both are `ARG`s: bumping an agent version should be a
deliberate edit, not a side effect of rebuilding.

`python3` is installed for agents that want to compute track geometry before
placing it (observed: fable doing exactly that when run outside a sandbox). It
has no network at all, verified: DNS fails for anthropic and pypi, a raw
`1.1.1.1:53` connect times out, and even the game's MCP port is unreachable,
because network is attributed per binary and python3 is in no binaries list. It
is local compute only, and only usable when the run grants Bash.

Bound every `sandbox exec`: against a sandbox that is still coming up it blocks
instead of failing, so an unbounded readiness poll hangs forever.

`opencode --version` hangs inside a policy-restricted sandbox because it reaches
out on startup and a denied connection stalls instead of failing. It is fine in
the Dockerfile, where the build network is open, but never use it as a readiness
probe: use `sandbox exec -- true` instead.

### codex

The codex lane is the weak one and is still unbuilt. The base image ships codex 0.117 at
`/usr/bin/codex`, while the live `codex-arena` runs a 263 MB build at
`/home/sandbox/bin/codex` that was placed there by hand, leaving no record of
its origin (no tarball, and npm global holds only 0.117). `codex.Dockerfile`
pins `0.145.0-alpha.18` from npm and copies it to the same path, which matches
the policy but has not been built or run. Expect the copy step to need the
package's real binary layout.
