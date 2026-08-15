/**
 * Parse `npm pack --json` output for a single package.
 *
 * npm <= 11 emits an array of pack results. npm >= 12 emits an object keyed by
 * package name. Both shapes describe exactly one package here, so normalize to
 * the single result.
 */
export function parsePackResult(output) {
	const parsed = JSON.parse(output);
	const results = Array.isArray(parsed) ? parsed : Object.values(parsed);
	if (results.length !== 1) {
		throw new Error(`Expected one npm pack result, received ${results.length}`);
	}
	return results[0];
}
