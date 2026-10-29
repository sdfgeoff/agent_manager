// Run after `cargo build`: PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node tests/browser.mjs
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, copyFile, chmod, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import assert from 'node:assert/strict';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const dir = await mkdtemp(join(tmpdir(), 'agentmanager-browser-'));
const probe = createServer(); await new Promise(r => probe.listen(0, '127.0.0.1', r)); const port = probe.address().port; await new Promise(r => probe.close(r));
const origin = `http://127.0.0.1:${port}`;
const binaryDir = process.env.AGENTMANAGER_BIN_DIR || 'target/debug';
const serverBinary = resolve(binaryDir, 'server'); const workerBinary = resolve(binaryDir, 'worker');
const setup = spawnSync(serverBinary, ['--data-dir', join(dir, 'server'), 'set-password', '--stdin'], { input: 'browser test password\n', encoding: 'utf8' }); assert.equal(setup.status, 0, setup.stderr);
const server = spawn(serverBinary, ['--data-dir', join(dir, 'server'), '--listen', `127.0.0.1:${port}`, '--public-url', origin], { stdio: 'ignore' });
let worker; let browser;
const sleep = ms => new Promise(r => setTimeout(r, ms));
try {
  for (let i = 0; i < 100; i++) { try { if ((await fetch(origin)).ok) break; } catch {} await sleep(50); }
  const fake = join(dir, 'codex-test'); await copyFile('tests/fixtures/codex.py', fake); await chmod(fake, 0o700);
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM || '/usr/bin/chromium', headless: true, args: ['--no-sandbox'] });
  const mobile = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  const page = await mobile.newPage(); const errors = []; page.on('pageerror', e => errors.push(e.message));
  await page.goto(origin); await page.locator('#password').fill('browser test password'); await page.locator('#login-form button').click(); await page.locator('#app').waitFor({ state: 'visible' });
  await page.locator('#open-menu').click(); await page.locator('#add-worker').click(); await page.locator('#worker-dialog').waitFor({ state: 'visible' });
  const command = await page.locator('#worker-command').textContent(); const key = command.split('--key ')[1].trim();
  worker = spawn(workerBinary, ['--host', origin, '--key', key, '--name', 'Test laptop', '--credential-file', join(dir, 'worker.json'), '--codex', fake], { stdio: 'ignore' });
  await page.locator('#worker-dialog .close-dialog').click(); await page.waitForFunction(() => !document.getElementById('new-session').disabled);
  await page.locator('#new-session').click(); await page.locator('#new-name').fill('Remote workspace'); await page.locator('#new-model').fill('test-model');
  await page.locator('#browse').click(); await page.locator('#folder-dialog').waitFor({ state: 'visible' }); await page.locator('#folder-create').click();
  await page.locator('#input-value').fill(join(dir, 'brand-new-project')); await page.locator('#input-form button.primary').click(); await page.waitForFunction(path => document.getElementById('folder-path').value === path, join(dir, 'brand-new-project'));
  await page.locator('#folder-use').click(); await page.locator('#create').click(); await page.locator('#composer').waitFor({ state: 'visible' });
  assert.equal(await page.locator('#session-title').textContent(), 'Remote workspace'); assert((await stat(join(dir, 'brand-new-project'))).isDirectory());
  worker.kill('SIGTERM'); await new Promise(r => worker.once('exit', r));
  worker = spawn(workerBinary, ['--credential-file', join(dir, 'worker.json'), '--codex', fake], { stdio: 'ignore' });
  await page.waitForFunction(() => !document.getElementById('send').disabled);
  const desktop = await browser.newContext({ viewport: { width: 1440, height: 1000 } }); const page2 = await desktop.newPage(); page2.on('pageerror', e => errors.push(e.message));
  await page2.goto(origin); await page2.locator('#password').fill('browser test password'); await page2.locator('#login-form button').click(); await page2.locator('.session').first().click(); await page2.locator('#composer').waitFor({ state: 'visible' });
  await page.locator('#prompt').fill('Check the worker connection.'); await page.locator('#send').click();
  await page.getByRole('button', { name: 'Approve once', exact: true }).waitFor(); await page2.getByRole('button', { name: 'Approve once', exact: true }).waitFor();
  await page.reload(); await page.locator('#open-menu').click(); await page.locator('.session').first().click(); await page.getByRole('button', { name: 'Approve once', exact: true }).click();
  await page.getByText('Done. The worker is connected and ready.', { exact: true }).waitFor(); await page2.getByText('Done. The worker is connected and ready.', { exact: true }).waitFor();
  await page2.locator('#requests').filter({ has: page2.locator('button') }).waitFor({ state: 'hidden' });
  await page.screenshot({ path: join(dir, 'phone.png'), fullPage: true }); await page2.screenshot({ path: join(dir, 'desktop.png'), fullPage: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  assert.equal(await page2.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  assert.deepEqual(errors, []);
  console.log(`Browser tests passed: registration, folder creation, session configuration, two clients, approval after reload, live output, phone layout. Screenshots: ${dir}`);
} finally { await browser?.close(); worker?.kill('SIGTERM'); server.kill('SIGTERM'); }
