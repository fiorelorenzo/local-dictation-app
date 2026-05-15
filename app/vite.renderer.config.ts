import { defineConfig } from 'vite';

export default defineConfig(async () => {
  const { svelte } = await import('@sveltejs/vite-plugin-svelte');
  return {
    plugins: [svelte()],
    build: {
      target: 'esnext',
    },
  };
});
