import { beforeEach, describe, expect, it } from 'vitest';
import { firstIncompleteIndex, STEPS, useWizard } from '../stores/wizard';

const wizard = () => useWizard.getState();

describe('wizard gating', () => {
  beforeEach(() => {
    wizard().reset();
  });

  it('starts at welcome with only the first step reachable', () => {
    expect(wizard().step).toBe('welcome');
    expect(wizard().canVisit('welcome')).toBe(true);
    expect(wizard().canVisit('scan')).toBe(false);
    expect(wizard().goTo('build')).toBe(false);
    expect(wizard().step).toBe('welcome');
  });

  it('unlocks steps in order as they are completed', () => {
    wizard().complete('welcome');
    expect(wizard().step).toBe('scan');
    wizard().complete('scan');
    wizard().complete('hardware');
    expect(wizard().step).toBe('compatibility');
    expect(wizard().canVisit('compatibility')).toBe(true);
    expect(wizard().canVisit('bios')).toBe(false);
    expect(wizard().goTo('welcome')).toBe(true);
    expect(wizard().goTo('compatibility')).toBe(true);
  });

  it('refuses to complete a step that is not reachable yet', () => {
    wizard().complete('build');
    expect(wizard().completed).toEqual([]);
    expect(wizard().step).toBe('welcome');
  });

  it('complete without advance keeps the current step', () => {
    wizard().complete('welcome', false);
    expect(wizard().step).toBe('welcome');
    expect(wizard().completed).toEqual(['welcome']);
  });

  it('invalidateFrom drops later completions and pulls the user back', () => {
    for (const s of ['welcome', 'scan', 'hardware', 'compatibility', 'bios', 'build', 'review'] as const) wizard().complete(s);
    expect(wizard().step).toBe('deploy');
    wizard().invalidateFrom('build');
    expect(wizard().completed).toEqual(['welcome', 'scan', 'hardware', 'compatibility', 'bios']);
    expect(wizard().step).toBe('build');
    expect(wizard().canVisit('review')).toBe(false);
  });

  it('invalidation after the current step keeps the user where they are', () => {
    for (const s of ['welcome', 'scan', 'hardware', 'compatibility', 'bios', 'build'] as const) wizard().complete(s);
    wizard().goTo('hardware');
    wizard().invalidateFrom('compatibility');
    expect(wizard().step).toBe('hardware');
    expect(wizard().completed).toEqual(['welcome', 'scan', 'hardware']);
  });

  it('locks block navigation but not the current step', () => {
    wizard().complete('welcome');
    wizard().complete('scan');
    wizard().lock('scan');
    expect(wizard().isLocked()).toBe(true);
    expect(wizard().goTo('welcome')).toBe(false);
    expect(wizard().goTo('hardware')).toBe(true);
    wizard().lock('scan');
    expect(wizard().locks).toEqual(['scan']);
    wizard().unlock('scan');
    expect(wizard().goTo('welcome')).toBe(true);
  });

  it('restore keeps only a contiguous prefix of completed steps', () => {
    wizard().restore('review', ['welcome', 'scan', 'bios']);
    expect(wizard().completed).toEqual(['welcome', 'scan']);
    expect(wizard().step).toBe('hardware');
  });

  it('firstIncompleteIndex clamps to the last step', () => {
    expect(firstIncompleteIndex([...STEPS])).toBe(STEPS.length - 1);
    expect(firstIncompleteIndex([])).toBe(0);
  });
});
