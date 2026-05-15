import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron';

export type SidecarStateMsg =
  | { kind: 'idle' }
  | { kind: 'starting' }
  | { kind: 'connected'; version: string }
  | { kind: 'failed'; reason: string };

const api = {
  onSidecarState(callback: (state: SidecarStateMsg) => void): () => void {
    const handler = (_e: IpcRendererEvent, state: SidecarStateMsg) => callback(state);
    ipcRenderer.on('sidecar:state', handler);
    return () => ipcRenderer.off('sidecar:state', handler);
  },
};

contextBridge.exposeInMainWorld('lda', api);

export type LdaAPI = typeof api;

declare global {
  interface Window {
    lda: LdaAPI;
  }
}
