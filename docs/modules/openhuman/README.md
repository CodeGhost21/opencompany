# OpenHuman Module

OpenHuman is the tenant harness, embedded as a **library**. The
`src/harness/` module links `openhuman_core` (`vendor/openhuman`) directly and,
under `feature = "openhuman"`, builds one openhuman `Agent` per manifest
`[[agent]]` through `AgentBuilder`. The default build links none of it and
keeps its offline, echo-brained behaviour.

The builder seams are wired to OpenCompany's own ports:

- **Persona** → each agent gets a system prompt framing it as its manifest
  `role` at the company, built with `SystemPromptBuilder::for_subagent` and
  `omit_identity` so it speaks as that role rather than openhuman's own
  assistant identity.
- **Memory** → `harness::memory::OcMemory`, an openhuman `Memory` over the
  OpenCompany `ContextStore`.
- **Inference provider** → `harness::provider::HostedProvider`, an
  OpenAI-compatible client for the hosted TinyHumans brain (`chat()` sends the
  full history and parses token/cost usage back out), with a `MockProvider`
  for offline tests.
- **Approval policy** → `harness::policy::ApprovalPolicy` maps `[policy].mode`
  onto openhuman's `ToolPolicy`; the security-tier words
  (readonly/supervised/full) line up 1:1.
- **Tools / skills** → injected from the company's manifest grants.

See [`docs/modules/runtime/README.md`](../runtime/README.md) for `HarnessPool`
and [`docs/spec/integrations/openhuman.md`](../../spec/integrations/openhuman.md)
for the full integration contract.

## `HarnessBrain` — cognition on the embedded runtime

`harness::brain::HarnessBrain` implements the `Brain` cognition port over a
`HarnessPool`: each operator message runs one openhuman agent turn and returns
the agent's reply, in place of the offline `EchoBrain`'s `"You said: …"`. A
company routes through it when the `RuntimeBuilder` has a harness pool
(`with_harness`) and no explicit brain — brain precedence is `with_brain` >
harness > hosted/echo. The `opencompany` binary's `attach_harness` resolves the
managed default from the environment (below).

Whether a company *has* inference configured is decided per cycle, not at boot
(issue #585). The builder wraps the harness brain and the hosted/echo fallback in
a `runtime::deferred::DeferredBrain`, which re-resolves the effective config —
console runtime override > manifest `[inference]` > managed env default — on
every `run_cycle` and dispatches to the harness when one resolves and to the
fallback when none does. That is what lets an admin set the company's key in the
console on a tenant that booted with no credential at all and have agents think
on the next cycle, with no container restart. Before this the decision was frozen
at `RuntimeBuilder::build`, and since a runtime is built once and cached in the
`CompanyRegistry`, a console-set key was unreachable until a restart.

## Inference config (environment)

`harness::provider::harness_inference_from_env` resolves the endpoint, key, and
default model, most specific first:

| Value | Source | Fallback |
| --- | --- | --- |
| key | `OPENCOMPANY_INFERENCE_KEY` | `TINYHUMANS_API_KEY` — **no key ⇒ echo brain** |
| url | `OPENCOMPANY_INFERENCE_URL` | `https://api.tinyhumans.ai/openai/v1` |
| model | `OPENCOMPANY_INFERENCE_MODEL` | `chat-v1` |

The two key names keep a per-tenant override distinct from the platform-wide
credential the hosting manager injects.

This is the **lowest**-precedence source. A company's own key, set write-only
through the console (`PUT …/inference` with `key`, stored under the
`inference/key` secret), wins over both env names — including on the `managed`
provider, where only the credential changes and the platform endpoint is kept.
Clearing it (`PUT …/inference` with `key: ""`, the console's **Remove key**)
falls back to the env credential rather than 401ing. See
`docs/modules/runtime/README.md` for the resolution order in full.

## Cost metering

`harness::cost` maps a completed turn's usage onto the ledger and the
`UsageMeter`. `HarnessPool::run` reads the real per-turn token/cost totals from
openhuman's public `Agent::last_turn_usage()` accessor
(tinyhumansai/openhuman#4940), so metering is **live**. Gating differs by
surface: a usage sample is recorded whenever tokens moved (the `/openai/v1`
passthrough reports tokens but bills backend-side, echoing no USD), while a
ledger `inference.spend` entry is written only when the turn actually cost USD —
so a token-bearing zero-cost turn meters usage without a `$0.00` spend line. An
offline provider that reports no usage yields a zero turn, which writes nothing.

## `src/openhuman/` — legacy JSON-RPC path (behind `openhuman-rpc`)

The former out-of-process seam is retained for one release and then removed.
`src/openhuman/` still hosts the launcher (`opencompany open-human --dry-run`
shells out through Cargo to the vendored checkout) and the JSON-RPC adapters —
`rpc.rs` (the `OpenHumanRpc` transport trait + `MockOpenHumanRpc`),
`http_client.rs` (the `reqwest` client behind `openhuman-rpc`), `tools.rs`
(`OpenHumanToolProvider`, catalog filtered by manifest grants, ungranted calls
rejected), and `channel.rs` (`OpenHumanChannelAdapter`). It degrades to
built-in tools and the operator channel with a boot warning when OpenHuman is
unreachable — never a boot failure. New work targets the embedded library, not
this path.
