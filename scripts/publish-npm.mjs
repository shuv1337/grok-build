#!/usr/bin/env node
/**
 * Publish the ShuvGrok npm packages.
 *
 * Usage:
 *   node scripts/publish-npm.mjs [--dry-run] [--no-provenance]
 *
 * Publishes, in this order:
 *   1. the six platform packages @shuv1337/shuvgrok-<platform>-<arch>
 *   2. the meta package @shuv1337/shuvgrok
 *
 * The order is load-bearing: the meta package's optionalDependencies pin the
 * platform packages at an exact version, so publishing the meta package first
 * would leave a window where `npm i -g @shuv1337/shuvgrok` resolves to a
 * version whose binaries do not exist yet.
 *
 * Idempotent: a version already present on the registry is skipped (its
 * contents are still validated), so a re-run after a partial failure finishes
 * the release instead of erroring out.
 *
 * Expects the platform packages to have been assembled first:
 *   crates/codegen/xai-grok-pager/npm/shuvgrok/scripts/assemble-platform-packages.js
 */

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parsePackResult } from "./npm-pack.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const npmRoot = join(repoRoot, "crates", "codegen", "xai-grok-pager", "npm");
const metaDirectory = join(npmRoot, "shuvgrok");

const PLATFORM_KEYS = ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "win32-arm64", "win32-x64"];

const dryRun = process.argv.includes("--dry-run");
// Provenance is signed from CI's OIDC token, so it is unavailable for the
// one-time local bootstrap publish that has to happen before npm will accept a
// trusted-publisher config. See docs/RELEASING.md.
const noProvenance = process.argv.includes("--no-provenance");
const unknownArgs = process.argv
	.slice(2)
	.filter((arg) => arg !== "--dry-run" && arg !== "--no-provenance");

if (unknownArgs.length > 0) {
	console.error(`Usage: node scripts/publish-npm.mjs [--dry-run] [--no-provenance]`);
	process.exit(1);
}

function commandForPlatform(command) {
	return process.platform === "win32" ? `${command}.cmd` : command;
}

function run(command, args, options = {}) {
	console.log(`$ ${[command, ...args].join(" ")}`);
	const result = spawnSync(commandForPlatform(command), args, {
		cwd: options.cwd,
		encoding: "utf8",
		stdio: options.capture ? ["inherit", "pipe", "pipe"] : "inherit",
	});

	if (result.status !== 0) {
		const output = [result.stdout, result.stderr].filter(Boolean).join("\n");
		throw new Error(output ? `Command failed: ${command} ${args.join(" ")}\n${output}` : `Command failed: ${command} ${args.join(" ")}`);
	}

	return result;
}

function readPackage(directory) {
	const manifestPath = join(directory, "package.json");
	if (!existsSync(manifestPath)) {
		throw new Error(`Missing package.json in ${directory}`);
	}
	const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
	return { directory, name: manifest.name, version: manifest.version, manifest };
}

/** The compressed binary the assemble step drops into bin/ must be there. */
function assertPlatformPayloadExists(pkg) {
	const binDirectory = join(pkg.directory, "bin");
	const payloads = existsSync(binDirectory) ? readdirSync(binDirectory).filter((entry) => entry.endsWith(".br")) : [];

	if (payloads.length !== 1) {
		throw new Error(`${pkg.name}: expected exactly one brotli payload in ${binDirectory}, found ${payloads.length}. Run crates/codegen/xai-grok-pager/npm/shuvgrok/scripts/assemble-platform-packages.js first.`);
	}
	if (!existsSync(join(pkg.directory, "THIRD_PARTY_NOTICES.md"))) {
		throw new Error(`${pkg.name}: missing THIRD_PARTY_NOTICES.md. Run the assemble script first.`);
	}
}

function assertMetaPayloadExists(pkg) {
	for (const file of ["bin/shuvgrok", "bin/postinstall.js"]) {
		if (!existsSync(join(pkg.directory, file))) {
			throw new Error(`${pkg.name}: missing ${file}`);
		}
	}
}

function validatePack(directory) {
	const result = run("npm", ["pack", "--dry-run", "--ignore-scripts", "--json"], { capture: true, cwd: directory });
	const packed = parsePackResult(result.stdout);
	console.log(`  ${packed.filename}: ${packed.files.length} files, ${packed.size} bytes packed, ${packed.unpackedSize} bytes unpacked`);
}

function isPublished(name, version) {
	const result = spawnSync(commandForPlatform("npm"), ["view", `${name}@${version}`, "version", "--json"], {
		encoding: "utf8",
		stdio: ["inherit", "pipe", "pipe"],
	});

	if (result.status === 0 && result.stdout.trim()) {
		return true;
	}

	const output = [result.stdout, result.stderr].filter(Boolean).join("\n");
	if (result.status !== 0 && (output.includes("E404") || output.includes("404 Not Found"))) {
		return false;
	}

	throw new Error(output ? `Failed to query ${name}@${version}\n${output}` : `Failed to query ${name}@${version}`);
}

// Platform packages first, meta package last.
const platformPackages = PLATFORM_KEYS.map((key) => readPackage(join(npmRoot, `shuvgrok-${key}`)));
const metaPackage = readPackage(metaDirectory);
const packages = [...platformPackages, metaPackage];

const versions = [...new Set(packages.map((pkg) => pkg.version))];
if (versions.length !== 1) {
	throw new Error(`Publish packages are not lockstep versioned: ${packages.map((pkg) => `${pkg.name}@${pkg.version}`).join(", ")}`);
}
const version = versions[0];

// The meta package resolves its binaries through exact-version
// optionalDependencies; a drifted pin ships an uninstallable release.
for (const key of PLATFORM_KEYS) {
	const name = `@shuv1337/shuvgrok-${key}`;
	const pinned = metaPackage.manifest.optionalDependencies?.[name];
	if (pinned !== version) {
		throw new Error(`${metaPackage.name} optionalDependencies["${name}"] is ${pinned ?? "missing"}, expected ${version}`);
	}
}

console.log(`Publishing ShuvGrok packages at ${version}${dryRun ? " (dry run)" : ""}\n`);

const packageStates = packages.map((pkg) => ({ ...pkg, published: false }));

for (const pkg of packageStates) {
	if (pkg === packageStates.at(-1)) {
		assertMetaPayloadExists(pkg);
	} else {
		assertPlatformPayloadExists(pkg);
	}

	pkg.published = isPublished(pkg.name, pkg.version);

	if (pkg.published) {
		console.log(`${pkg.name}@${pkg.version} is already published; validating package contents only.`);
	} else {
		console.log(`${pkg.name}@${pkg.version} is not published; validating package contents before publish.`);
	}
	validatePack(pkg.directory);
	console.log();
}

if (dryRun) {
	process.exit(0);
}

console.log("All packages validated; starting publication.\n");

for (const pkg of packageStates) {
	if (pkg.published) {
		console.log(`Skipping ${pkg.name}@${pkg.version}: already published\n`);
		continue;
	}

	const publishArgs = ["publish", "--access", "public", "--ignore-scripts"];
	if (!noProvenance) publishArgs.splice(3, 0, "--provenance");
	run("npm", publishArgs, { cwd: pkg.directory });
	console.log();
}

console.log(`Published ShuvGrok ${version}: install with npm i -g @shuv1337/shuvgrok@${version}`);
