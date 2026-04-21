import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

export default defineConfig({
  plugins: [vue()],
  publicDir: false,
  build: {
    outDir: path.resolve(__dirname, '../coremain/www/assets/vue-blog'),
    emptyOutDir: true,
    sourcemap: false,
    cssCodeSplit: false,
    rollupOptions: {
      input: path.resolve(__dirname, 'index.html'),
      output: {
        entryFileNames: 'app.js',
        chunkFileNames: 'chunks/[name]-[hash].js',
        assetFileNames: (assetInfo) => {
          if (assetInfo?.names?.some((name) => name.endsWith('.css'))) {
            return 'app.css'
          }
          return 'assets/[name]-[hash][extname]'
        }
      }
    }
  }
})
