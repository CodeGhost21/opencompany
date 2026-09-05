# Hive Math Lab

The [Agentic Math Lab](../agentic_math_lab/README.md) with its three working
roles — theorist, programmer, verifier — seated on **one desk** instead of
four, so a stated problem is answered by a tinyhivemind deliberation episode
rather than by an orchestrator handing work from lead to lead. See
`docs/spec/runtime/hivemind.md` for the mechanics and
`scripts/hive-euler.py` for the headless Project Euler driver.

Run it locally against the ladder router and a CortexDB memory instance:

```bash
scripts/cortexdb-up.sh                     # prints the OPENCOMPANY_MEMORY_* exports
OPENCOMPANY_INFERENCE_KEY=$LADDER_API_KEY OPENCOMPANY_AUTH_MODE=none \
  cargo run --features openhuman --bin opencompany -- serve --company companies/hive_math_lab
python3 scripts/hive-euler.py --problems 1,5,12,31,60,100
```
## Tool servers

An answer here ships with the program that produced it, so the library documentation has to match the version that ran.

Declared in [`mcp.json`](mcp.json) and merged with anything the install
ships and anything an operator adds from the console. A server marked
*needs a token* is declared but off: write its credential from
Settings → Connections, then enable it there.

| Server | What it is for | Ships |
| --- | --- | --- |
| `deepwiki` | Documentation and Q&A for any public GitHub repository. Public and no-auth. | on |
| `context7` | Version-accurate API and library documentation, so answers match the release in use. | on |
| `huggingface` | Models, datasets and papers on the Hugging Face Hub. Public and no-auth. | on |
