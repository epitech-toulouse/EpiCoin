// Reading a list of students from a CSV chosen by a staff member.

const EMAIL = /^[^\s@,;"]+@[^\s@,;"]+\.[^\s@,;"]+$/;

/**
 * Extracts every email address found in a CSV or plain text file, whatever its columns and
 * separator. Addresses are lowercased and deduplicated, in order of appearance.
 */
export function extractEmails(text: string): string[] {
  const seen = new Set<string>();
  for (const cell of text.split(/[\n\r,;\t]+/)) {
    const candidate = cell.trim().replace(/^"+|"+$/g, '').trim().toLowerCase();
    if (EMAIL.test(candidate)) seen.add(candidate);
  }
  return [...seen];
}
