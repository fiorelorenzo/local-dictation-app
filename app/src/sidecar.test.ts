import { describe, it, expect, beforeEach } from 'vitest';
import { SidecarSupervisor, type SidecarState } from './sidecar';

describe('SidecarSupervisor', () => {
  let states: SidecarState[];
  let supervisor: SidecarSupervisor;

  beforeEach(() => {
    states = [];
    supervisor = new SidecarSupervisor({
      binaryPath: '/nonexistent/path/inference-core',
      socketPath: '/tmp/test-sidecar.sock',
      onStateChange: (s) => states.push(s),
    });
  });

  it('starts in idle state', () => {
    expect(supervisor.state).toEqual({ kind: 'idle' });
  });

  it('emits failed when the binary does not exist', async () => {
    await supervisor.start();
    // wait briefly for state to settle
    await new Promise((r) => setTimeout(r, 50));
    const lastState = states[states.length - 1];
    expect(lastState?.kind).toBe('failed');
    if (lastState?.kind === 'failed') {
      expect(lastState.reason.toLowerCase()).toContain('not');
    }
  });

  it('shutdown transitions back to idle', async () => {
    await supervisor.shutdown();
    expect(supervisor.state).toEqual({ kind: 'idle' });
  });
});
