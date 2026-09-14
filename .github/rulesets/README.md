# Branch rulesets

The JSON files here are the source of truth for the repository's GitHub
branch rulesets. Rulesets are repository settings, not files, so a change to
one is only visible if it is committed here first and then synced with:

```bash
scripts/ci/apply-rulesets.sh            # tinyhumansai/opencompany
scripts/ci/apply-rulesets.sh owner/repo # another repository
```

The script matches each file to a live ruleset by `name` and creates or
updates it; it never deletes. They are copied from `tinyhumansai/openhuman`
and adjusted where this repository differs (the required check, the release
lane, the release workflows' push identity).

| File | Applies to | What it enforces |
|---|---|---|
| `protect-main.json` | `main` | No deletion, no force-push. Every change arrives by pull request with one approval from the `maintainers` team, given after the last push (`require_last_push_approval`), with stale approvals dismissed on push. `PR CI Gate` must be green. Merge or squash. Copilot review is on for non-draft PRs. |
| `protect-release.json` | `release` | No deletion, no force-push. Every change arrives by pull request with one approval (anyone), `PR CI Gate` green, squash only. |
| `disallow-creating-branches.json` | every branch except `main` and `release` | No creating, updating, or deleting branches on the canonical repository, with no bypass for anyone. Work happens on forks; this is what keeps the branch list at two. |
| `copilot-code-review.json` | default branch | Copilot review on every push, drafts included. **Disabled**, kept for parity with openhuman; `protect-main.json` already carries the enabled variant. |

## Who can bypass

`bypass_actors` names actors by numeric id, which is what the API takes:

| Id | Actor | Where | Mode |
|---|---|---|---|
| 3186922 | `tiny-humans-bot` GitHub App | main, release | always — this is what lets `promote-main-to-release.yml`, `release-staging.yml` and `release-production.yml` push the promotion merge, the version bump and the back-merge directly ([releases.md](../../docs/spec/runtime/releases.md)). GitHub Actions' own `GITHUB_TOKEN` cannot be put on a bypass list, which is why the workflows mint an App token. |
| 17075707 | `maintainers` team | main, release | pull requests only — a maintainer can merge a PR that lacks the review or the check; nobody can push around a PR. |
| 16568570 | `admin` team | release | pull requests only |
| 15368 | GitHub Actions | — | not a bypass: it is the `integration_id` of the required `PR CI Gate` check, so a check of the same name reported by any other app does not satisfy the rule. |

## The required check

`PR CI Gate` is the last job in `.github/workflows/ci.yml`. Every real lane
there is skipped when its area did not change, and a required check that
never reports blocks a PR forever, so the rulesets require the one job that
always runs and fails unless every lane passed or was skipped. Rename it and
every PR is blocked until the JSON here — and the live ruleset — say the new
name.

Openhuman's `main` requires `PR CI Gate` from its quick lane and `release`
requires `CI Full Gate` from its full lane. This repository has one workflow
that is the full suite for every PR whatever its base, so both rulesets
require the same check.
