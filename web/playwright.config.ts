import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  // The scenario is one story told in order, on one shared stack.
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:4173',
    locale: 'fr-FR',
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'node e2e/stack.mjs',
    url: 'http://127.0.0.1:4173/api/health',
    reuseExistingServer: false,
    // Includes compiling the Rust binaries on a cold cache.
    timeout: 900_000,
    stdout: 'pipe',
  },
});
