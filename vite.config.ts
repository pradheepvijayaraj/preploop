import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [tailwindcss(), sveltekit()],
  build: {
    target: "esnext",
    cssMinify: true,
    rolldownOptions: {
      // Keep dependency initialization side effects: Svelte's legacy-mode
      // flag is required by Bits UI's Switch input in production builds.
      output: {
        comments: false,
        minify: {
          compress: {
            dropConsole: true,
            dropDebugger: true,
          },
          mangle: true,
          codegen: {
            legalComments: "none",
          },
        },
      },
    },
  },
  server: {
    port: 5173,
    strictPort: true,
  },
});
