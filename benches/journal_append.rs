//! What a durable journal append costs (issue #392).
//!
//! The per-record-kind durability policy in `RuntimeJournal::append` rests on a
//! claim about price: a flush is affordable on `EffectExecuted` because that
//! record is written at operator-decision scale in front of a network call, and
//! unaffordable on `CycleStarted` because that one is written on the front edge
//! of every cycle. This bench is the measurement behind the claim.
//!
//! Three write modes, at two realistic record sizes:
//!
//! * `plain` — one `write_all` under `O_APPEND`, what `store::fs::append_line`
//!   does. Process-crash durable.
//! * `sync_data` — the same write plus `File::sync_data`, what
//!   `store::fs::append_line_durable` does for an append to an existing file.
//! * `sync_data_and_dir` — plus opening the parent directory and `sync_all`ing
//!   it. In production that is paid **only by the append that creates the
//!   file**; here every iteration pays it, so the number is the upper bound for
//!   that step rather than its amortised cost.
//!
//! # Reading these numbers honestly
//!
//! * **macOS overstates the cost.** Rust's `sync_data` maps to a full-flush
//!   -flavoured `fcntl` on macOS, which is stronger (and slower, ~10-20ms on
//!   APFS) than the Linux `fdatasync` a container would issue. A local macOS
//!   figure is an upper-bound flavour, not the hosted truth.
//! * **A local SSD does not predict EFS.** The deployed path writes to a network
//!   volume whose client page cache is the thing `sync_data` forces through;
//!   its absolute latency is a different measurement on different hardware.
//!
//! So what these numbers evidence is the **order of magnitude** of a flush and
//! **what fraction of appends pay it** — which is the part the policy turns on —
//! not the hosted absolute cost.
//!
//! # Measured baseline (macOS 15 / APFS / local SSD, `cargo bench`)
//!
//! | mode | ~200B p50 | ~200B p99 | ~700B p50 | ~700B p99 |
//! |---|---|---|---|---|
//! | `plain` | 16.1µs | 34.9µs | 19.9µs | 72.0µs |
//! | `sync_data` | 3.98ms | 7.31ms | 3.99ms | 6.03ms |
//! | `sync_data_and_dir` | 4.01ms | 8.94ms | 7.90ms | 11.30ms |
//!
//! Two things fall out, and both are the policy's argument:
//!
//! * A flush costs roughly **200× a plain append** here — far too much to spend
//!   on `CycleStarted`, which is written on the front edge of every cycle, and
//!   entirely invisible in front of an `EffectExecuted`'s 100ms-2s network call.
//!   Blanket-flushing would have been the expensive answer to a rare problem.
//! * The cost is **flat in record size** (~4ms at both 200B and 700B): it is the
//!   flush, not the payload. So "how many appends flush" is the only lever, which
//!   is exactly the lever a per-record-kind policy pulls.
//!
//! The directory flush is charged on every iteration here; in production only a
//! journal's first-ever append pays it.
//!
//! The three modes are reimplemented here rather than called: `append_line` and
//! `append_line_durable` are `pub(crate)`, and a bench is an external crate.
//! They mirror `store::fs::append_line_inner` exactly, minus the
//! `spawn_blocking` hop, which is deliberate — the point is to price the
//! syscalls, not tokio's scheduler.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

/// How an append is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    /// `store::fs::append_line`: one `O_APPEND` write, no flush.
    Plain,
    /// `store::fs::append_line_durable` appending to a file that exists.
    SyncData,
    /// `store::fs::append_line_durable` on the append that creates the file —
    /// charged on every iteration, so an upper bound for the directory flush.
    SyncDataAndDir,
}

impl Mode {
    const ALL: [Mode; 3] = [Mode::Plain, Mode::SyncData, Mode::SyncDataAndDir];

    fn name(self) -> &'static str {
        match self {
            Mode::Plain => "plain",
            Mode::SyncData => "sync_data",
            Mode::SyncDataAndDir => "sync_data_and_dir",
        }
    }
}

/// One append, exactly as `store::fs::append_line_inner` performs it.
fn append(path: &Path, record: &str, mode: Mode) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open");
    file.write_all(record.as_bytes()).expect("write");
    if mode != Mode::Plain {
        file.sync_data().expect("sync_data");
    }
    if mode == Mode::SyncDataAndDir {
        let dir = File::open(path.parent().expect("parent")).expect("open parent");
        dir.sync_all().expect("sync parent");
    }
}

/// An `EffectExecuted` line at its realistic size (~200 bytes): the key plus the
/// executed-effect description, which is what the at-most-once commit carries.
fn effect_executed_line() -> String {
    let line = serde_json::json!({
        "record": "EffectExecuted",
        "key": "effect:task-01H9Z2QK3M4N5P6R7S8T9V0W1X:filing.submit:3",
        "effect": {
            "kind": "filing.submit",
            "amount_usd": 249.5,
            "task_id": "task-01H9Z2QK3M4N5P6R7S8T9V0W1X",
            "at_millis": 1_767_225_600_000u64,
            "irreversible": true
        }
    })
    .to_string();
    pad_to(line, 200)
}

/// An `ApprovalParked` line at its realistic size (~700 bytes): the whole parked
/// effect plus its four correlation keys. The largest record the journal writes
/// in normal operation, and a process-durable one — included to show the flush
/// cost is dominated by the syscall, not the payload.
fn approval_parked_line() -> String {
    let line = serde_json::json!({
        "record": "ApprovalParked",
        "id": "appr-01H9Z2QK3M4N5P6R7S8T9V0W1X",
        "effect": {
            "kind": "composio_execute",
            "group": "Spend",
            "amount_usd": 1249.0,
            "established_thread": false,
            "first_time_counterparty": true,
            "payload": {
                "tool": "gmail_send_email",
                "arguments": {
                    "recipient": "counterparty@example.invalid",
                    "subject": "Q3 filing package",
                    "body": "Attaching the signed filing package for counter-signature."
                }
            },
            "agent": "finance",
            "run_id": "run-01H9Z2QK3M4N5P6R7S8T9V0W1X"
        },
        "at_millis": 1_767_225_600_000u64,
        "task": { "kind": "Task", "id": "task-01H9Z2QK3M4N5P6R7S8T9V0W1X" },
        "thread": "desk-finance",
        "parent": 4821,
        "cycle": "cycle-01H9Z2QK3M4N5P6R7S8T9V0W1X"
    })
    .to_string();
    pad_to(line, 700)
}

/// Pads a line to `target` bytes with a filler field, so the two sizes are the
/// sizes they claim to be regardless of how the JSON above happens to serialize.
fn pad_to(line: String, target: usize) -> String {
    let mut line = line;
    if line.len() + 12 < target {
        let filler = "x".repeat(target - line.len() - 12);
        line.truncate(line.len() - 1);
        line.push_str(&format!(",\"_pad\":\"{filler}\"}}"));
    }
    line.push('\n');
    line
}

/// A directly measured latency profile, printed before the criterion run.
///
/// Criterion reports a mean and a median; it does not report a p99, and the tail
/// is the interesting part of a flush. So the percentiles are sampled here by
/// hand: `SAMPLES` timed appends per (mode, size), sorted, reported at p50/p99
/// alongside the appends-per-second implied by the p50.
fn print_latency_percentiles() {
    const SAMPLES: usize = 400;

    println!("\n# issue #392 — append latency by durability mode");
    println!("# (macOS sync_data is a full-flush flavour; local SSD, not EFS — see module docs)");
    println!(
        "\n{:<20} {:>6} {:>12} {:>12} {:>14}",
        "mode", "bytes", "p50", "p99", "appends/sec@p50"
    );

    for (size_label, line) in [
        ("~200B", effect_executed_line()),
        ("~700B", approval_parked_line()),
    ] {
        for mode in Mode::ALL {
            let dir = tempfile::Builder::new()
                .prefix("opencompany-bench-")
                .tempdir()
                .expect("tempdir");
            let path = dir.path().join("journal.jsonl");

            let mut samples = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let start = Instant::now();
                append(&path, &line, mode);
                samples.push(start.elapsed());
            }
            samples.sort_unstable();

            let p50 = samples[SAMPLES / 2];
            let p99 = samples[(SAMPLES * 99) / 100];
            let per_sec = 1.0 / p50.as_secs_f64();
            println!(
                "{:<20} {:>6} {:>12} {:>12} {:>14.0}",
                format!("{} {}", mode.name(), size_label),
                line.len(),
                fmt(p50),
                fmt(p99),
                per_sec
            );
        }
    }
    println!();
}

fn fmt(d: Duration) -> String {
    let micros = d.as_secs_f64() * 1e6;
    if micros >= 1000.0 {
        format!("{:.3}ms", micros / 1000.0)
    } else {
        format!("{micros:.1}us")
    }
}

fn journal_append(c: &mut Criterion) {
    print_latency_percentiles();

    let mut group = c.benchmark_group("journal_append");
    group.throughput(Throughput::Elements(1));

    for (size_label, line) in [
        ("effect_executed_200b", effect_executed_line()),
        ("approval_parked_700b", approval_parked_line()),
    ] {
        for mode in Mode::ALL {
            let dir = tempfile::Builder::new()
                .prefix("opencompany-bench-")
                .tempdir()
                .expect("tempdir");
            let path = dir.path().join("journal.jsonl");

            group.bench_with_input(
                BenchmarkId::new(mode.name(), size_label),
                &line,
                |b, line| b.iter(|| append(&path, line, mode)),
            );
        }
    }

    group.finish();
}

criterion_group!(benches, journal_append);
criterion_main!(benches);
