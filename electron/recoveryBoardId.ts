/**
 * Apple Recovery board-ID lookup by macOS version string.
 *
 * The board IDs correspond to real Mac models whose firmware is trusted
 * by Apple's recovery download servers to receive the correct recovery image.
 */

export const RECOVERY_BOARD_IDS: Record<string, { boardId: string; mlb: string }> = {
  '16':    { boardId: 'Mac-827FAC58A8FDFA22', mlb: '00000000000000000' },
  '15':    { boardId: 'Mac-827FAC58A8FDFA22', mlb: '00000000000000000' },
  '14':    { boardId: 'Mac-827FAC58A8FDFA22', mlb: '00000000000000000' },
  '13':    { boardId: 'Mac-4B682C642B45593E', mlb: '00000000000000000' },
  '12':    { boardId: 'Mac-FFE5EF870D7BA81A', mlb: '00000000000000000' },
  '11':    { boardId: 'Mac-42FD25EABCABB274', mlb: '00000000000000000' },
  '10.15': { boardId: 'Mac-00BE6ED71E35EB86', mlb: '00000000000000000' },
  '10.14': { boardId: 'Mac-7BA5B2D9BE2258A1', mlb: 'F4K10270Q2J3WLVAD' },
  '10.13': { boardId: 'Mac-BE088AF8C5EB4FA2', mlb: 'F17M0XA0H7G3F91AD' },
};

/**
 * Resolve the recovery board-ID key from a macOS version string.
 *
 * Strategy: extract the version number from the string, then match
 * against keys with exact prefix comparison. This avoids all substring
 * collision issues (e.g., '10.15' vs '15', '10.14' vs '14').
 */
export function resolveRecoveryVersionKey(macOSVersion: string): string {
  // Extract the numeric version from strings like "macOS Sequoia 15", "macOS Catalina 10.15"
  const versionMatch = macOSVersion.match(/(\d+(?:\.\d+)?)\s*$/);
  const extractedVersion = versionMatch?.[1] ?? '';

  // Try exact match first (e.g., "10.15" -> "10.15", "15" -> "15")
  if (extractedVersion && RECOVERY_BOARD_IDS[extractedVersion]) {
    return extractedVersion;
  }

  // Try major-only match (e.g., "15.7" -> "15", "13.4" -> "13")
  const major = extractedVersion.split('.')[0];
  if (major && RECOVERY_BOARD_IDS[major]) {
    return major;
  }

  // Fallback: scan keys for substring match, longest-first
  const sortedKeys = Object.keys(RECOVERY_BOARD_IDS).sort(
    (a, b) => b.length - a.length || b.localeCompare(a),
  );
  for (const key of sortedKeys) {
    if (macOSVersion.includes(key)) return key;
  }

  return '15'; // default to latest stable
}

/**
 * Get the board ID and MLB for a given macOS version string.
 */
export function getRecoveryBoardId(macOSVersion: string): { boardId: string; mlb: string } {
  const key = resolveRecoveryVersionKey(macOSVersion);
  return RECOVERY_BOARD_IDS[key];
}
