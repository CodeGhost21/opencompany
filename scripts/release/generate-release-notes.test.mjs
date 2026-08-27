// `node --test scripts/release/` — no test framework, no dependency, because
// this directory has no package.json and adding one to run four assertions
// would put a second npm project in a repository that has exactly one
// (`frontend/`). Run in CI by the `Console` job, alongside the other
// repository-wide policy checks.
//
// Only the pure functions are covered. `collectCommits`, `fetchPullRequest`
// and `summarizeWithOpenAi` shell out to git/gh or the network and are not
// exported; what they produce is a plain object, and every transformation
// applied to it after that point is tested here.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  buildReleasePayload,
  collectContributorStats,
  ensureAllPullRequestsLinked,
  extractPullRequestNumbers,
  extractResponseText,
  formatHandleList,
  groupPullRequestsByHighlight,
  parseArgs,
  parseGitHubRepoFromRemote,
  parseGitLog,
  releaseTitle,
  renderDeterministicNotes,
  trimBody,
} from './generate-release-notes.mjs';

test('parseArgs defaults and overrides', () => {
  const defaults = parseArgs([]);
  assert.equal(defaults.from, 'latest-release');
  assert.equal(defaults.to, 'latest-tag');
  assert.equal(defaults.noAi, false);

  const custom = parseArgs(['--from', 'v0.1.0', '--to', 'main', '--no-ai', '-o', 'notes.md']);
  assert.equal(custom.from, 'v0.1.0');
  assert.equal(custom.to, 'main');
  assert.equal(custom.noAi, true);
  assert.equal(custom.output, 'notes.md');

  assert.throws(() => parseArgs(['--nope']), /Unknown option/);
  // A flag swallowing the next flag as its value is the silent failure here:
  // `--from --no-ai` would otherwise set from='--no-ai' and produce an empty range.
  assert.throws(() => parseArgs(['--from', '--no-ai']), /requires a value/);
});

test('parseGitHubRepoFromRemote handles ssh and https', () => {
  assert.equal(parseGitHubRepoFromRemote('git@github.com:tinyhumansai/opencompany.git'), 'tinyhumansai/opencompany');
  assert.equal(parseGitHubRepoFromRemote('https://github.com/tinyhumansai/opencompany'), 'tinyhumansai/opencompany');
  assert.equal(parseGitHubRepoFromRemote('https://gitlab.com/a/b.git'), null);
  assert.equal(parseGitHubRepoFromRemote(''), null);
});

test('extractPullRequestNumbers takes the merge PR last', () => {
  assert.deepEqual(extractPullRequestNumbers('fix: thing (#12)'), [12]);
  assert.deepEqual(extractPullRequestNumbers('fix: closes (#12) (#34)'), [12, 34]);
  assert.deepEqual(extractPullRequestNumbers('chore: no pr'), []);
});

test('parseGitLog splits on ASCII separators', () => {
  const log = [
    ['aaaaaaaaaaaa1', 'feat: ledgers (#7)', 'Ada', 'ada@example.com', '2026-08-01T00:00:00Z'].join('\x1f'),
    ['bbbbbbbbbbbb2', 'chore: tidy', 'Bo', 'bo@example.com', '2026-08-02T00:00:00Z'].join('\x1f'),
  ].join('\x1e');

  const commits = parseGitLog(log);
  assert.equal(commits.length, 2);
  assert.equal(commits[0].primaryPrNumber, 7);
  assert.equal(commits[1].primaryPrNumber, null);
  assert.equal(parseGitLog('   ').length, 0);
});

test('collectContributorStats flags first-time contributors', () => {
  const commits = parseGitLog(
    [
      ['a1', 'feat: a (#1)', 'Ada', 'ada@example.com', '2026-08-01T00:00:00Z'].join('\x1f'),
      ['b2', 'feat: b (#2)', 'Bo', 'bo@example.com', '2026-08-02T00:00:00Z'].join('\x1f'),
      ['c3', 'feat: c (#3)', 'Ada', 'ada@example.com', '2026-08-03T00:00:00Z'].join('\x1f'),
    ].join('\x1e'),
  );
  // Case-insensitive on purpose: git records whatever casing the author's
  // config carries, and "ADA" is not a new contributor.
  const prior = new Set(['ADA'.toLowerCase()]);

  const stats = collectContributorStats(commits, prior);
  assert.deepEqual(
    stats.map((s) => [s.name, s.commits, s.prs, s.isNew]),
    [
      ['Ada', 2, [1, 3], false],
      ['Bo', 1, [2], true],
    ],
  );
});

test('collectContributorStats merges one person committing under two emails', () => {
  // The real shape this guards: a contributor whose PR merges land under their
  // personal address and whose web edits land under GitHub's noreply one. Keyed
  // on name+email they appeared twice — once with the PRs, once as a bare line
  // that also claimed to be a first-time contributor.
  const commits = parseGitLog(
    [
      ['a1', 'feat: a (#1)', 'Ada', 'ada@example.com', '2026-08-01T00:00:00Z'].join('\x1f'),
      ['a2', 'docs: b (#2)', 'Ada', '1234+ada@users.noreply.github.com', '2026-08-02T00:00:00Z'].join('\x1f'),
    ].join('\x1e'),
  );

  const stats = collectContributorStats(commits, new Set(['ada']));
  assert.equal(stats.length, 1);
  assert.equal(stats[0].commits, 2);
  assert.deepEqual(stats[0].prs, [1, 2]);
  assert.equal(stats[0].isNew, false);
});

test('groupPullRequestsByHighlight buckets by keyword and never drops a PR', () => {
  const prs = [
    { number: 1, title: 'feat: company logo backend', url: 'u1', labels: [] },
    { number: 2, title: 'fix: ledger fold ordering', url: 'u2', labels: [] },
    { number: 3, title: 'fix: run status legend', url: 'u3', labels: [] },
    { number: 4, title: 'chore: notarize the dmg', url: 'u4', labels: [] },
    { number: 5, title: 'something entirely unclassifiable', url: 'u5', labels: [] },
  ];

  const groups = groupPullRequestsByHighlight(prs);
  const placed = groups.flatMap((group) => group.pullRequests.map((pr) => pr.number));
  assert.deepEqual(placed.sort((a, b) => a - b), [1, 2, 3, 4, 5]);
  assert.ok(groups.every((group) => group.pullRequests.length > 0));
  // The unmatched PR falls into the last bucket rather than vanishing.
  assert.ok(groups.at(-1).pullRequests.some((pr) => pr.number === 5));
});

test('renderDeterministicNotes emits every PR link and omits an empty new-contributor section', () => {
  const commits = parseGitLog(
    [
      ['a1', 'feat: ledger fold (#1)', 'Ada', 'ada@example.com', '2026-08-01T00:00:00Z'].join('\x1f'),
      ['b2', 'fix: console nav (#2)', 'Bo', 'bo@example.com', '2026-08-02T00:00:00Z'].join('\x1f'),
    ].join('\x1e'),
  );
  const contributors = collectContributorStats(commits, new Set(['ada', 'bo']));
  const pullRequests = [
    { number: 1, title: 'feat: ledger fold', url: 'https://x/1', author: 'ada', labels: [], commits: [] },
    { number: 2, title: 'fix: console nav', url: 'https://x/2', author: 'bo', labels: [], commits: [] },
  ];
  const payload = buildReleasePayload({
    from: 'v0.1.0',
    to: 'v0.2.0',
    resolvedTo: 'v0.2.0',
    repo: 'tinyhumansai/opencompany',
    commits,
    pullRequests,
    contributors,
  });

  assert.equal(payload.totals.commits, 2);
  assert.equal(payload.totals.newContributors, 0);
  assert.equal(payload.range.compareUrl, 'https://github.com/tinyhumansai/opencompany/compare/v0.1.0...v0.2.0');

  const markdown = renderDeterministicNotes({ title: releaseTitle('v0.1.0', 'v0.2.0', 'v0.2.0'), payload });
  assert.match(markdown, /\[#1\]\(https:\/\/x\/1\)/);
  assert.match(markdown, /\[#2\]\(https:\/\/x\/2\)/);
  assert.match(markdown, /## Contributor Credits/);
  assert.doesNotMatch(markdown, /## New Contributors/);
  assert.match(markdown, /## Full Compare/);
});

test('renderDeterministicNotes celebrates first-time contributors when there are any', () => {
  const commits = parseGitLog(
    [['b2', 'fix: console nav (#2)', 'Bo', 'bo@example.com', '2026-08-02T00:00:00Z'].join('\x1f')].join('\x1e'),
  );
  const payload = buildReleasePayload({
    from: 'v0.1.0',
    to: 'v0.2.0',
    resolvedTo: 'v0.2.0',
    repo: 'tinyhumansai/opencompany',
    commits,
    pullRequests: [{ number: 2, title: 'fix: console nav', url: 'https://x/2', author: 'bo', labels: [], commits: [] }],
    contributors: collectContributorStats(commits, new Set()),
  });

  const markdown = renderDeterministicNotes({ title: 'v0.1.0 to v0.2.0', payload });
  assert.match(markdown, /## New Contributors/);
  assert.match(markdown, /Welcome Bo!/);
});

test('ensureAllPullRequestsLinked appends only what the model dropped', () => {
  const prs = [
    { number: 1, title: 'a', url: 'https://x/1', author: 'ada' },
    { number: 2, title: 'b', url: 'https://x/2', author: 'bo' },
  ];
  const complete = '# Notes\n\n[#1](https://x/1) and [#2](https://x/2)\n\n## Full Compare\n\nurl\n';
  assert.equal(ensureAllPullRequestsLinked(complete, prs), complete);

  const partial = '# Notes\n\n[#1](https://x/1)\n\n## Full Compare\n\nurl\n';
  const repaired = ensureAllPullRequestsLinked(partial, prs);
  assert.match(repaired, /### Additional highlights/);
  assert.match(repaired, /\[#2\]\(https:\/\/x\/2\)/);
  // Repaired in place, before the trailing sections — not stapled past them.
  assert.ok(repaired.indexOf('Additional highlights') < repaired.indexOf('## Full Compare'));
});

test('formatHandleList reads as English at every length', () => {
  assert.equal(formatHandleList(['a']), '@a');
  assert.equal(formatHandleList(['a', 'b']), '@a and @b');
  assert.equal(formatHandleList(['a', 'b', 'c']), '@a, @b, and @c');
});

test('trimBody strips PR-template comments and caps length', () => {
  assert.equal(trimBody('<!-- template -->\nreal text'), 'real text');
  assert.equal(trimBody(null), '');
  assert.ok(trimBody('x'.repeat(5000)).length <= 700);
});

test('extractResponseText reads both Responses API shapes', () => {
  assert.equal(extractResponseText({ output_text: 'hi' }), 'hi');
  assert.equal(
    extractResponseText({ output: [{ content: [{ text: 'a' }, { text: 'b' }] }] }),
    'a\nb',
  );
  assert.equal(extractResponseText({}), '');
});
