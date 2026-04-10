import { defineConfig } from "vite";
import basicSsl from "@vitejs/plugin-basic-ssl";

export default defineConfig({
  plugins: [basicSsl()],
  server: {
    https: true,
    proxy: {
      "/tasks": "http://127.0.0.1:9100",
      "/health": "http://127.0.0.1:9100",
    },
  },
  preview: {
    https: true,
  },
});
