<script lang="ts">
  import type { SidecarStateMsg } from '../preload';

  let state = $state<SidecarStateMsg>({ kind: 'idle' });

  $effect(() => {
    const off = window.lda.onSidecarState((s) => { state = s; });
    return off;
  });
</script>

<main>
  <h1>local-dictation-app</h1>
  <p>
    {#if state.kind === 'idle'}
      Sidecar is idle.
    {:else if state.kind === 'starting'}
      Sidecar is starting...
    {:else if state.kind === 'connected'}
      Sidecar connected, version {state.version}
    {:else}
      Sidecar failed: {state.reason}
    {/if}
  </p>
</main>

<style>
  main {
    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
    padding: 24px;
  }
  h1 {
    font-size: 18px;
    margin: 0 0 12px 0;
  }
  p {
    color: #555;
  }
</style>
