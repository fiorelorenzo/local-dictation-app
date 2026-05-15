import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, unlinkSync } from 'node:fs';
import { request } from 'node:http';

export type SidecarState =
  | { kind: 'idle' }
  | { kind: 'starting' }
  | { kind: 'connected'; version: string }
  | { kind: 'failed'; reason: string };

export interface SidecarSupervisorOptions {
  binaryPath: string;
  socketPath: string;
  onStateChange?: (state: SidecarState) => void;
  healthPollIntervalMs?: number;
  healthPollMaxAttempts?: number;
}

const DEFAULT_POLL_INTERVAL_MS = 100;
const DEFAULT_POLL_MAX_ATTEMPTS = 30;

export class SidecarSupervisor {
  private child: ChildProcess | null = null;
  private _state: SidecarState = { kind: 'idle' };

  constructor(private readonly opts: SidecarSupervisorOptions) {}

  get state(): SidecarState {
    return this._state;
  }

  private setState(next: SidecarState): void {
    this._state = next;
    this.opts.onStateChange?.(next);
  }

  async start(): Promise<void> {
    if (this._state.kind !== 'idle' && this._state.kind !== 'failed') {
      return;
    }
    this.setState({ kind: 'starting' });

    if (!existsSync(this.opts.binaryPath)) {
      this.setState({ kind: 'failed', reason: `sidecar binary not found at ${this.opts.binaryPath}` });
      return;
    }

    try {
      this.child = spawn(this.opts.binaryPath, [], {
        env: {
          ...process.env,
          SIDECAR_SOCKET_PATH: this.opts.socketPath,
          SIDECAR_LOG_LEVEL: 'info',
          RUST_BACKTRACE: '1',
        },
        stdio: ['ignore', 'pipe', 'pipe'],
      });

      this.child.on('exit', (code, signal) => {
        if (this._state.kind === 'connected' || this._state.kind === 'starting') {
          this.setState({ kind: 'failed', reason: `sidecar exited (code=${code} signal=${signal})` });
        }
      });

      const version = await this.pollHealthz();
      this.setState({ kind: 'connected', version });
    } catch (e) {
      this.setState({ kind: 'failed', reason: e instanceof Error ? e.message : String(e) });
    }
  }

  private async pollHealthz(): Promise<string> {
    const interval = this.opts.healthPollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
    const maxAttempts = this.opts.healthPollMaxAttempts ?? DEFAULT_POLL_MAX_ATTEMPTS;

    for (let attempt = 0; attempt < maxAttempts; attempt++) {
      try {
        const body = await this.healthzRequest();
        const parsed = JSON.parse(body) as { status?: string; version?: string };
        if (parsed.status === 'ok' && typeof parsed.version === 'string') {
          return parsed.version;
        }
      } catch {
        // socket not ready yet
      }
      await new Promise((r) => setTimeout(r, interval));
    }
    throw new Error(`sidecar /healthz did not respond within ${interval * maxAttempts}ms`);
  }

  private healthzRequest(): Promise<string> {
    return new Promise((resolve, reject) => {
      const req = request(
        {
          socketPath: this.opts.socketPath,
          path: '/healthz',
          method: 'GET',
          timeout: 1000,
        },
        (res) => {
          const chunks: Buffer[] = [];
          res.on('data', (chunk) => chunks.push(chunk));
          res.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
        }
      );
      req.on('error', reject);
      req.on('timeout', () => {
        req.destroy(new Error('timeout'));
      });
      req.end();
    });
  }

  async shutdown(): Promise<void> {
    if (this.child && !this.child.killed) {
      this.child.kill('SIGTERM');
      const exited = await new Promise<boolean>((resolve) => {
        const timer = setTimeout(() => resolve(false), 2000);
        this.child?.once('exit', () => {
          clearTimeout(timer);
          resolve(true);
        });
      });
      if (!exited) {
        this.child.kill('SIGKILL');
      }
    }
    this.child = null;
    if (existsSync(this.opts.socketPath)) {
      try {
        unlinkSync(this.opts.socketPath);
      } catch {
        // ignore
      }
    }
    this.setState({ kind: 'idle' });
  }
}
