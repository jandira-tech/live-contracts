// Personal data removed from every exhibit on its way into D1 (data-subject requests
// under GDPR Art. 17/21), whatever re-push, reseed or replay later brings the
// original text back.
//
// The patterns are data, not code: they live in the `PII_REDACTIONS` Worker secret
// (a JSON array of regular-expression sources), so the personal data being removed
// never lands in this public repository. Each match is replaced with REPLACEMENT;
// names, roles and the rest of the contract stay. Patterns are global, not tied to an
// accession, so a later filing of the same contract is covered too.
//
//   wrangler secret put PII_REDACTIONS   # e.g. ["Examplestraße\\s*12(?!\\d)", "12345\\s*Exampletown"]

export const REPLACEMENT = '[address redacted]';

export type RedactionRules = readonly RegExp[];

/** Parse the secret. Throws on anything malformed: the caller must fail closed. */
export function compileRedactions(secret: string | undefined | null): RedactionRules {
  if (!secret || !secret.trim()) return [];
  try {
    const sources: unknown = JSON.parse(secret);
    if (!Array.isArray(sources) || !sources.every((s) => typeof s === 'string' && s.length > 0)) {
      throw new TypeError('not an array of non-empty strings');
    }
    return sources.map((s) => new RegExp(s, 'gu'));
  } catch (error) {
    // Never echo the secret: it holds the personal data being removed.
    throw new Error(`PII_REDACTIONS must be a JSON array of non-empty regex strings (${(error as Error).name})`);
  }
}

export function redactPersonalData<T extends string | null | undefined>(text: T, rules: RedactionRules): T {
  if (!text || rules.length === 0) return text;
  let out: string = text;
  for (const rule of rules) out = out.replace(rule, REPLACEMENT);
  return out as T;
}
