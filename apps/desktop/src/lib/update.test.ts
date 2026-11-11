import { describe, expect, it } from 'vitest';
import type { UpdateState } from '@cloudit/api-types';
import { checkOnEntry, downloadPercent, requiredUpdate, updateRequired } from './update';

const state = (over: Partial<UpdateState> = {}): UpdateState => ({
  phase: 'idle',
  install: 'in_app',
  current_version: '0.1.0',
  latest_version: null,
  notes: null,
  release_url: null,
  downloaded: 0,
  total: null,
  error: null,
  checked_at: null,
  ...over
});

describe('updateRequired', () => {
  it('replaces the shell only when the app is too old', () => {
    expect(updateRequired({ status: 'ok' })).toBeNull();
    expect(
      updateRequired({
        status: 'server_outdated',
        server_version: '0.1.0',
        api_level: 1,
        required_api_level: 2
      })
    ).toBeNull();
    expect(
      updateRequired({ status: 'app_outdated', app_version: '0.1.0', min_client_version: '0.2.0' })
    ).toEqual({ appVersion: '0.1.0', minVersion: '0.2.0' });
  });
});

describe('requiredUpdate', () => {
  it('offers the install that fits the build', () => {
    const available = { phase: 'available', latest_version: '0.2.0' } as const;
    expect(requiredUpdate(state(available))).toEqual({ kind: 'install', version: '0.2.0' });
    expect(requiredUpdate(state({ ...available, install: 'release_page' }))).toEqual({
      kind: 'download_page',
      version: '0.2.0'
    });
    expect(requiredUpdate(state({ ...available, install: 'never' }))).toEqual({
      kind: 'dev_build',
      version: '0.2.0'
    });
  });

  it('follows the install under way', () => {
    expect(
      requiredUpdate(
        state({ phase: 'downloading', latest_version: '0.2.0', downloaded: 50, total: 200 })
      )
    ).toEqual({ kind: 'downloading', version: '0.2.0', percent: 25 });
    expect(requiredUpdate(state({ phase: 'waiting', latest_version: '0.2.0' }))).toEqual({
      kind: 'waiting',
      version: '0.2.0',
      percent: null
    });
    expect(requiredUpdate(state({ phase: 'installing', latest_version: '0.2.0' })).kind).toBe(
      'installing'
    );
    expect(requiredUpdate(state({ phase: 'failed', latest_version: '0.2.0' }))).toEqual({
      kind: 'failed',
      version: '0.2.0'
    });
  });

  it('tells a failed check from no version found', () => {
    expect(requiredUpdate(null)).toEqual({ kind: 'none', checkFailed: false });
    expect(requiredUpdate(state({ phase: 'checking' }))).toEqual({ kind: 'checking' });
    expect(requiredUpdate(state({ phase: 'up_to_date' }))).toEqual({
      kind: 'none',
      checkFailed: false
    });
    expect(requiredUpdate(state({ phase: 'failed', error: 'offline' }))).toEqual({
      kind: 'none',
      checkFailed: true
    });
  });
});

describe('checkOnEntry', () => {
  it('checks only when no version is known and no check runs', () => {
    expect(checkOnEntry(null)).toBe(true);
    expect(checkOnEntry(state())).toBe(true);
    expect(checkOnEntry(state({ phase: 'up_to_date' }))).toBe(true);
    expect(checkOnEntry(state({ phase: 'checking' }))).toBe(false);
    expect(checkOnEntry(state({ phase: 'available', latest_version: '0.2.0' }))).toBe(false);
  });
});

describe('downloadPercent', () => {
  it('is capped and unknown without a total', () => {
    expect(downloadPercent(state({ downloaded: 10 }))).toBeNull();
    expect(downloadPercent(state({ downloaded: 300, total: 200 }))).toBe(100);
  });
});
