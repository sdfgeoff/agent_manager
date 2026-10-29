'use strict';
const $ = id => document.getElementById(id);
const state = { workers: [], worker: null, thread: null, threads: [], items: new Map(), active: {}, requests: [], events: null, cursor: null, older: null, edit: false, folder: '~', epoch: 0, syncing: false, buffered: [], sessionBusy: false };
const settings = new Map();
const drafts = new Map();
function saveDraft() { if (state.thread) drafts.set(`${state.worker}/${state.thread.id}`, $('prompt').value); }
const el = (tag, cls, text) => { const node = document.createElement(tag); if (cls) node.className = cls; if (text !== undefined) node.textContent = text; return node; };
function notice(text) { $('notice').textContent = text || ''; $('notice').hidden = !text; }
async function api(path, body, method) {
  const response = await fetch(path, { method: method || (body === undefined ? 'GET' : 'POST'), headers: { 'Content-Type': 'application/json', 'X-Agentmanager': '1' }, body: body === undefined ? undefined : JSON.stringify(body), credentials: 'same-origin' });
  const data = await response.json().catch(() => ({}));
  if (response.status === 401 && path !== '/api/login') showLogin();
  if (!response.ok) throw new Error(data.error || `Request failed (${response.status})`);
  return data;
}
function rpc(method, params = {}, worker = state.worker) { if (!worker) return Promise.reject(new Error('Choose an online worker first.')); return api(`/api/workers/${encodeURIComponent(worker)}/rpc`, { method, params }); }
function errorText(error) { const message = error.message || String(error); return message.includes('active writer') ? 'This conversation is open in another Codex process. You can read its history here; close the other client before sending a message or changing settings.' : message; }
function safe(task) { return (...args) => Promise.resolve().then(() => task(...args)).catch(e => notice(errorText(e))); }
function rememberSettings(id, result) {
  const previous = settings.get(id) || {}; const thread = result.thread || {};
  const config = { ...previous, model: result.model || thread.model || previous.model || '', effort: result.reasoningEffort || thread.reasoningEffort || previous.effort || '', approvalPolicy: result.approvalPolicy || previous.approvalPolicy || 'on-request', sandbox: result.sandbox ? result.sandbox.type === 'dangerFullAccess' ? 'danger-full-access' : result.sandbox.type === 'readOnly' ? 'read-only' : 'workspace-write' : previous.sandbox || 'workspace-write' };
  settings.set(id, config); return config;
}
function showLogin() {
  state.events?.close(); state.events = null; state.epoch++; state.worker = null; state.thread = null;
  state.items.clear(); state.requests = []; state.active = {}; settings.clear(); drafts.clear();
  $('messages').replaceChildren(); $('requests').replaceChildren(); $('prompt').value = ''; $('app').hidden = true; $('login-view').hidden = false;
  document.querySelectorAll('dialog[open]').forEach(d => d.close());
}
async function start() {
  const user = await api('/api/me'); $('logout').title = `Sign out ${user.username}`; $('login-view').hidden = true; $('app').hidden = false; await loadWorkers(); connectEvents();
}
$('login-form').onsubmit = async event => {
  event.preventDefault(); const button = event.submitter; button.disabled = true; $('login-error').textContent = '';
  try { await api('/api/login', { username: $('username').value.trim(), password: $('password').value }); $('password').value = ''; await start(); } catch (e) { $('login-error').textContent = e.message; } finally { button.disabled = false; }
};
$('logout').onclick = safe(async () => { await api('/api/logout', {}); showLogin(); });
async function loadWorkers() {
  state.workers = await api('/api/workers'); $('worker-count').textContent = state.workers.length;
  const select = $('worker-select'); select.replaceChildren();
  if (!state.workers.length) select.append(new Option('No workers registered', ''));
  for (const worker of state.workers) select.append(new Option(`${worker.online ? '●' : '○'} ${worker.name}${worker.online ? '' : ' · offline'}`, worker.id));
  if (!state.workers.some(w => w.id === state.worker)) {
    const worker = state.workers.find(w => w.online) || state.workers[0];
    await chooseWorker(worker?.id || null);
  }
  select.value = state.worker || ''; updateControls();
}
async function chooseWorker(id) {
  saveDraft(); $('prompt').value = '';
  state.epoch++; state.worker = id; state.thread = null; state.items.clear(); state.active = {}; state.requests = []; state.threads = []; state.cursor = null;
  $('messages').replaceChildren(); $('requests').replaceChildren(); $('sessions').replaceChildren(); $('welcome').hidden = false; $('conversation').hidden = true; $('composer').hidden = true; $('configure').disabled = true;
  $('session-title').textContent = 'Your workspace'; $('session-path').textContent = state.workers.find(w => w.id === id)?.name || 'Register a worker to get started.'; notice('');
  if (online()) { await Promise.all([loadThreads(), loadModels(), loadStatus()]); }
  updateControls();
}
function online() { return !!state.workers.find(w => w.id === state.worker)?.online; }
function updateControls() {
  $('new-session').disabled = !online(); $('welcome-new').disabled = !online(); $('refresh').disabled = !online(); $('revoke-worker').disabled = !state.worker;
  $('send').disabled = !online() || state.sessionBusy; $('stop').disabled = !online();
  const active = state.thread && state.active[state.thread.id]; $('stop').hidden = !active; $('send').textContent = active ? 'Steer ↑' : 'Send ↑';
  $('session-status').textContent = !online() ? 'Offline' : active ? 'Working' : 'Ready'; $('busy-hint').textContent = active ? 'New messages steer the current task' : '';
}
$('worker-select').onchange = safe(event => chooseWorker(event.target.value));
async function loadThreads(more = false) {
  const worker = state.worker;
  const data = await rpc('manager/list', { limit: 50, ...(more && state.cursor ? { cursor: state.cursor } : {}) }, worker);
  if (worker !== state.worker) return;
  state.threads = [...new Map((more ? [...state.threads, ...data.data] : data.data).map(thread => [thread.id, thread])).values()].sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0)); state.cursor = data.nextCursor; $('more-sessions').hidden = !state.cursor; renderThreads();
}
function renderThreads() {
  const container = $('sessions'); container.replaceChildren();
  if (!state.threads.length) container.append(el('p', 'muted', online() ? 'No sessions yet.' : 'Worker is offline.'));
  for (const thread of state.threads) {
    const button = el('button', `session${state.thread?.id === thread.id ? ' selected' : ''}`);
    button.append(el('div', 'title', thread.name || thread.preview || 'Untitled session'), el('div', 'subtitle', `${state.active[thread.id] ? '● Working · ' : ''}${thread.cwd || ''}`));
    button.onclick = safe(() => openThread(thread)); container.append(button);
  }
}
async function loadModels() {
  const worker = state.worker;
  const data = await rpc('model/list', {}, worker);
  if (worker !== state.worker) return;
  $('models').replaceChildren(...(data.data || []).map(m => new Option(m.displayName || m.model, m.model || m.id)));
}
async function loadStatus() {
  const worker = state.worker;
  const data = await rpc('manager/status', {}, worker);
  if (worker !== state.worker) return;
  state.active = data.active || {}; state.requests = data.requests || []; renderRequests(); updateControls(); renderThreads();
  if (!data.alive) notice('Codex stopped on this worker. Restart the worker to resume saved sessions.');
}
async function openThread(thread) {
  saveDraft(); $('prompt').value = drafts.get(`${state.worker}/${thread.id}`) || '';
  const epoch = ++state.epoch; const worker = state.worker; state.thread = thread; state.items.clear(); state.syncing = true; state.buffered = [];
  $('messages').replaceChildren(); $('welcome').hidden = true; $('conversation').hidden = false; $('composer').hidden = false; $('configure').disabled = false;
  $('session-title').textContent = thread.name || thread.preview || 'Untitled session'; $('session-path').textContent = thread.cwd || ''; $('sidebar').classList.remove('open'); renderThreads(); notice('');
  try {
    const result = await rpc('thread/read', { threadId: thread.id }, worker);
    if (epoch !== state.epoch) return;
    state.thread = { ...thread, ...result.thread }; rememberSettings(thread.id, result);
    const history = await rpc('thread/turns/list', { threadId: thread.id, itemsView: 'full', sortDirection: 'desc', limit: 20 }, worker);
    if (epoch !== state.epoch) return;
    state.older = history.nextCursor; $('older').hidden = !state.older;
    for (const turn of [...history.data].reverse()) for (const item of turn.items || []) upsertItem(item, false);
    state.syncing = false;
    for (const message of state.buffered) handleCodex(message); state.buffered = [];
    $('model-label').textContent = settings.get(thread.id).model || 'Worker’s default model';
    await loadStatus(); scrollEnd(true);
  } catch (e) { if (epoch === state.epoch) { state.syncing = false; notice(e.message); } }
}
$('older').onclick = safe(async () => {
  const epoch = state.epoch; const threadId = state.thread.id; const worker = state.worker;
  const data = await rpc('thread/turns/list', { threadId, cursor: state.older, itemsView: 'full', sortDirection: 'desc', limit: 20 }, worker);
  if (epoch !== state.epoch) return;
  const fragment = document.createDocumentFragment();
  for (const turn of [...data.data].reverse()) for (const item of turn.items || []) { if (state.items.has(item.id)) continue; const node = renderItem(item); state.items.set(item.id, { item, node }); fragment.append(node); }
  const container = $('conversation'); const height = container.scrollHeight; $('messages').prepend(fragment); container.scrollTop += container.scrollHeight - height;
  state.older = data.nextCursor; $('older').hidden = !state.older;
});
function renderItem(item) {
  const user = item.type === 'userMessage'; const agent = item.type === 'agentMessage';
  const node = el('article', `message ${user ? 'user' : agent ? 'agent' : 'tool'}`); node.dataset.item = item.id;
  if (user || agent) {
    const text = agent ? item.text || '' : (item.content || []).map(c => c.text || c.type).join('\n');
    node.append(el('div', 'message-label', user ? 'You' : 'Codex'), el('div', 'message-body', text));
  } else {
    const details = el('details');
    const title = item.type === 'commandExecution' ? `$ ${item.command || 'Command'} · ${item.status || ''}` : item.type === 'fileChange' ? `File changes · ${item.status || ''}` : item.type === 'reasoning' ? 'Reasoning' : item.type;
    details.append(el('summary', '', title), el('div', 'message-body', item.aggregatedOutput || item.output || (item.summary || []).join?.('\n') || JSON.stringify(item, null, 2))); node.append(details);
  }
  return node;
}
function upsertItem(item, shouldScroll = true) {
  if (!item?.id) return;
  const old = state.items.get(item.id); const node = renderItem(item);
  if (old) old.node.replaceWith(node); else $('messages').append(node);
  state.items.set(item.id, { item, node }); if (shouldScroll) scrollEnd();
}
function scrollEnd(force = false) { const box = $('conversation'); if (force || box.scrollHeight - box.scrollTop - box.clientHeight < 220) $('chat-end').scrollIntoView({ block: 'end' }); }
function connectEvents() {
  state.events?.close(); const events = new EventSource('/api/events'); state.events = events;
  events.onopen = () => { $('connection').textContent = '● Connected'; $('connection').classList.add('online'); safe(async () => { await loadWorkers(); if (online()) { await loadThreads(); if (state.thread) await openThread(state.thread); else await loadStatus(); } })(); };
  events.onerror = () => { $('connection').textContent = '○ Reconnecting…'; $('connection').classList.remove('online'); };
  events.onmessage = event => {
    try {
      const data = JSON.parse(event.data);
      if (data.type === 'workers_changed') safe(async () => { await loadWorkers(); if (online()) { await loadThreads(); await loadStatus(); if (state.thread) await openThread(state.thread); } })();
      if (data.type === 'resync') safe(async () => { if (state.thread) await openThread(state.thread); await loadStatus(); })();
      if (data.type === 'codex' && data.worker === state.worker) {
        if (state.syncing) state.buffered.push(data.message); else handleCodex(data.message);
      }
    } catch (e) { notice(e.message); }
  };
}
function handleCodex(message) {
  const method = message.method; const p = message.params || {};
  if (message.id !== undefined || method === 'manager/requestsChanged' || method === 'turn/completed' || method === 'item/completed') safe(loadStatus)();
  if (method === 'manager/codexExited') notice('Codex exited. Restart the worker to resume saved sessions.');
  if (['manager/sessionsChanged', 'thread/started', 'thread/name/updated', 'thread/archived', 'thread/unarchived'].includes(method)) safe(() => loadThreads())();
  if (method === 'manager/resync' || method === 'manager/connected') safe(async () => { await loadStatus(); if (state.thread) await openThread(state.thread); })();
  if (method === 'turn/started') { state.active[p.threadId] = p.turn.id; updateControls(); renderThreads(); }
  if (method === 'turn/completed') { delete state.active[p.threadId]; updateControls(); safe(() => loadThreads())(); if (p.turn?.error) notice(p.turn.error.message || JSON.stringify(p.turn.error)); }
  if (!state.thread || p.threadId !== state.thread.id) return;
  if (method === 'item/started' || method === 'item/completed') upsertItem(p.item);
  if (method === 'item/agentMessage/delta') {
    const previous = state.items.get(p.itemId)?.item || { id: p.itemId, type: 'agentMessage', text: '' };
    upsertItem({ ...previous, text: previous.text + p.delta });
  }
  if (method === 'item/commandExecution/outputDelta') {
    const previous = state.items.get(p.itemId)?.item || { id: p.itemId, type: 'commandExecution', command: '' };
    upsertItem({ ...previous, aggregatedOutput: (previous.aggregatedOutput || '') + p.delta });
  }
  if (method === 'error') notice(p.error?.message || JSON.stringify(p));
}
function renderRequests() {
  const container = $('requests'); container.replaceChildren();
  for (const request of state.requests) {
    if (request.params?.threadId && request.params.threadId !== state.thread?.id) continue;
    const box = el('section', 'request'); const method = request.method; const params = request.params || {};
    box.append(el('h3', '', method === 'item/tool/requestUserInput' ? 'Codex has a question' : 'Your input is needed'));
    const actions = el('div', 'request-actions');
    const answer = result => safe(async () => { box.querySelectorAll('button').forEach(b => b.disabled = true); try { await rpc('manager/respond', { requestId: request.id, result }); } finally { await loadStatus(); } })();
    if (method === 'item/tool/requestUserInput' || method === 'tool/requestUserInput') {
      const inputs = [];
      for (const q of params.questions || []) {
        const label = el('label'); label.append(el('div', 'question-text', q.question || q.header || q.id));
        let input;
        if (q.options?.length) {
          input = el('select'); for (const option of q.options) input.append(new Option(`${option.label}${option.description ? ' — ' + option.description : ''}`, option.label));
        } else { input = el('input'); input.type = q.isSecret ? 'password' : 'text'; }
        label.append(input); box.append(label); inputs.push([q.id, input]);
      }
      const submit = el('button', 'primary', 'Answer'); submit.onclick = () => answer({ answers: Object.fromEntries(inputs.map(([id, input]) => [id, { answers: [input.value] }])) }); actions.append(submit);
    } else if (method === 'item/commandExecution/requestApproval' || method === 'item/fileChange/requestApproval') {
      box.append(el('pre', '', [params.command, params.cwd, params.reason, params.changes ? JSON.stringify(params.changes, null, 2) : ''].filter(Boolean).join('\n') || JSON.stringify(params, null, 2)));
      for (const [label, decision] of [['Approve once', 'accept'], ['Approve for session', 'acceptForSession'], ['Decline', 'decline'], ['Cancel', 'cancel']]) {
        if (params.availableDecisions && !params.availableDecisions.includes(decision)) continue;
        const button = el('button', decision === 'accept' ? 'primary' : '', label); button.onclick = () => answer({ decision }); actions.append(button);
      }
    } else {
      box.append(el('p', 'muted', 'Respond using the JSON format requested by Codex.'), el('pre', '', JSON.stringify(request, null, 2)));
      const response = el('textarea'); response.rows = 3; response.placeholder = 'JSON response'; response.setAttribute('aria-label', 'JSON response'); box.append(response);
      const submit = el('button', 'primary', 'Send response'); submit.onclick = safe(async () => { let result; try { result = JSON.parse(response.value); } catch { throw new Error('Enter a valid JSON response.'); } answer(result); }); actions.append(submit);
    }
    box.append(actions); container.append(box);
  }
}
$('composer').onsubmit = safe(async event => {
  event.preventDefault(); const text = $('prompt').value.trim(); if (!text || !state.thread || state.sessionBusy) return;
  const worker = state.worker; const threadId = state.thread.id; const active = state.active[threadId];
  state.sessionBusy = true; updateControls(); notice('');
  try {
    const input = [{ type: 'text', text }];
    if (active) await rpc('turn/steer', { threadId, expectedTurnId: active, input }, worker);
    else {
      const resumed = await rpc('thread/resume', { threadId, excludeTurns: true }, worker);
      const config = rememberSettings(threadId, resumed);
      const params = { threadId, input }; if (config.model) params.model = config.model; if (config.effort) params.effort = config.effort;
      const result = await rpc('turn/start', params, worker); if (state.thread?.id === threadId) state.active[threadId] = result.turn.id;
    }
    drafts.delete(`${worker}/${threadId}`);
    if (state.thread?.id === threadId && $('prompt').value.trim() === text) $('prompt').value = ''; await loadStatus(); scrollEnd(true);
  } finally { state.sessionBusy = false; updateControls(); }
});
$('prompt').onkeydown = event => { if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && !matchMedia('(pointer: coarse)').matches) { event.preventDefault(); $('composer').requestSubmit(); } };
$('stop').onclick = safe(async () => { const threadId = state.thread.id; await rpc('turn/interrupt', { threadId, turnId: state.active[threadId] }); });
async function newSession() {
  if (!online()) return; state.edit = false; $('dialog-title').textContent = 'New session'; $('create').textContent = 'Create session ↗';
  $('new-name').value = ''; $('new-path').value = '~'; $('new-model').value = ''; $('new-effort').value = ''; $('new-approval').value = 'on-request'; $('new-sandbox').value = 'workspace-write'; $('new-instructions').value = ''; $('new-instructions').disabled = false; $('new-path').disabled = false; $('browse').disabled = false; $('session-error').textContent = ''; $('session-dialog').showModal();
}
$('new-session').onclick = newSession; $('welcome-new').onclick = newSession;
$('configure').onclick = safe(async () => {
  if (!state.thread) return; const epoch = state.epoch; const id = state.thread.id;
  const resumed = await rpc('thread/resume', { threadId: id, excludeTurns: true });
  if (epoch !== state.epoch) return;
  state.edit = true; const config = rememberSettings(id, resumed);
  $('dialog-title').textContent = 'Session settings'; $('create').textContent = 'Save settings'; $('new-name').value = state.thread.name || ''; $('new-path').value = state.thread.cwd || ''; $('new-path').disabled = true; $('browse').disabled = true; $('new-model').value = config.model || ''; $('new-effort').value = config.effort || ''; $('new-approval').value = config.approvalPolicy || 'on-request'; $('new-sandbox').value = config.sandbox || 'workspace-write'; $('new-instructions').value = ''; $('new-instructions').disabled = true; $('session-error').textContent = ''; $('session-dialog').showModal();
});
$('session-form').onsubmit = async event => {
  event.preventDefault(); const button = $('create'); button.disabled = true; $('session-error').textContent = '';
  const worker = state.worker;
  const config = { model: $('new-model').value.trim(), effort: $('new-effort').value, approvalPolicy: $('new-approval').value, sandbox: $('new-sandbox').value };
  try {
    const params = { ...(config.model ? { model: config.model } : {}), ...(config.effort ? { config: { model_reasoning_effort: config.effort } } : {}), approvalPolicy: config.approvalPolicy, sandbox: config.sandbox };
    let thread;
    if (state.edit) {
      if (state.active[state.thread.id]) throw new Error('Stop or wait for the current task before changing session settings.');
      const result = await rpc('thread/resume', { ...params, threadId: state.thread.id, excludeTurns: true }, worker); thread = result.thread;
    } else {
      const folder = await rpc('fs/list', { path: $('new-path').value.trim() }, worker);
      const instructions = $('new-instructions').value.trim();
      const result = await rpc('thread/start', { ...params, cwd: folder.path, ...(instructions ? { developerInstructions: instructions } : {}), ...(config.effort ? { config: { model_reasoning_effort: config.effort } } : {}) }, worker); thread = result.thread;
    }
    settings.set(thread.id, config);
    const name = $('new-name').value.trim(); if (name) { await rpc('thread/name/set', { threadId: thread.id, name }, worker); thread.name = name; }
    $('session-dialog').close(); await loadThreads(); await openThread(thread);
  } catch (e) { $('session-error').textContent = e.message; } finally { button.disabled = false; }
};
async function browse(path) {
  $('folder-error').textContent = ''; const data = await rpc('fs/list', { path }); state.folder = data.path; $('folder-path').value = data.path; $('folder-up').disabled = !data.parent; $('folder-up').dataset.parent = data.parent || '';
  $('folder-list').replaceChildren(); for (const entry of data.entries) { const button = el('button', '', `▱  ${entry.name}`); button.onclick = safe(() => browse(entry.path)); $('folder-list').append(button); }
  if (!data.entries.length) $('folder-list').append(el('p', 'muted', 'No subfolders.'));
  if (data.truncated) $('folder-error').textContent = 'Large directory: some folders are omitted. Enter a path directly to open one.';
}
$('browse').onclick = safe(async () => { $('folder-dialog').showModal(); try { await browse($('new-path').value); } catch (e) { $('folder-error').textContent = e.message; } });
$('folder-form').onsubmit = async event => { event.preventDefault(); try { await browse($('folder-path').value); } catch (e) { $('folder-error').textContent = e.message; } };
$('folder-up').onclick = safe(() => browse($('folder-up').dataset.parent));
$('folder-use').onclick = () => { $('new-path').value = state.folder; $('folder-dialog').close(); };
function ask(title, description) {
  return new Promise(resolve => {
    $('input-title').textContent = title; $('input-description').textContent = description; $('input-value').value = ''; $('input-dialog').showModal();
    $('input-form').onsubmit = event => { event.preventDefault(); resolve($('input-value').value); $('input-dialog').close(); };
    $('input-dialog').onclose = () => resolve(null);
  });
}
$('folder-create').onclick = safe(async () => {
  const name = await ask('Create a folder', 'Folder name or absolute path'); if (!name) return;
  const path = name.startsWith('/') || name.startsWith('~/') ? name : `${state.folder}/${name}`;
  const result = await rpc('fs/mkdir', { path }); await browse(result.path);
});
$('add-worker').onclick = safe(async () => { const data = await api('/api/enrollment', {}); $('worker-command').textContent = `worker --host ${data.host} --key ${data.key}`; $('worker-dialog').showModal(); });
$('copy-worker').onclick = safe(() => navigator.clipboard.writeText($('worker-command').textContent));
$('revoke-worker').onclick = safe(async () => { const worker = state.workers.find(w => w.id === state.worker); if (!worker || !confirm(`Revoke ${worker.name}? It will lose remote access and need a new registration key.`)) return; await api(`/api/workers/${worker.id}`, undefined, 'DELETE'); await loadWorkers(); });
$('refresh').onclick = safe(async () => { await loadWorkers(); if (online()) { await loadThreads(); await loadStatus(); } });
$('more-sessions').onclick = safe(() => loadThreads(true));
$('open-menu').onclick = () => $('sidebar').classList.add('open'); $('close-menu').onclick = () => $('sidebar').classList.remove('open');
document.querySelectorAll('.close-dialog').forEach(button => button.onclick = () => button.closest('dialog').close());
document.addEventListener('visibilitychange', () => { if (!document.hidden && state.events) safe(async () => { await loadWorkers(); if (online()) { await loadStatus(); if (state.thread) await openThread(state.thread); } })(); });
start().catch(() => showLogin());
