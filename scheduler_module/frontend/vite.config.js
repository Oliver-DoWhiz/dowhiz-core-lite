import { defineConfig } from "vite";
import basicSsl from "@vitejs/plugin-basic-ssl";

export default defineConfig({
  plugins: [basicSsl()],
  server: {
    https: true,
    proxy: {
      "/uploads": "http://127.0.0.1:9100",
      "/tasks": "http://127.0.0.1:9100",
      "/health": "http://127.0.0.1:9100",
    },
  },
  preview: {
    https: true,
  },
});
