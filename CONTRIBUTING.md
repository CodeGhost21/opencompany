# Contributing

## Before You Open Anything

Questions, self-hosting trouble, and a run that misbehaved go to
[Discussions](https://github.com/tinyhumansai/opencompany/discussions);
reproducible behavior that should change goes to an issue.
[SUPPORT.md](SUPPORT.md) has the full routing table, and
[docs/community/discussions.md](docs/community/discussions.md) explains how
threads are triaged.

A change big enough to break an existing company — the manifest, stored data, a
public route — starts as an
[RFC](https://github.com/tinyhumansai/opencompany/discussions/categories/rfcs),
not as a pull request.

## Local Checks

Run these before opening a pull request:

```sh
git submodule update --init --recursive
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --features tiny
```

## Pull Requests

Keep changes focused. Include a short summary, any API or behavior changes, and
the local verification commands you ran.
