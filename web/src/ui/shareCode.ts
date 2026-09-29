// Client-side share-code *format* handling for the start screen.
//
// A share code is `LS` followed by exactly 26 Crockford base32 symbols (version,
// seed, sliders and a CRC-8; see crates/ls-worldgen/src/sharecode.rs). It prints
// as `LSab-cdef-ghij-klmn-opqr-stuv-wxyz` (34 characters): `LS` plus the first two
// symbols form the first group of four. Like ls-worldgen's decoder, this module
// ignores case, dashes and whitespace and reads O as 0 and I/L as 1. It only checks
// the format; decoding and checksum verification happen in ls-worldgen (via ls-wasm
// in wave 3, and on the server).

export const SHARE_CODE_PREFIX = "LS";
/** Number of base32 symbols after the prefix (ls-worldgen `CODE_SYMBOLS`). */
export const SHARE_CODE_SYMBOLS = 26;
/** Printed length: prefix + symbols + 6 dashes. */
export const SHARE_CODE_PRINTED_LEN = SHARE_CODE_PREFIX.length + SHARE_CODE_SYMBOLS + 6;

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

export type ShareCodeCheck = { ok: true; code: string } | { ok: false; error: string };

/**
 * Uppercases, removes whitespace and dashes and maps Crockford look-alikes
 * (O -> 0, I/L -> 1) after the `LS` prefix. The prefix is kept as is.
 */
export function normalizeShareCode(raw: string): string {
  let s = raw.toUpperCase().replace(/[\s-]+/g, "");
  let prefix = "";
  if (s.startsWith(SHARE_CODE_PREFIX)) {
    prefix = SHARE_CODE_PREFIX;
    s = s.slice(SHARE_CODE_PREFIX.length);
  }
  s = s.replace(/O/g, "0").replace(/[IL]/g, "1");
  return prefix + s;
}

/** Groups a normalised code the way ls-worldgen prints it (`LSab-cdef-…`). */
export function formatShareCode(normalized: string): string {
  const body = normalized.slice(SHARE_CODE_PREFIX.length);
  let out = SHARE_CODE_PREFIX;
  for (let i = 0; i < body.length; i++) {
    if (i >= 2 && (i - 2) % 4 === 0) out += "-";
    out += body[i];
  }
  return out;
}

/**
 * Validates the format and returns the code in printed form. An empty string is
 * not a code (callers treat it as "use the server's").
 */
export function checkShareCode(raw: string): ShareCodeCheck {
  const code = normalizeShareCode(raw);
  if (code.length === 0) return { ok: false, error: "Enter a share code." };
  if (!code.startsWith(SHARE_CODE_PREFIX)) {
    return { ok: false, error: `Share codes start with "${SHARE_CODE_PREFIX}".` };
  }
  const body = code.slice(SHARE_CODE_PREFIX.length);
  for (const ch of body) {
    if (!CROCKFORD.includes(ch)) {
      return { ok: false, error: `"${ch}" is not valid in a share code.` };
    }
  }
  if (body.length < SHARE_CODE_SYMBOLS) {
    return { ok: false, error: "The share code is incomplete." };
  }
  if (body.length > SHARE_CODE_SYMBOLS) {
    return { ok: false, error: "The share code is too long." };
  }
  return { ok: true, code: formatShareCode(code) };
}
