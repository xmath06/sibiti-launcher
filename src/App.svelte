<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { openUrl } from '@tauri-apps/plugin-opener';

  let backendPath = $state('');
  let frontendPath = $state('');
  let status = $state({
    is_postgres_installed: false,
    is_postgres_running: false,
    is_pm2_installed: false,
    are_deps_installed: false,
    is_frontend_built: false
  });
  let running = $state({ frontend: false, backend: false });
  let installing = $state(false);
  let installingDb = $state(false);
  let logs = $state([]);
  let error = $state('');
  let tauriReady = $state(true);

  const ready = $derived(
    status.is_postgres_running &&
      status.is_pm2_installed &&
      status.are_deps_installed &&
      status.is_frontend_built
  );
  const isRunning = $derived(running.frontend && running.backend);

  async function refresh() {
    try {
      const d = await invoke('discover');
      backendPath = d.backend || '';
      frontendPath = d.frontend || '';
      if (backendPath && frontendPath) {
        status = await invoke('check-system-status', {
          backend: backendPath,
          frontend: frontendPath
        });
      }
      running = await invoke('check-running');
    } catch (e) {
      error = String(e);
    }
  }

  onMount(async () => {
    const hasTauri =
      typeof window !== 'undefined' &&
      ('__TAURI_INTERNALS__' in window || '__TAURI__' in window);

    if (!hasTauri) {
      // Preview di browser biasa — bridge Tauri tidak ada.
      tauriReady = false;
      error =
        'Mode preview (browser). Jalankan "bun run tauri dev" untuk mengaktifkan perintah launcher.';
      return;
    }

    try {
      const unlisten = await listen('install-log', (e) => {
        logs = [...logs, e.payload];
      });
      await refresh();
      return () => unlisten();
    } catch (e) {
      tauriReady = false;
      error = 'Gagal memanggil Tauri: ' + String(e);
    }
  });

  async function install() {
    if (!backendPath || !frontendPath) {
      error = 'Path backend/frontend belum terdeteksi.';
      return;
    }
    installing = true;
    logs = [];
    error = '';
    try {
      await invoke('run-installer', {
        backend: backendPath,
        frontend: frontendPath
      });
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      installing = false;
    }
  }

  async function installDatabase() {
    if (!backendPath) {
      error = 'Path backend belum terdeteksi.';
      return;
    }
    installingDb = true;
    logs = [];
    error = '';
    try {
      await invoke('install-database', { backend: backendPath });
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      installingDb = false;
    }
  }

  async function start() {
    error = '';
    try {
      await invoke('start-services', {
        backend: backendPath,
        frontend: frontendPath
      });
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  async function stop() {
    error = '';
    try {
      await invoke('stop-services');
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  async function openApp() {
    try {
      await openUrl('http://127.0.0.1:5173');
    } catch {
      error = 'Buka manual: http://127.0.0.1:5173';
    }
  }
  async function openSwagger() {
    try {
      await openUrl('http://127.0.0.1:3000/api/v1/docs');
    } catch {
      error = 'Buka manual: http://127.0.0.1:3000/api/v1/docs';
    }
  }

  const checks = $derived([
    { label: 'PostgreSQL Database', ok: status.is_postgres_installed && status.is_postgres_running },
    { label: 'PM2 (process manager)', ok: status.is_pm2_installed },
    { label: 'Node Modules (deps)', ok: status.are_deps_installed },
    { label: 'Build Bundle Frontend', ok: status.is_frontend_built }
  ]);
</script>

<main class="wrap">
  <header class="head">
    <div>
      <h1>CBT Desktop Launcher</h1>
      <p>Kelola backend &amp; frontend ujian CBT di Windows</p>
    </div>
    <span class="badge" class:on={isRunning} class:warn={!ready && !isRunning}>
      {isRunning ? 'RUNNING' : ready ? 'READY' : 'NOT READY'}
    </span>
  </header>

  <section class="grid">
    {#each checks as c}
      <div class="card">
        <span class="dot" class:ok={c.ok} class:no={!c.ok}></span>
        <span class="cl">{c.label}</span>
        <span class="st">{c.ok ? 'TERPASANG' : 'BELUM'}</span>
      </div>
    {/each}
  </section>

  <section class="paths">
    <div><span class="k">Detected Backend:</span> <code>{backendPath || '— tidak terdeteksi —'}</code></div>
    <div><span class="k">Detected Frontend:</span> <code>{frontendPath || '— tidak terdeteksi —'}</code></div>
  </section>

  {#if error}
    <div class="err">{error}</div>
  {/if}

  <section class="actions">
    <button class="btn ghost" onclick={install} disabled={installing}>
      {installing ? 'Menginstal…' : ready ? '⟳ Re-Install / Repair' : '⚙ INSTALL LAUNCHER'}
    </button>

    <button class="btn" onclick={installDatabase} disabled={installingDb || installing}>
      {installingDb
        ? 'Memasang Database…'
        : status.is_postgres_running
          ? '⟳ Repair Database'
          : '🐘 INSTALL DATABASE'}
    </button>

    {#if !isRunning}
      <button class="btn green" onclick={start} disabled={!ready || installing}>
        ▶ START SERVICES
      </button>
    {:else}
      <button class="btn red" onclick={stop} disabled={installing}>
        ■ STOP SERVICES
      </button>
    {/if}

    {#if isRunning}
      <button class="btn" onclick={openApp}>🌐 Buka Aplikasi</button>
      <button class="btn" onclick={openSwagger}>📘 Buka Swagger API</button>
    {/if}
  </section>

  <section class="term">
    <div class="term-head">Terminal Log</div>
    <div class="term-body">
      {#if logs.length === 0}
        <span class="muted">Log instalasi akan muncul di sini…</span>
      {:else}
        {#each logs as l}
          <div class="ln">{l}</div>
        {/each}
      {/if}
    </div>
  </section>
</main>

<style>
  .wrap {
    max-width: 960px;
    margin: 0 auto;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    height: 100dvh;
    box-sizing: border-box;
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .head h1 { margin: 0; font-size: 22px; }
  .head p { margin: 4px 0 0; color: var(--muted); font-size: 13px; }
  .badge {
    font-size: 12px;
    font-weight: 700;
    padding: 6px 12px;
    border-radius: 999px;
    background: var(--panel-2);
    color: var(--muted);
  }
  .badge.on { background: rgba(34, 197, 94, 0.15); color: var(--green); }
  .badge.warn { background: rgba(239, 68, 68, 0.15); color: var(--red); }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 12px;
  }
  .card {
    background: var(--panel);
    border: 1px solid #334155;
    border-radius: 12px;
    padding: 14px;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .dot { width: 12px; height: 12px; border-radius: 50%; flex: none; }
  .dot.ok { background: var(--green); box-shadow: 0 0 8px var(--green); }
  .dot.no { background: var(--red); }
  .cl { flex: 1; font-size: 14px; }
  .st { font-size: 11px; color: var(--muted); }

  .paths {
    background: var(--panel);
    border: 1px solid #334155;
    border-radius: 12px;
    padding: 14px;
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .paths .k { color: var(--muted); }
  .paths code { color: #c7d2fe; word-break: break-all; }

  .err {
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid var(--red);
    color: #fecaca;
    padding: 10px 14px;
    border-radius: 10px;
    font-size: 13px;
    white-space: pre-wrap;
  }

  .actions { display: flex; flex-wrap: wrap; gap: 10px; }
  .btn {
    border: 1px solid #334155;
    background: var(--panel-2);
    color: var(--text);
    padding: 10px 16px;
    border-radius: 10px;
    font-weight: 600;
    font-size: 14px;
    cursor: pointer;
    transition: filter 0.15s;
  }
  .btn:hover:not(:disabled) { filter: brightness(1.15); }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .btn.green { background: var(--green); border-color: var(--green); color: #052e16; }
  .btn.red { background: var(--red); border-color: var(--red); color: #450a0a; }
  .btn.ghost { background: transparent; }

  .term {
    background: #020617;
    border: 1px solid #334155;
    border-radius: 12px;
    overflow: hidden;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .term-head {
    padding: 8px 14px;
    background: #0b1220;
    font-size: 12px;
    color: var(--muted);
    border-bottom: 1px solid #1e293b;
  }
  .term-body {
    padding: 12px 14px;
    overflow-y: auto;
    font-family: 'JetBrains Mono', ui-monospace, monospace;
    font-size: 12.5px;
    line-height: 1.55;
    flex: 1;
  }
  .ln { white-space: pre-wrap; word-break: break-word; color: #a5f3fc; }
  .muted { color: var(--muted); }
</style>
