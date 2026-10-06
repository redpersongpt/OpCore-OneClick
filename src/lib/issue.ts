import { REPO_URL } from './external';

/** Maximum length of the diagnostics block embedded in an issue URL. */
export const MAX_DIAGNOSTICS = 1500;

/** Strip user names from home-directory paths and MAC addresses / serials from logs. */
export function redact(text: string): string {
  return text
    .replace(/([A-Za-z]:\\Users\\)[^\\\s"']+/gi, '$1<user>')
    .replace(/(\/home\/)[^/\s"']+/g, '$1<user>')
    .replace(/(\/Users\/)[^/\s"']+/g, '$1<user>')
    .replace(/\b([0-9A-F]{2}[:-]){5}[0-9A-F]{2}\b/gi, '<mac>')
    .replace(/\b(serial|mlb|systemserialnumber|systemuuid|rom)(["'\s:=]+)[A-Z0-9-]{6,}/gi, '$1$2<redacted>');
}

export function truncate(text: string, max = MAX_DIAGNOSTICS): string {
  if (text.length <= max) return text;
  return `${text.slice(0, max)}\n… (truncated)`;
}

export interface IssueInput {
  title: string;
  description: string;
  diagnostics: string;
}

export function buildIssueUrl(input: IssueInput): string {
  const body = [
    '## Description',
    input.description,
    '',
    '## Steps to reproduce',
    '1. ',
    '',
    '## Diagnostics',
    '```',
    truncate(redact(input.diagnostics)),
    '```',
  ].join('\n');
  const params = new URLSearchParams({ title: input.title, body, labels: 'bug' });
  return `${REPO_URL}/issues/new?${params.toString()}`;
}
