/**
 * The About dialog's facts that are not the engine's (docs/16 §Settings;
 * wave 5 R1): the repository the links point at, the wording for a build
 * the engine did not report, and how long "copied" stays. Kept out of the
 * component file so the dialog module exports components alone.
 */

/** The public repository — the About links, and the release notes under it. */
export const REPOSITORY_URL = "https://github.com/bencbartlett/Cicada";

/** What a field reads when the engine's `hello` carried no build (before R1). */
export const NOT_REPORTED = "not reported by this engine (before 0.1.0-alpha.1)";

/** How long the "copied" note stays beside the commit. */
export const COPIED_MS = 1500;

/**
 * This version's release notes: the tag's release page (`v<semver>`), or the
 * releases list when the engine reported no version.
 */
export function releaseNotesUrl(semver: string | null): string {
  return semver === null ? `${REPOSITORY_URL}/releases` : `${REPOSITORY_URL}/releases/tag/v${semver}`;
}
