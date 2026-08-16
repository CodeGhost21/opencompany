# Company Module

The company module owns the on-disk [company manifest](../../spec/runtime/manifest.md)
and the entrypoints that load it. It is Phase 0 of the runtime: parse and
validate a company definition, then boot it far enough to report its effective
configuration. The cognition kernel (Brain, cycle loop, stores) lands in later
phases — see [`docs/spec/roadmap.md`](../../spec/roadmap.md).

## Surface

- `CompanyManifest::from_path` — locate, read, parse, and validate a manifest
  from a file or a directory. `discover` prefers `company.toml` over the legacy
  `agents.toml`. When the bundle carries an `agents/` directory the roster is
  read from those per-teammate files instead of from `[[agent]]`; declaring both
  forms is a validation error. A caller that needs the `Located` value for
  itself goes through `from_located` rather than `from_file`, so the
  bundle-roster check cannot be skipped by taking a different route in.
- `agent_file` — the `agents/<id>.toml` loader: filename-as-id, deterministic
  ordering, and `prompt_files` resolved against the bundle. See
  [runtime/agents.md](../../spec/runtime/agents.md).
- `prompt` — how a teammate's system prompt is composed from its definition and
  the documents routed to it, including the budget clamp.
- `context_routing` — the per-tier default table and the class-based exclusions
  deciding which workspace documents reach a role. See
  [context-routing.md](../../spec/runtime/orchestration/context-routing.md).
- `tool_catalog` — one list of everything a company can grant: built-in
  families, MCP servers, Composio toolkits. See
  [runtime/tools.md](../../spec/runtime/tools.md).
- `CompanyManifest::validate` — return every problem in prosumer language
  (e.g. "`[policy].mode` must be one of readonly, supervised, full — you wrote
  `supervized`"), never serde traces. Problems are collected and reported
  together, not one at a time.
- `CompanyManifest::effective_summary` — a human-readable snapshot of the
  effective configuration used by the boot banner and `opencompany check`.
- `run_company(path)` — the two-line entrypoint each `examples/*` harness
  calls; loads, validates, and prints the company.
- `run_check(path)` — backs `opencompany check <dir>`; prints a deprecation
  note for `agents.toml`, the effective config on success, or every problem on
  failure.

## Compatibility

Every key in today's `agents.toml` keeps its exact meaning, and a bare
`agents.toml` (just `[company]` + `[[agent]]`) remains a complete, valid
company. Every new table is optional with a prosumer-safe default: the defaults
produce a working company (`hosted` brain, `supervised` policy, `openhuman`
tools, private) with only `TINYHUMANS_API_KEY` set at runtime.

Enum-like fields are deserialized as plain strings and validated against known
sets in `manifest.rs` so validation errors stay actionable. Keep new keys
optional and defaulted, and keep the validation messages written for a
non-technical operator.
