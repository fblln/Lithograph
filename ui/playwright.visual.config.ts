import { defineConfig, devices } from '@playwright/test'

const externalBaseUrl = process.env.LITHOGRAPH_VISUAL_BASE_URL

export default defineConfig({
  testDir: './visual-tests',
  timeout: 45_000,
  fullyParallel: false,
  reporter: [['list'], ['html', { outputFolder: 'playwright-report', open: 'never' }]],
  outputDir: 'test-results/visual',
  webServer: externalBaseUrl ? undefined : {
    command: 'cargo run --bin lithograph -- serve tests/golden/polyglot --assets ui/dist --port 4317',
    cwd: '..',
    url: 'http://127.0.0.1:4317',
    reuseExistingServer: true,
    timeout: 120_000,
  },
  use: {
    baseURL: externalBaseUrl ?? 'http://127.0.0.1:4317',
    viewport: { width: 1440, height: 900 },
    colorScheme: 'dark',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
    video: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
