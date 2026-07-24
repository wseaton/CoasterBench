# Benchmark findings

Behavioural observations from CoasterBench runs. Each entry is dated and cites
the run it came from, so a claim can be checked against the artifacts.

## kimi-k3 does not end a round on its own (2026-07-24)

**Run:** `20260724-kimi-k3-twister` (opencode / OpenRouter, ride type 51).

In every round, kimi-k3 ran the full 30-minute wall-clock budget and was stopped
by the `--session-timeout` rather than ending the session itself:

| Round | Wall time | finish_and_test calls | Score |
| --- | --- | --- | --- |
| 1 | ~30 min (timeout) | 3 | 6.08 |
| 2 | ~29 min (timeout) | 5 | 7.05 |
| 3 | ~30 min (timeout) | 5 | 5.32 |

The pattern each round is the same: test a working coaster, then demolish and
rebuild to try for a higher score, repeating until the timeout.

Claude Code models on the same task behave differently. In
`20260723-sub-twister-1` (fable-5, `--max-turns 120`, no wall-clock timeout),
sessions ended on their own at 46-89 turns, below the cap.

Whether this is the model or the opencode harness is not yet settled. The two
lanes differ in more than the model: Claude Code enforces `--max-turns` while
opencode has no turn cap, so kimi may be running to the wall-clock only because
nothing else bounds it. Separating the two needs another opencode model that
builds a real coaster, which we do not yet have.

The budget prompt did not change this. The round prompt states the wall-clock
budget ("you have about 30 minutes... bank a tested circuit early"), and this
run used that prompt. A one-shot prompt can state a deadline but cannot make the
model converge, since it has no clock and can always try another rebuild.

### Consequences for the harness

- **Cost.** A model that uses the full budget every round costs the full budget.
  kimi averaged about $7-8 per round here.
- **Best-result scoring matters for models like this.** kimi is stopped
  mid-rebuild every round, so scoring the final park state would record no
  coaster each time. Scoring the best tested result of the round (the server's
  `best_result`) records the real work: round 1 scored 6.08 for the same
  situation that scored zero before the change.
- **Stopping a model early requires driving the loop directly.** A mid-session
  reminder ("N minutes left, finalise now") needs per-turn injection, which the
  one-shot `opencode run` and `claude -p` invocations do not allow. That means
  running the agent loop the way driver.py does rather than delegating to the
  harness CLI. Whether it is worth doing is open, now that scores are protected.
