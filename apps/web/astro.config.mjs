import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

export default defineConfig({
  // Used to build absolute OG/canonical URLs.
  site: "https://vole.mosly.dev",
  vite: {
    plugins: [tailwindcss()],
  },
});
