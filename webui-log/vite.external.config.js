import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// The same `src/` as the built-in UI, built as a movable package for any `<config root>/ui/<name>/`
// (served at `/<name>/`). Resource URLs are relative (`base: './'`), so the directory works
// under any name; API calls are same-origin absolute paths and do not depend on it.
const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const version = process.env.UI_VERSION || 'dev'
const outDir = path.resolve(__dirname, 'dist-external/package')

export default defineConfig({
  base: './',
  plugins: [
    vue(),
    {
      name: 'mosdns-external-version-stamp',
      closeBundle() {
        const indexPath = path.join(outDir, 'index.html')
        if (!fs.existsSync(indexPath)) {
          return
        }
        const stamp = encodeURIComponent(version)
        const html = fs.readFileSync(indexPath, 'utf8')
        const stamped = html
          .replace(/\.\/app\.js(?:\?v=[^"']+)?/g, `./app.js?v=${stamp}`)
          .replace(/\.\/app\.css(?:\?v=[^"']+)?/g, `./app.css?v=${stamp}`)
        if (stamped !== html) {
          fs.writeFileSync(indexPath, stamped, 'utf8')
        }
      }
    }
  ],
  publicDir: false,
  build: {
    outDir,
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
