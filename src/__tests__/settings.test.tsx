import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Settings from '../pages/Settings';
import { api } from '../bridge/invoke';
import { getVersion } from '@tauri-apps/api/app';
import { save } from '@tauri-apps/plugin-dialog';

vi.mock('@tauri-apps/api/app', () => ({
  getVersion: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-shell', () => ({
  open: vi.fn(),
}));

vi.mock('../bridge/invoke', () => ({
  api: {
    logGetTail: vi.fn(),
    logGetSessionId: vi.fn(),
    saveSupportLog: vi.fn(),
    clearAppCache: vi.fn(),
  },
}));

// Mock fetch for update checker
const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);

describe('Settings', () => {
  beforeEach(() => {
    vi.mocked(getVersion).mockResolvedValue('4.0.0');
    vi.mocked(save).mockResolvedValue(null);
    vi.mocked(api.logGetSessionId).mockResolvedValue('session-123');
    vi.mocked(api.logGetTail).mockResolvedValue('line one\nline two');
    vi.mocked(api.saveSupportLog).mockResolvedValue(undefined);
    vi.mocked(api.clearAppCache).mockResolvedValue(undefined);

    mockFetch.mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({
        tag_name: 'v4.0.0',
        html_url: 'https://github.com/redpersongpt/OpCore-OneClick/releases/tag/v4.0.0',
        body: 'Release notes',
        published_at: '2026-01-01T00:00:00Z',
      }),
    });
  });

  it('loads version and log tail when opened', async () => {
    render(<Settings open onClose={() => {}} />);

    await screen.findByText('Session session-123');
    await waitFor(() => {
      expect(screen.getByText((content) => content.includes('line one') && content.includes('line two'))).toBeInTheDocument();
    });
    expect(screen.getByText('v4.0.0')).toBeInTheDocument();
    expect(api.logGetTail).toHaveBeenCalledWith(200);
  });

  it('exports diagnostics and clears cache through the bridge', async () => {
    vi.mocked(save).mockResolvedValue('/tmp/opcore-support.log');

    render(<Settings open onClose={() => {}} />);
    await screen.findByText('Session session-123');

    fireEvent.click(screen.getByRole('button', { name: 'Export Diagnostics' }));
    await waitFor(() => {
      expect(api.saveSupportLog).toHaveBeenCalledWith('/tmp/opcore-support.log');
    });

    fireEvent.click(screen.getByRole('button', { name: 'Clear Cache' }));
    await waitFor(() => {
      expect(api.clearAppCache).toHaveBeenCalled();
    });
  });
});
