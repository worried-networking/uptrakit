#!/usr/bin/env node
// Static guard for the Playwright e2e bucket manifest.
//
// Run from the `frontend` directory: `node scripts/check-e2e-buckets.mjs`
// (wired as `npm run test:e2e:check-buckets`).
//
// Asserts:
//   1. Every spec file under tests/e2e/*.{spec,test}.ts appears in exactly
//      one of `behavior | parity | skipped`.
//   2. Every listed file actually exists on disk.
//   3. Behavior specs do not import parity APIs (would crash on Linux CI).
//   4. Parity specs have a sibling <name>-snapshots/ directory.
//   5. Skipped specs emit a CI warning every run (no quiet rot).

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const FRONTEND_DIR = resolve(__dirname, '..');
const E2E_DIR = join(FRONTEND_DIR, 'tests', 'e2e');
const BUCKETS_PATH = join(E2E_DIR, 'buckets.json');

const PARITY_API_RE = /parity-config|expectParityScreenshot|toHaveScreenshot|toMatchSnapshot/;

const errors = [];
const fail = (msg) => errors.push(msg);

const buckets = JSON.parse(readFileSync(BUCKETS_PATH, 'utf8'));
for (const key of ['behavior', 'parity', 'skipped']) {
	if (!Array.isArray(buckets[key])) {
		fail(`buckets.json missing or non-array key "${key}"`);
	}
}
if (errors.length) {
	for (const e of errors) console.error(`ERROR: ${e}`);
	process.exit(1);
}

const specsOnDisk = readdirSync(E2E_DIR).filter((f) => f.endsWith('.spec.ts') || f.endsWith('.test.ts'));
const declared = [...buckets.behavior, ...buckets.parity, ...buckets.skipped];

// 1. Bijection between disk specs and declared specs.
const declaredSet = new Set(declared);
const onDiskSet = new Set(specsOnDisk);

for (const spec of specsOnDisk) {
	if (!declaredSet.has(spec)) {
		fail(`spec on disk not bucketed: ${spec}`);
	}
}
for (const spec of declared) {
	if (!onDiskSet.has(spec)) {
		fail(`bucketed spec does not exist: ${spec}`);
	}
}

// 1b. No spec in multiple buckets.
const seen = new Map();
for (const [bucket, list] of Object.entries(buckets)) {
	for (const spec of list) {
		if (seen.has(spec)) {
			fail(`spec "${spec}" listed in both "${seen.get(spec)}" and "${bucket}"`);
		} else {
			seen.set(spec, bucket);
		}
	}
}

// 3. Behavior specs must not use parity APIs.
for (const spec of buckets.behavior) {
	const path = join(E2E_DIR, spec);
	if (!onDiskSet.has(spec)) continue; // already reported above
	const src = readFileSync(path, 'utf8');
	if (PARITY_API_RE.test(src)) {
		fail(
			`behavior spec "${spec}" uses parity APIs (parity-config / ` +
				`expectParityScreenshot / toHaveScreenshot / toMatchSnapshot) ` +
				`— move to "parity" bucket`
		);
	}
}

// 4. Parity specs must have a sibling -snapshots/ dir.
for (const spec of buckets.parity) {
	if (!onDiskSet.has(spec)) continue;
	const snapDir = join(E2E_DIR, `${spec}-snapshots`);
	let exists;
	try {
		exists = statSync(snapDir).isDirectory();
	} catch {
		exists = false;
	}
	if (!exists) {
		fail(
			`parity spec "${spec}" has no sibling "${spec}-snapshots/" directory. ` +
				`Run "npm run test:e2e:parity -- ${spec} --update-snapshots" on macOS ` +
				`and commit the generated baselines.`
		);
	}
}

// 5. Skipped specs: GitHub Actions warning annotation every run.
for (const spec of buckets.skipped) {
	const path = `frontend/tests/e2e/${spec}`;
	console.warn(
		`::warning file=${path}::spec "${spec}" is skipped in CI. ` +
			`Fix the spec and move out of buckets.json "skipped" array.`
	);
}

if (errors.length) {
	console.error('');
	for (const e of errors) console.error(`ERROR: ${e}`);
	console.error('');
	console.error(`check-e2e-buckets: ${errors.length} error(s)`);
	process.exit(1);
}

console.log(
	`check-e2e-buckets: OK — ${buckets.behavior.length} behavior, ` +
		`${buckets.parity.length} parity, ${buckets.skipped.length} skipped`
);
