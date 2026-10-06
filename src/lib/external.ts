import { open } from '@tauri-apps/plugin-shell';

export const REPO_URL = 'https://github.com/redpersongpt/OpCore-OneClick';
export const RELEASES_URL = `${REPO_URL}/releases/latest`;

/** Open a URL in the system browser. Only http(s) links are allowed. */
export async function openExternal(url: string): Promise<void> {
  if (!/^https?:\/\//i.test(url)) return;
  try {
    await open(url);
  } catch {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
