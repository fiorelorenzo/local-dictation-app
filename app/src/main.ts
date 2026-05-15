// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Lorenzo Fiore

import { app, BrowserWindow, Tray, Menu, nativeImage } from 'electron';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { SidecarSupervisor, type SidecarState } from './sidecar';

let tray: Tray | null = null;
let mainWindow: BrowserWindow | null = null;
let supervisor: SidecarSupervisor | null = null;

function resolveSidecarBinary(): string {
  // dev: SIDECAR_BIN env var overrides everything
  const fromEnv = process.env.SIDECAR_BIN;
  if (fromEnv && existsSync(fromEnv)) return fromEnv;

  if (app.isPackaged) {
    // packaged: <App>.app/Contents/Resources/inference-core
    return join(process.resourcesPath, 'inference-core');
  }

  // dev fallback: cargo target dir at workspace root
  return join(__dirname, '..', '..', '..', '..', 'target', 'debug', 'inference-core');
}

function formatState(state: SidecarState): string {
  switch (state.kind) {
    case 'idle': return 'Sidecar: idle';
    case 'starting': return 'Sidecar: starting...';
    case 'connected': return `Sidecar: connected, v${state.version}`;
    case 'failed': return `Sidecar: failed (${state.reason})`;
  }
}

function createTray(): void {
  // 16x16 transparent placeholder until we have a real icon (M0.5)
  const icon = nativeImage.createEmpty();
  tray = new Tray(icon);
  tray.setTitle('LDA');
  rebuildTrayMenu({ kind: 'idle' });
}

function rebuildTrayMenu(state: SidecarState): void {
  if (!tray) return;
  const menu = Menu.buildFromTemplate([
    { label: formatState(state), enabled: false },
    { type: 'separator' },
    { label: 'Quit', click: () => app.quit() },
  ]);
  tray.setContextMenu(menu);
  tray.setTitle(state.kind === 'connected' ? 'LDA ✓' : 'LDA');
}

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 480,
    height: 320,
    show: true,
    webPreferences: {
      preload: join(__dirname, '..', 'preload', 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  if (MAIN_WINDOW_VITE_DEV_SERVER_URL) {
    mainWindow.loadURL(MAIN_WINDOW_VITE_DEV_SERVER_URL);
  } else {
    mainWindow.loadFile(join(__dirname, '..', 'renderer', MAIN_WINDOW_VITE_NAME, 'index.html'));
  }
}

app.whenReady().then(async () => {
  createTray();
  createWindow();

  const socketPath = join(app.getPath('userData'), 'sidecar.sock');
  supervisor = new SidecarSupervisor({
    binaryPath: resolveSidecarBinary(),
    socketPath,
    onStateChange: (state) => {
      rebuildTrayMenu(state);
      mainWindow?.webContents.send('sidecar:state', state);
    },
  });
  await supervisor.start();
});

app.on('before-quit', async (e) => {
  if (supervisor) {
    e.preventDefault();
    const sv = supervisor;
    supervisor = null;
    await sv.shutdown();
    app.quit();
  }
});

app.on('window-all-closed', () => {
  // keep app running in menu bar
});
