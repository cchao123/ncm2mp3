import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // Tauri 会接管控制台输出，清屏会把 Rust 侧的报错冲掉
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // 不监听 Rust 侧，否则 cargo 的增量产物会不断触发前端重载
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
