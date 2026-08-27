#!/usr/bin/env node
// Dynamic release notes for OpenCompany, ported from the generator OpenHuman
// has used since its v0.57 line (`scripts/release/generate-release-notes.mjs`
// there).
//
// WHY NOT `generate_release_notes: true`. GitHub's built-in note generation
// emits a flat "* <PR title> by @author in <url>" list in merge order. For a
// release spanning a hundred PRs that is a changelog nobody reads and which
// says nothing about what the release IS. This walks the same commit range but
// resolves each PR through `gh`, groups the work into themed highlights, and
// distinguishes FIRST-TIME contributors from returning ones — the one fact the
// built-in list cannot derive, because it never looks at history before the
// range.
//
// TWO RENDERERS, and the AI one is not required. `--no-ai` produces the whole
// document deterministically from the keyword groups below; the OpenAI path
// only rewrites that same payload into prose. `release.yml` calls the AI path
// first and falls back to `--no-ai` on any failure, so a missing or rate-limited
// `OPENAI_API_KEY` degrades the notes rather than failing the release.
//
// Every function that has a pure input and output is exported for
// `generate-release-notes.test.mjs`; the git/gh/network ones are not.
import { execFileSync } from 'node:child_process';
import { existsSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const DEFAULT_MODEL = 'gpt-5.2';
const DEFAULT_FROM = 'latest-release';
const DEFAULT_TO = 'latest-tag';
// The prompt ceiling is ours, not the model's: a 300-PR range serialises to
// megabytes of PR bodies, and the request is rejected at the API rather than
// truncated. `serializeOpenAiPayload` sheds detail in stages to stay under it.
const MAX_PROMPT_CHARS = 120_000;
const MAX_PR_BODY_CHARS = 700;

function usage() {
  return `Usage: node scripts/release/generate-release-notes.mjs [options]

Generate Markdown release notes from merged PRs between two refs.

Options:
  --from <tag>          Start tag/ref, excluded from the range. Defaults to ${DEFAULT_FROM}.
                         Use latest-release to resolve the most recent GitHub Release tag.
  --to <ref>            End ref, included. Defaults to ${DEFAULT_TO}. Use main for testing.
  --repo <owner/repo>   GitHub repo. Defaults to the upstream/origin remote.
  --model <model>       OpenAI model. Defaults to ${DEFAULT_MODEL}.
  --output <file>       Write generated Markdown to a file instead of stdout.
  --no-ai               Build deterministic Markdown without calling OpenAI.
  --dry-run             Print the OpenAI request JSON without calling OpenAI.
  --help                Show this help.

Environment:
  OPENAI_API_KEY        Preferred OpenAI API key variable.
  OPENAI_API            Backward-compatible fallback API key variable.
  OPENAI_MODEL          Overrides the default model.
  GITHUB_REPOSITORY     Default for --repo (set for you inside Actions).

Examples:
  node scripts/release/generate-release-notes.mjs --to main --no-ai
  node scripts/release/generate-release-notes.mjs --from v0.1.0 --to v0.2.0 -o notes.md
`;
}

export function parseArgs(argv) {
  if (argv[0] === '--') {
    argv = argv.slice(1);
  }

  const options = {
    from: process.env.RELEASE_NOTES_FROM || DEFAULT_FROM,
    to: process.env.RELEASE_NOTES_TO || DEFAULT_TO,
    repo: process.env.GITHUB_REPOSITORY || null,
    model: process.env.OPENAI_MODEL || DEFAULT_MODEL,
    output: null,
    noAi: false,
    dryRun: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const readValue = (name) => {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error(`${name} requires a value`);
      }
      index += 1;
      return value;
    };

    if (arg === '--help' || arg === '-h') {
      return { ...options, help: true };
    }
    if (arg === '--from') {
      options.from = readValue(arg);
    } else if (arg === '--to') {
      options.to = readValue(arg);
    } else if (arg === '--repo') {
      options.repo = readValue(arg);
    } else if (arg === '--model') {
      options.model = readValue(arg);
    } else if (arg === '--output' || arg === '-o') {
      options.output = readValue(arg);
    } else if (arg === '--no-ai') {
      options.noAi = true;
    } else if (arg === '--dry-run') {
      options.dryRun = true;
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }

  return options;
}

function runGit(args, options = {}) {
  return execFileSync('git', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', options.allowFailure ? 'pipe' : 'inherit'],
  }).trim();
}

function runGh(args, options = {}) {
  return execFileSync('gh', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', options.allowFailure ? 'pipe' : 'inherit'],
  }).trim();
}

export function parseGitHubRepoFromRemote(remoteUrl) {
  const cleaned = String(remoteUrl || '')
    .trim()
    .replace(/\.git$/, '');
  const match = cleaned.match(/github\.com[:/]([^/\s]+\/[^/\s]+)$/);
  return match ? match[1] : null;
}

// `upstream` FIRST. A contributor checkout has `origin` pointing at their fork,
// and resolving the repo from it would ask `gh` for PR numbers that only exist
// on the canonical repo — every lookup 404s and every PR falls back to its
// commit subject. In Actions `GITHUB_REPOSITORY` short-circuits this entirely.
function resolveRepo(explicitRepo) {
  if (explicitRepo) {
    return explicitRepo;
  }
  for (const remote of ['upstream', 'origin']) {
    try {
      const url = runGit(['remote', 'get-url', remote], { allowFailure: true });
      const repo = parseGitHubRepoFromRemote(url);
      if (repo) {
        return repo;
      }
    } catch {
      // Try the next remote.
    }
  }
  throw new Error('Could not infer GitHub repo. Pass --repo owner/repo.');
}

function resolveEndRef(to) {
  if (to !== 'latest-tag') {
    return to;
  }
  const tag = runGit(['describe', '--tags', '--abbrev=0']);
  if (!tag) {
    throw new Error('Could not resolve latest tag');
  }
  return tag;
}

// The most recent RELEASE, not the most recent tag: the tag being published is
// usually already pushed by the time this runs, so `git describe` would resolve
// the start of the range to its end and produce an empty document.
function resolveStartRef(repo, from) {
  if (from !== 'latest-release') {
    return from;
  }
  const tag = runGh(['release', 'view', '--repo', repo, '--json', 'tagName', '--jq', '.tagName']);
  if (!tag) {
    throw new Error(`Could not resolve latest GitHub Release tag for ${repo}`);
  }
  return tag;
}

// A shallow clone is the usual cause of a miss here, and its failure mode is
// silent: `git log A..B` against an unknown A prints nothing, so the notes come
// out empty and green. Say which end is missing instead.
function assertRefExists(ref, label) {
  try {
    runGit(['rev-parse', '--verify', `${ref}^{commit}`], { allowFailure: true });
  } catch {
    throw new Error(`${label} ref not found: ${ref} (a shallow checkout needs fetch-depth: 0)`);
  }
}

export function extractPullRequestNumbers(subject) {
  const matches = [...String(subject || '').matchAll(/\(#(\d+)\)/g)];
  return [...new Set(matches.map((match) => Number(match[1])).filter(Number.isInteger))];
}

// `%x1f` between fields and `%x1e` between records: both are ASCII separators
// that cannot occur in a commit subject, an author name, or an email, so no
// commit message can forge a record boundary the way a newline delimiter would.
export function parseGitLog(logText) {
  if (!logText.trim()) {
    return [];
  }

  return logText
    .split('\x1e')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const [sha, subject, authorName, authorEmail, authoredAt] = entry.split('\x1f');
      const prNumbers = extractPullRequestNumbers(subject);
      return {
        sha,
        shortSha: sha.slice(0, 9),
        subject,
        authorName,
        authorEmail,
        authoredAt,
        prNumbers,
        // The LAST number wins. A squashed merge reads "fix: thing (#12) (#34)"
        // when the branch itself referenced an issue; #34 is the merge.
        primaryPrNumber: prNumbers.at(-1) || null,
      };
    });
}

function collectCommits(from, to) {
  const format = '%H%x1f%s%x1f%an%x1f%ae%x1f%aI%x1e';
  const output = runGit(['log', `${from}..${to}`, '--reverse', `--format=${format}`]);
  return parseGitLog(output);
}

// Everything reachable from the START of the range. Anyone absent from this set
// is a first-time contributor, which is the fact the built-in generator cannot
// produce.
function priorAuthorKeys(from) {
  const output = runGit(['log', from, '--format=%an%x1f%ae%x1e']);
  const keys = new Set();
  for (const entry of output.split('\x1e')) {
    const [name, email] = entry.trim().split('\x1f');
    if (name || email) {
      keys.add(authorKey({ authorName: name, authorEmail: email }));
    }
  }
  return keys;
}

export function authorKey(author) {
  return `${String(author.authorName || '').toLowerCase()} <${String(author.authorEmail || '').toLowerCase()}>`;
}

export function collectContributorStats(commits, priorKeys) {
  const contributors = new Map();
  for (const commit of commits) {
    const key = authorKey(commit);
    if (!contributors.has(key)) {
      contributors.set(key, {
        name: commit.authorName,
        email: commit.authorEmail,
        commits: 0,
        prs: new Set(),
        isNew: !priorKeys.has(key),
      });
    }
    const contributor = contributors.get(key);
    contributor.commits += 1;
    if (commit.primaryPrNumber) {
      contributor.prs.add(commit.primaryPrNumber);
    }
  }

  return [...contributors.values()]
    .map((contributor) => ({ ...contributor, prs: [...contributor.prs].sort((a, b) => a - b) }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function collectPrCommits(commits) {
  const byPr = new Map();
  for (const commit of commits) {
    if (!commit.primaryPrNumber) {
      continue;
    }
    if (!byPr.has(commit.primaryPrNumber)) {
      byPr.set(commit.primaryPrNumber, []);
    }
    byPr.get(commit.primaryPrNumber).push(commit);
  }
  return byPr;
}

// NEVER fatal. A PR can be unreachable for reasons that have nothing to do with
// this release — a transferred issue, a deleted fork, a `gh` rate limit partway
// through a hundred lookups. Falling back to the commit subject costs a title;
// throwing would cost the whole release's notes.
function fetchPullRequest(repo, number) {
  try {
    const json = runGh(
      ['pr', 'view', String(number), '--repo', repo, '--json', 'number,title,url,author,mergedAt,body,labels'],
      { allowFailure: true },
    );
    return JSON.parse(json);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      number,
      title: null,
      url: `https://github.com/${repo}/pull/${number}`,
      author: null,
      mergedAt: null,
      body: null,
      labels: [],
      warning: `Failed to fetch PR metadata with gh: ${message}`,
    };
  }
}

function collectPullRequests(repo, commits) {
  const prCommits = collectPrCommits(commits);
  const numbers = [...prCommits.keys()].sort((a, b) => a - b);

  return numbers.map((number) => {
    const detail = fetchPullRequest(repo, number);
    const commitsForPr = prCommits.get(number) || [];
    const fallbackTitle =
      commitsForPr.at(-1)?.subject.replace(/(?:\s+\(#\d+\))+\s*$/g, '') || `PR #${number}`;
    return {
      number,
      title: detail.title || fallbackTitle,
      url: detail.url || `https://github.com/${repo}/pull/${number}`,
      author: detail.author?.login || commitsForPr.at(-1)?.authorName || null,
      mergedAt: detail.mergedAt || commitsForPr.at(-1)?.authoredAt || null,
      labels: (detail.labels || []).map((label) => label.name || label).filter(Boolean),
      body: trimBody(detail.body),
      commits: commitsForPr.map((commit) => ({
        sha: commit.shortSha,
        subject: commit.subject,
        authorName: commit.authorName,
      })),
      warning: detail.warning,
    };
  });
}

// HTML comments go first: the PR template is mostly comments, and without this
// the payload is dominated by boilerplate the model then tries to summarise.
export function trimBody(body) {
  if (!body || typeof body !== 'string') {
    return '';
  }
  return body
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\r\n/g, '\n')
    .trim()
    .slice(0, MAX_PR_BODY_CHARS);
}

export function releaseTitle(from, to, resolvedTo) {
  const toLabel = to === 'latest-tag' ? resolvedTo : to;
  return `${from} to ${toLabel}`;
}

export function buildReleasePayload({ from, to, resolvedTo, repo, commits, pullRequests, contributors }) {
  return {
    repo,
    range: {
      from,
      to,
      resolvedTo,
      compareUrl: `https://github.com/${repo}/compare/${from}...${resolvedTo}`,
    },
    totals: {
      commits: commits.length,
      pullRequests: pullRequests.length,
      contributors: contributors.length,
      newContributors: contributors.filter((contributor) => contributor.isNew).length,
    },
    contributors: contributors.map(({ name, commits: count, prs, isNew }) => ({
      name,
      commits: count,
      prs,
      isNew,
    })),
    pullRequests,
    uncategorizedCommits: commits
      .filter((commit) => commit.prNumbers.length === 0)
      .map((commit) => ({ sha: commit.shortSha, subject: commit.subject, authorName: commit.authorName })),
  };
}

export function buildOpenAiRequest({ model, title, payload }) {
  const compactPayload = serializeOpenAiPayload(payload);

  return {
    model,
    input: [
      {
        role: 'developer',
        content:
          'You write polished but concrete release notes for OpenCompany, an operator console and runtime that runs a company as agents, workflows and append-only ledgers. Keep the tone warm, crisp, factual, and celebratory. Use tasteful emojis in headings, but do not overdo them. Never invent changes not present in the input. Preserve every PR link somewhere in the output.',
      },
      {
        role: 'user',
        content: `Create Markdown release notes for "${title}" from this JSON payload.

Required structure:
- Start with a short, exciting H1 title that summarizes the whole release theme, like "# The Ledger Upgrade" or "# The Self-Running Company Upgrade". Do not use the tag range as the title.
- Follow the title with a short celebratory paragraph and one tasteful emoji.
- Add "## Highlights" with multiple high-level highlight subsections, such as "### Agents & workflows". Use tasteful emojis in subsection headings.
- For each highlight subsection, write one or two short paragraphs maximum. Do not use highlight bullets.
- Each highlight subsection should summarize a cluster of related work, then end with compact PR links and contributor thanks, for example "([#123](url), [#124](url)) — Thank you @alice and @bob!"
- Across the highlight subsections, mention every PR in the payload exactly once if possible.
- Add "## New Contributors" celebrating first-time contributors only when there are first-time contributors. If none, omit this section entirely.
- For each new contributor, thank them and briefly describe what they contributed based on their PR titles.
- End the New Contributors section with a short note hoping they join the community for contributor rewards.
- Add "## Contributor Credits" thanking all contributors.
- Do not add a "## Pull Requests" section.
- Add "## Full Compare" with the compare URL.

JSON payload:
${compactPayload}`,
      },
    ],
  };
}

// Three stages, each losing the least valuable thing left: full payload, then
// PR bodies and commit lists, then whole PRs from the tail with a count of what
// was dropped so the model can say so rather than silently omit them.
export function serializeOpenAiPayload(payload) {
  const fullPayload = JSON.stringify(payload);
  if (fullPayload.length <= MAX_PROMPT_CHARS) {
    return fullPayload;
  }

  const compact = {
    ...payload,
    contributors: payload.contributors.map(({ name, isNew }) => ({ name, isNew })),
    pullRequests: payload.pullRequests.map(({ number, title, url, author, labels }) => ({
      number,
      title,
      url,
      author,
      labels,
    })),
    uncategorizedCommits: payload.uncategorizedCommits.map(({ subject, authorName }) => ({
      subject,
      authorName,
    })),
  };
  const compactPayload = JSON.stringify(compact);
  if (compactPayload.length <= MAX_PROMPT_CHARS) {
    return compactPayload;
  }

  const bounded = { ...compact, pullRequests: [], omittedPullRequests: compact.pullRequests.length };
  for (const pullRequest of compact.pullRequests) {
    bounded.pullRequests.push(pullRequest);
    bounded.omittedPullRequests -= 1;
    if (JSON.stringify(bounded).length > MAX_PROMPT_CHARS) {
      bounded.pullRequests.pop();
      bounded.omittedPullRequests += 1;
      break;
    }
  }
  return JSON.stringify(bounded);
}

function getOpenAiKey() {
  if (process.env.OPENAI_API_KEY) {
    return process.env.OPENAI_API_KEY;
  }
  if (process.env.OPENAI_API) {
    console.error('[release-notes] Using OPENAI_API; prefer OPENAI_API_KEY.');
    return process.env.OPENAI_API;
  }
  return null;
}

export function extractResponseText(responseJson) {
  if (typeof responseJson.output_text === 'string') {
    return responseJson.output_text;
  }
  const parts = [];
  for (const item of responseJson.output || []) {
    for (const content of item.content || []) {
      if (typeof content.text === 'string') {
        parts.push(content.text);
      }
    }
  }
  return parts.join('\n').trim();
}

async function summarizeWithOpenAi(request) {
  const key = getOpenAiKey();
  if (!key) {
    throw new Error('OPENAI_API_KEY is required unless --no-ai or --dry-run is used');
  }

  // A reasoning model on a hundred-PR payload is minutes, not seconds. The
  // abort matters because `fetch` has no default timeout: without it a hung
  // connection holds the release job open until the job timeout, and the
  // `--no-ai` fallback in `release.yml` never gets its turn.
  const timeoutMs = 300_000;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  let response;
  try {
    response = await fetch('https://api.openai.com/v1/responses', {
      method: 'POST',
      signal: controller.signal,
      headers: { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' },
      body: JSON.stringify(request),
    });
  } catch (error) {
    if (error?.name === 'AbortError') {
      throw new Error(`OpenAI Responses API timed out after ${timeoutMs}ms`);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }

  const body = await response.text();
  let json = null;
  try {
    json = JSON.parse(body);
  } catch {
    // Keep the raw body in the error below.
  }

  if (!response.ok) {
    throw new Error(`OpenAI Responses API failed (${response.status}): ${json?.error?.message || body}`);
  }

  const text = extractResponseText(json);
  if (!text) {
    throw new Error('OpenAI response did not contain output text');
  }
  return text;
}

export function renderDeterministicNotes({ title, payload }) {
  const newContributors = payload.contributors.filter((contributor) => contributor.isNew);
  const newContributorsSection = newContributors.length
    ? `
## New Contributors 🌟

${newContributors.map((contributor) => renderNewContributorLine(contributor, payload.pullRequests)).join('\n')}

Hope you stick around — first patches are how most of this repository got written.
`
    : '';
  const contributorLinks = payload.contributors.map((contributor) => {
    const prList = contributor.prs.map((number) => `#${number}`).join(', ');
    return `- ${contributor.name}${prList ? ` (${prList})` : ''}`;
  });
  const highlightSections = groupPullRequestsByHighlight(payload.pullRequests).map(
    ({ title: groupTitle, summary, pullRequests }) => {
      const links = pullRequests.map((pr) => `[#${pr.number}](${pr.url})`).join(', ');
      const authors = [...new Set(pullRequests.map((pr) => pr.author).filter(Boolean))];
      const thanks = authors.length
        ? `Thank you ${formatHandleList(authors)}!`
        : 'Thank you to everyone who contributed!';
      return `### ${groupTitle}\n\n${summary} (${links}) — ${thanks}`;
    },
  );

  return `# The OpenCompany Upgrade 🎉

${title} brings ${payload.totals.pullRequests} PRs across ${payload.totals.commits} commits, with work across companies, agents, ledgers, the operator console, and the hosted platform. Thank you to everyone who contributed to this release. ✨

## Highlights 🚀

${highlightSections.join('\n\n') || 'No PRs found in this range.'}
${newContributorsSection}

## Contributor Credits 🙌

${contributorLinks.map((line) => `${line} — thank you!`).join('\n') || '- No contributors found.'}

## Full Compare 🧭

${payload.range.compareUrl}
`;
}

export function formatHandleList(handles) {
  const names = handles.map((handle) => `@${handle}`);
  if (names.length <= 2) {
    return names.join(' and ');
  }
  return `${names.slice(0, -1).join(', ')}, and ${names.at(-1)}`;
}

// Keyword buckets over PR title + labels, in priority order — the FIRST group
// whose keyword appears wins, so the more specific buckets are listed before
// the catch-all. The last group is also the fallback for anything unmatched, so
// no PR can be silently dropped from the document.
//
// These names track this repository's own module surfaces (see CLAUDE.md):
// companies/globals, src/ledger, src/server + frontend, the hosted-platform
// seams, and the Tauri desktop shell.
export function groupPullRequestsByHighlight(pullRequests) {
  const groups = [
    {
      title: '🏢 Companies, agents & workflows',
      keywords: [
        'company', 'companies', 'vertical', 'global', 'globals', 'agent', 'subagent',
        'workflow', 'skill', 'skills', 'prompt', 'tool belt', 'toolbelt', 'brain',
        'run', 'chat', 'inbox', 'task',
      ],
      summary:
        'The company runtime itself moves forward — the agents a company starts with, the workflows they run, and the skills and tool belts they reach for.',
      pullRequests: [],
    },
    {
      title: '📒 Ledgers, records & data',
      keywords: [
        'ledger', 'record', 'derived', 'fold', 'append-only', 'schema', 'migration',
        'mongodb', 'mongo', 'storage', 'persistence', 'port', 'seed', 'data',
      ],
      summary:
        'Ledgers and the storage layer beneath them get sharper, from declared record shapes and the append-only fold through to the persistence ports each backend implements.',
      pullRequests: [],
    },
    {
      title: '🖥️ Operator console & UI',
      keywords: [
        'console', 'frontend', 'ui', 'ux', 'design', 'token', 'theme', 'page', 'view',
        'component', 'react', 'vite', 'layout', 'nav', 'onboarding', 'logo', 'icon', 'legend',
      ],
      summary:
        'The operator console gets clearer and calmer to work in, with interface polish, design-system work, and screens that explain more of what the runtime is doing.',
      pullRequests: [],
    },
    {
      title: '☁️ Hosting, auth & platform',
      keywords: [
        'tenant', 'hosting', 'hosted', 'manager', 'deploy', 'docker', 'container',
        'healthz', 'auth', 'user', 'users', 'admin', 'invite', 'session', 'connect',
        'grant', 'credential', 'secret', 'security', 'sentry',
      ],
      summary:
        'Hosted operation is tightened across tenancy, sign-in and admin eligibility, connected-account grants, and the container seams the control plane injects into.',
      pullRequests: [],
    },
    {
      title: '🧰 Desktop, docs & developer foundations',
      keywords: [
        'tauri', 'desktop', 'dmg', 'macos', 'bundle', 'sign', 'notariz', 'release',
        'ci', 'workflow file', 'clippy', 'fmt', 'test', 'docs', 'readme', 'spec',
        'refactor', 'deps', 'dependency', 'bump', 'chore',
      ],
      summary:
        'The desktop shell, documentation, and the build and release plumbing also move forward, keeping what ships reproducible and what it does written down.',
      pullRequests: [],
    },
  ];

  for (const pr of pullRequests) {
    const haystack = `${pr.title} ${(pr.labels || []).join(' ')}`.toLowerCase();
    const group = groups.find((candidate) => candidate.keywords.some((keyword) => haystack.includes(keyword)));
    (group || groups.at(-1)).pullRequests.push(pr);
  }

  return groups.filter((group) => group.pullRequests.length > 0);
}

function renderNewContributorLine(contributor, pullRequests) {
  const contributed = pullRequests
    .filter((pr) => contributor.prs.includes(pr.number))
    .map((pr) => `[#${pr.number}](${pr.url}) ${pr.title}`);
  const brief = contributed.length ? ` for ${contributed.join('; ')}` : '';
  return `- Welcome ${contributor.name}! Thank you${brief}.`;
}

// The prompt ASKS for every PR to appear; a language model is not a guarantee.
// This is the guarantee: anything the model dropped is appended in its own
// section rather than losing a contributor's credit to a summarisation choice.
export function ensureAllPullRequestsLinked(markdown, pullRequests) {
  const missing = pullRequests.filter((pr) => {
    const byNumber = new RegExp(`\\[#${pr.number}\\]\\(`).test(markdown);
    const byUrl = new RegExp(`${escapeRegExp(pr.url)}(?:\\)|\\s|$)`).test(markdown);
    return !byNumber && !byUrl;
  });
  if (missing.length === 0) {
    return markdown;
  }

  const links = missing.map((pr) => `[#${pr.number}](${pr.url})`).join(', ');
  const authors = [...new Set(missing.map((pr) => pr.author).filter(Boolean))];
  const thanks = authors.length
    ? `Thank you ${formatHandleList(authors)}!`
    : 'Thank you to everyone who contributed!';
  const insertion = `\n### Additional highlights 🔗\n\nA few more focused contributions round out this release with targeted fixes and improvements that are worth calling out. (${links}) — ${thanks}\n`;

  const sectionMatch = markdown.match(/\n## (New Contributors|Contributor Credits|Full Compare)\b/);
  if (!sectionMatch || typeof sectionMatch.index !== 'number') {
    return `${markdown.trim()}${insertion}`;
  }
  return `${markdown.slice(0, sectionMatch.index).trimEnd()}${insertion}${markdown.slice(sectionMatch.index)}`;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

async function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`[release-notes] ${error.message}`);
    console.error(usage());
    process.exit(2);
  }

  if (options.help) {
    process.stdout.write(usage());
    return;
  }

  const repo = resolveRepo(options.repo);
  const from = resolveStartRef(repo, options.from);
  const resolvedTo = resolveEndRef(options.to);

  assertRefExists(from, 'Start');
  assertRefExists(resolvedTo, 'End');

  console.error(`[release-notes] Collecting ${repo} changes from ${from} to ${resolvedTo}`);
  const commits = collectCommits(from, resolvedTo);
  const contributors = collectContributorStats(commits, priorAuthorKeys(from));
  const pullRequests = collectPullRequests(repo, commits);
  const payload = buildReleasePayload({
    from,
    to: options.to,
    resolvedTo,
    repo,
    commits,
    pullRequests,
    contributors,
  });
  const title = releaseTitle(from, options.to, resolvedTo);

  let markdown;
  if (options.dryRun) {
    markdown = JSON.stringify(buildOpenAiRequest({ model: options.model, title, payload }), null, 2);
  } else if (options.noAi) {
    markdown = renderDeterministicNotes({ title, payload });
  } else {
    markdown = await summarizeWithOpenAi(buildOpenAiRequest({ model: options.model, title, payload }));
    markdown = ensureAllPullRequestsLinked(markdown, payload.pullRequests);
  }

  if (options.output) {
    const outputPath = resolve(options.output);
    if (existsSync(outputPath) && basename(outputPath).startsWith('.')) {
      throw new Error(`Refusing to overwrite hidden file: ${outputPath}`);
    }
    writeFileSync(outputPath, markdown.endsWith('\n') ? markdown : `${markdown}\n`);
    console.error(`[release-notes] Wrote ${outputPath}`);
  } else {
    process.stdout.write(markdown.endsWith('\n') ? markdown : `${markdown}\n`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`[release-notes] ${error.message}`);
    process.exit(1);
  });
}
