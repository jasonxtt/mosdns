import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const assetVersion = process.env.MOSDNS_ASSET_VERSION || new Date().toISOString().replace(/[-:TZ.]/g, '').slice(0, 14)
const outDir = path.resolve(__dirname, '../coremain/www/assets/vue-log')
const rootHtmlPath = path.resolve(__dirname, '../coremain/www/log.html')

export default defineConfig({
  plugins: [
    vue(),
    {
      name: 'mosdns-asset-version-stamp',
      closeBundle() {
        const indexPath = path.join(outDir, 'index.html')
        if (!fs.existsSync(indexPath)) {
          return
        }
        const html = fs.readFileSync(indexPath, 'utf8')
        const stamped = html
          .replace('/app.js', `/app.js?v=${assetVersion}`)
          .replace('/app.css', `/app.css?v=${assetVersion}`)
        if (stamped !== html) {
          fs.writeFileSync(indexPath, stamped, 'utf8')
        }

        if (!fs.existsSync(rootHtmlPath)) {
          return
        }
        const rootHtml = fs.readFileSync(rootHtmlPath, 'utf8')
        const stampedRootHtml = rootHtml
          .replace(/\/assets\/vue-log\/app\.js\?v=[^"']+/g, `/assets/vue-log/app.js?v=${assetVersion}`)
          .replace(/\/assets\/vue-log\/app\.css\?v=[^"']+/g, `/assets/vue-log/app.css?v=${assetVersion}`)
        if (stampedRootHtml !== rootHtml) {
          fs.writeFileSync(rootHtmlPath, stampedRootHtml, 'utf8')
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
