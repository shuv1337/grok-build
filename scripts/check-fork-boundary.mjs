#!/usr/bin/env node
/**
 * Fork boundary check.
 *
 * A fork drifts in two directions, and both are silent. Upstream branding
 * creeps back into product surfaces on merge, and a well-meaning rename
 * "finishes the job" by renaming a compatibility surface that other things
 * depend on. This asserts both directions: the names that MUST be ours, and
 * the names that MUST still be upstream's.
 *
 * Deliberately assertions, not a deny-list of banned words — the preserved
 * upstream names are as much a part of the contract as the renamed ones.
 *
 * Run it after any upstream merge, and in the release script.
 * Exit 0 = boundary intact. Exit 1 = a named violation.
 */

import { readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const NPM_DIR = "crates/codegen/xai-grok-pager/npm";
const PLATFORMS = [
	"darwin-arm64",
	"darwin-x64",
	"linux-arm64",
	"linux-x64",
	"win32-arm64",
	"win32-x64",
];

const failures = [];
const checks = [];

function read(rel) {
	const path = join(ROOT, rel);
	if (!existsSync(path)) return null;
	return readFileSync(path, "utf8");
}

/** Assert `rel` contains `needle`. */
function mustContain(rel, needle, why) {
	const body = read(rel);
	checks.push(rel);
	if (body === null) {
		failures.push(`${rel}: file is missing (${why})`);
		return;
	}
	if (!body.includes(needle)) {
		failures.push(`${rel}: expected to contain ${JSON.stringify(needle)} — ${why}`);
	}
}

/** Assert `rel` does NOT contain `needle`. */
function mustNotContain(rel, needle, why) {
	const body = read(rel);
	checks.push(rel);
	if (body === null) {
		failures.push(`${rel}: file is missing (${why})`);
		return;
	}
	if (body.includes(needle)) {
		failures.push(`${rel}: must not contain ${JSON.stringify(needle)} — ${why}`);
	}
}

// ── 1. Canonical identity: must be ours ─────────────────────────────────────

mustContain(
	"crates/codegen/xai-grok-version/src/lib.rs",
	'PRODUCT_NAME: &str = "ShuvGrok"',
	"the single source of the user-visible product name",
);

const meta = read(`${NPM_DIR}/shuvgrok/package.json`);
checks.push(`${NPM_DIR}/shuvgrok/package.json`);
if (!meta) {
	failures.push(`${NPM_DIR}/shuvgrok/package.json: missing — the npm meta package is the published artifact`);
} else {
	const pkg = JSON.parse(meta);
	if (pkg.name !== "@shuv1337/shuvgrok") {
		failures.push(`npm meta package name is ${pkg.name}, expected @shuv1337/shuvgrok`);
	}
	if (!pkg.bin || !pkg.bin.shuvgrok) {
		failures.push(`npm meta package must expose the "shuvgrok" command, got ${JSON.stringify(pkg.bin)}`);
	}
	if (pkg.bin && pkg.bin.grok) {
		failures.push(`npm meta package still exposes a "grok" command; this fork ships shuvgrok only`);
	}
	// The meta package pins each platform package to an exact version, so a
	// version skew publishes an installable package that cannot resolve.
	for (const platform of PLATFORMS) {
		const dep = `@shuv1337/shuvgrok-${platform}`;
		const pinned = (pkg.optionalDependencies || {})[dep];
		if (!pinned) {
			failures.push(`npm meta package is missing optionalDependency ${dep}`);
		} else if (pinned !== pkg.version) {
			failures.push(`${dep} pinned at ${pinned} but meta package is ${pkg.version} — versions must be lockstep`);
		}
	}
}

for (const platform of PLATFORMS) {
	const rel = `${NPM_DIR}/shuvgrok-${platform}/package.json`;
	const body = read(rel);
	checks.push(rel);
	if (!body) {
		failures.push(`${rel}: missing platform package`);
		continue;
	}
	const name = JSON.parse(body).name;
	if (name !== `@shuv1337/shuvgrok-${platform}`) {
		failures.push(`${rel}: name is ${name}, expected @shuv1337/shuvgrok-${platform}`);
	}
}

// The updater must never be able to name upstream's package, even disabled.
mustNotContain(
	"crates/codegen/xai-grok-update/src/version.rs",
	'"@xai-official/grok"',
	"a live reference here lets an upgrade path replace the fork with upstream",
);

// ── 2. Deliberate behavioral deltas ─────────────────────────────────────────

mustContain(
	"crates/codegen/xai-grok-update/src/lib.rs",
	"SELF_UPDATE_ENABLED: bool = false",
	"the upstream updater overwrites the fork binary; see FORK.md",
);

// ── 3. Compatibility surfaces: must still be upstream's ─────────────────────
//
// Each of these is a name something OUTSIDE this repo depends on. Renaming any
// of them is the classic over-eager-fork mistake, so they are asserted
// present, not merely left alone.

mustContain(
	"crates/codegen/xai-grok-pager/npm/shuvgrok/bin/postinstall.js",
	".grok",
	"the installer must keep using ~/.grok so existing credentials and config survive",
);

mustContain(
	"crates/codegen/xai-grok-shell/src/auth/anthropic/wire.rs",
	'AUTH_SCOPE: &str = "anthropic::oauth"',
	"auth.json scope keys; renaming signs every user out",
);
mustContain(
	"crates/codegen/xai-grok-shell/src/auth/openai_codex/wire.rs",
	'AUTH_SCOPE: &str = "openai-codex::oauth"',
	"auth.json scope keys; renaming signs every user out",
);

mustContain(
	"crates/codegen/xai-grok-shell/src/extensions/auth.rs",
	'"x.ai/auth/info"',
	"ACP method names are a wire protocol shared with clients outside this repo",
);

// ── 4. Provenance ───────────────────────────────────────────────────────────

const license = read("LICENSE") ?? read("LICENSE.md") ?? read("LICENSE.txt");
checks.push("LICENSE");
if (!license) {
	failures.push("LICENSE: missing — upstream is Apache-2.0 and the license must be preserved");
} else if (!/Apache License/i.test(license)) {
	failures.push("LICENSE: does not look like the upstream Apache-2.0 license");
}

mustContain("FORK.md", "xai-org/grok-build", "the fork must name its upstream");

// ── Report ──────────────────────────────────────────────────────────────────

if (failures.length > 0) {
	console.error(`fork boundary: ${failures.length} violation(s)\n`);
	for (const f of failures) console.error(`  ✗ ${f}`);
	console.error(`\nSee FORK.md for what each class of name means.`);
	process.exit(1);
}

console.log(`fork boundary intact (${new Set(checks).size} files checked)`);
