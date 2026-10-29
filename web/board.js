'use strict';
(() => {
  const b = { overview: null, agents: [], channel: null, mode: 'conversations', topics: [], cursor: null, topic: null, visible: false, drafts: new Map(), files: new Map(), timer: null, refreshing: false, again: false, edit: null, source: null, notifications: [], inboxCursor: null, epoch: 0 };
  const readOps = new Set(['overview', 'list_topics', 'read_topic', 'list_agents', 'list_notifications', 'read_attachment']);
  const call = (op, data = {}) => api(`/api/board/${op}`, readOps.has(op) ? data : { requestId: crypto.randomUUID(), ...data });
  function error(e) { $('board-error').textContent = e.message || String(e); $('board-error').hidden = false; }
  function action(fn) { return event => { if (event?.type === 'submit') event.preventDefault(); return Promise.resolve().then(() => fn(event)).catch(error); }; }
  function button(text, fn, cls = '') { const n = el('button', cls, text); n.type = 'button'; n.onclick = action(fn); return n; }
  const person = id => b.overview?.people.find(p => p.id === id)?.name || id || 'Unassigned';
  function label(text, control) { const n = el('label', '', text); n.append(control); return n; }
  function select(options, value, aria, fn) { const n = el('select'); n.setAttribute('aria-label', aria); for (const [v, text] of options) n.add(new Option(text, v)); n.value = value || ''; n.onchange = action(() => fn(n.value)); return n; }
  function date(t) { return new Date(t * 1000).toLocaleString(); }
  function markdown(text) {
    const root = el('div', 'board-markdown');
    // Render a deliberately small Markdown subset using DOM nodes; never interpret HTML.
    const inline = (node, value) => {
      const pattern = /(`[^`\n]+`|\*\*[^*\n]+\*\*|\[[^\]\n]+\]\([^\s)]+\))/g; let at = 0;
      for (const match of value.matchAll(pattern)) {
        node.append(document.createTextNode(value.slice(at, match.index))); const s = match[0];
        if (s.startsWith('`')) node.append(el('code', '', s.slice(1, -1)));
        else if (s.startsWith('**')) node.append(el('strong', '', s.slice(2, -2)));
        else { const [, title, href] = s.match(/^\[([^\]]+)\]\((.+)\)$/); let url; try { url = new URL(href, location.origin); } catch {} if (url && ['http:', 'https:'].includes(url.protocol)) { const a = el('a', '', title); a.href = url.href; a.target = '_blank'; a.rel = 'noopener noreferrer'; node.append(a); } else node.append(document.createTextNode(s)); }
        at = match.index + s.length;
      }
      node.append(document.createTextNode(value.slice(at)));
    };
    let code = null;
    for (const line of String(text || '').split('\n')) {
      if (line.startsWith('```')) { if (code) code = null; else { code = el('pre'); root.append(code); } continue; }
      if (code) { code.append(document.createTextNode(`${line}\n`)); continue; }
      const heading = line.match(/^(#{1,3})\s+(.+)$/); const n = el(heading ? `h${heading[1].length + 2}` : 'p'); inline(n, heading ? heading[2] : line); root.append(n);
    }
    return root;
  }
  function attachmentNodes(files) {
    const root = el('div');
    for (const file of files || []) {
      const a = el('a', 'board-file', `${file.name} · ${Math.ceil(file.size / 1024)} KB`); a.href = `/api/attachments/${encodeURIComponent(file.id)}`; a.target = '_blank'; a.rel = 'noopener';
      if (['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(file.mime)) { const image = el('img', 'board-image'); image.src = a.href; image.alt = file.name; image.loading = 'lazy'; a.prepend(image); }
      root.append(a);
    }
    return root;
  }
  function composer(key, submit, title) {
    const form = el('form', 'board-composer'); let titleInput;
    if (title) { titleInput = el('input'); titleInput.placeholder = 'Topic title (optional)'; titleInput.setAttribute('aria-label', 'Topic title'); titleInput.maxLength = 500; titleInput.value = b.drafts.get(`${key}:title`) || ''; titleInput.oninput = () => b.drafts.set(`${key}:title`, titleInput.value); form.append(titleInput); }
    const input = el('textarea'); input.setAttribute('aria-label', title ? 'New topic message' : 'Reply'); input.placeholder = title ? 'Start a discussion… Mention @someone to bring them in.' : 'Reply…'; input.maxLength = 200000; input.value = b.drafts.get(key) || ''; input.oninput = () => b.drafts.set(key, input.value); form.append(input);
    const files = el('input'); files.type = 'file'; files.multiple = true; files.setAttribute('aria-label', 'Attach files');
    const list = el('div');
    function renderFiles() { list.replaceChildren(); for (const f of b.files.get(key) || []) { const row = el('div', 'board-meta', f.name); row.append(button('Remove', () => { b.files.set(key, b.files.get(key).filter(x => x.id !== f.id)); renderFiles(); }, 'text-button')); list.append(row); } }
    renderFiles(); form.append(list);
    const send = el('button', 'primary', title ? 'Create discussion' : 'Send reply'); send.type = 'submit'; const row = el('div', 'board-row'); row.append(files, send); form.append(row);
    let uploading = false; let requestId = null;
    files.onchange = action(async () => {
      const chosen = [...files.files]; const existing = b.files.get(key) || [];
      if (chosen.length + existing.length > 10) throw new Error('At most 10 attachments per post.');
      uploading = true; send.disabled = true;
      try { for (const file of chosen) { if (file.size > 20 * 1024 * 1024) throw new Error('Files must be 20 MB or smaller.'); const encoded = await new Promise((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve(reader.result.split(',')[1]); reader.onerror = reject; reader.readAsDataURL(file); }); const saved = await call('upload_attachment', { name: file.name, mime: file.type || 'application/octet-stream', data: encoded }); existing.push(saved); b.files.set(key, existing); renderFiles(); } } finally { uploading = false; send.disabled = false; files.value = ''; }
    });
    form.onsubmit = action(async () => {
      if (send.disabled || uploading || (!input.value.trim() && !(b.files.get(key) || []).length)) return;
      send.disabled = true; requestId ||= crypto.randomUUID();
      try { await submit({ body: input.value, title: titleInput?.value, attachments: (b.files.get(key) || []).map(f => f.id), requestId }); b.drafts.delete(key); b.drafts.delete(`${key}:title`); b.files.delete(key); input.value = ''; if (titleInput) titleInput.value = ''; requestId = null; renderFiles(); await refresh(); } finally { send.disabled = false; }
    });
    input.onkeydown = event => { if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && !matchMedia('(pointer: coarse)').matches) { event.preventDefault(); if (!send.disabled) form.requestSubmit(send); } };
    input.addEventListener('input', () => { requestId = null; });
    return form;
  }
  async function loadOverview() { b.overview = await call('overview'); b.agents = await call('list_agents'); if (!b.channel) b.channel = b.overview.channels[0]?.id; }
  function channels() {
    const root = $('board-channel-list'); root.replaceChildren();
    for (const c of b.overview.channels) root.append(button(`#${c.name}${c.subscribed ? ' ●' : ''}`, async () => { b.channel = c.id; b.topic = null; b.topics = []; b.cursor = null; if (b.mode === 'agents' || b.mode === 'inbox') b.mode = 'conversations'; await refresh(); }, c.id === b.channel ? 'selected' : ''));
  }
  function topicCard(t) {
    const card = button('', () => openTopic(t.id), 'board-card'); card.append(el('h3', '', t.title), el('div', 'board-meta', `#${t.id} · ${t.status} · ${person(t.assignee)}`), el('div', '', t.body.slice(0, 180))); return card;
  }
  function topics() {
    const root = $('board-topic-list');
    const mountedComposer=$('board-new-topic').querySelector(`[data-channel-composer="${b.channel}"]`);
    const signature=JSON.stringify([b.mode,b.topics,b.overview.people,$('board-show-archived').checked,$('board-show-discussion').checked]);
    if(root.dataset.signature===signature && mountedComposer)return;
    root.dataset.signature=signature;root.replaceChildren(); $('board-new-topic').replaceChildren();
    const archived = $('board-show-archived').checked; const discussion = $('board-show-discussion').checked;
    const visible = b.topics.filter(t => (archived || t.status !== 'archived') && (b.mode !== 'board' || discussion || t.status !== 'discussion'));
    if (b.mode === 'board') {
      const columns = el('div', 'board-columns');
      for (const status of b.overview.statuses.filter(s => (s !== 'discussion' || discussion) && (s !== 'archived' || archived))) { const col = el('section', 'board-column'); const entries = visible.filter(t => t.status === status); col.append(el('h3', '', `${status} · ${entries.length}`)); for (const t of entries) col.append(topicCard(t)); columns.append(col); }
      root.append(columns);
    } else { for (const t of visible) root.append(topicCard(t)); }
    if (!visible.length) root.append(el('p', 'board-empty', b.mode === 'board' ? 'No topics in the visible board columns. New topics begin in Discussion.' : 'No conversations yet.'));
    const channel = b.channel;
    const newComposer=mountedComposer || composer(`channel:${channel}`, async data => { const t = await call('create_topic', { ...data, channel }); b.topic = t.id; }, true);newComposer.dataset.channelComposer=channel;$('board-new-topic').append(newComposer);
    $('board-more').hidden = !b.cursor;
  }
  async function openTopic(id) { b.topic = id; await renderTopic(); if (matchMedia('(max-width:1000px)').matches) $('board-detail').scrollIntoView({ behavior: 'smooth', block: 'start' }); }
  async function renderTopic() {
    const root = $('board-detail'); if (!b.topic || !['conversations', 'board'].includes(b.mode)) { root.hidden = true; return; }
    const id = b.topic; const t = await call('read_topic', { topic: id }); if (b.topic !== id || !b.visible) return;
    root.hidden = false;const signature=JSON.stringify([t,b.overview.people]);if(root.dataset.signature===signature && root.querySelector(`[data-topic-composer="${id}"]`))return;root.dataset.signature=signature; const oldScroll = root.scrollTop;
    // Keep the mounted reply composer (focus, selection, draft and file picker) across live updates.
    const existingComposer = root.querySelector(`[data-topic-composer="${id}"]`); root.replaceChildren();
    const heading = el('div', 'board-row'); heading.append(el('h2', '', t.title), button('Rename', async () => {const title=await ask('Rename topic','Topic title');if(title){await call('update_topic',{topic:id,title});await refresh();}}), button('Close', () => { b.topic = null; root.hidden = true; })); root.append(heading);
    const controls = el('div', 'board-row');
    controls.append(select(b.overview.statuses.map(s => [s, s]), t.status, 'Topic status', async status => { await call('update_topic', { topic: id, status }); await refresh(); }), select([['', 'Unassigned'], ...b.overview.people.map(p => [p.id, `${p.name} (@${p.handle})`])], t.assignee, 'Topic assignee', async assignee => { await call('update_topic', { topic: id, assignee: assignee || null }); await refresh(); }), button(t.subscribed ? 'Unsubscribe' : 'Subscribe', async () => { await call('set_subscription', { kind: 'topic', target: id, subscribed: !t.subscribed }); await renderTopic(); })); root.append(controls);
    const opening = el('article', 'board-post'); opening.append(el('div', 'board-meta', `${person(t.author)} · #${id} · ${date(t.createdAt)}`), markdown(t.body), attachmentNodes(t.attachments)); root.append(opening);
    const repliesRoot = el('div'); root.append(repliesRoot);
    const addReplies = replies => { const fragment = document.createDocumentFragment(); for (const r of replies) { const n = el('article', 'board-post'); n.append(el('div', 'board-meta', `${person(r.author)} · ${date(r.createdAt)}`), markdown(r.body), attachmentNodes(r.attachments)); fragment.append(n); } return fragment; };
    if (t.nextCursor) { let cursor = t.nextCursor; const earlier = button('Load earlier replies', async () => { const page = await call('read_topic', { topic: id, before: cursor }); repliesRoot.prepend(addReplies(page.replies)); cursor = page.nextCursor; earlier.hidden = !cursor; }); root.insertBefore(earlier, repliesRoot); }
    repliesRoot.append(addReplies(t.replies));
    const reply = existingComposer || composer(`topic:${id}`, data => call('post_reply', { ...data, topic: id }), false); reply.dataset.topicComposer = id; root.append(reply); root.scrollTop = oldScroll;
  }
  async function renderAgents() {
    const root = $('board-topic-list'); root.replaceChildren(button('＋ Create agent', () => agentDialog(), 'primary')); $('board-new-topic').replaceChildren(); $('board-more').hidden = true; $('board-detail').hidden = true;
    for (const a of b.agents) {
      const card = el('article', 'agent-card'); card.append(el('h3', '', `${a.name} (@${a.handle})`), el('div', 'board-meta', `${a.paused ? 'Paused' : a.online ? 'Online' : 'Worker offline'} · ${state.workers.find(w => w.id === a.worker)?.name || a.worker}`));
      const actions = el('div', 'agent-actions'); actions.append(button('Edit persona & subscriptions', () => agentDialog(a)), button(a.paused ? 'Resume' : 'Pause', async () => { const result = await call('update_agent', { id: a.id, paused: !a.paused }); if (!result.synchronized) error(new Error('Change saved; worker could not be reached. It will be applied when the worker reconnects.')); await refresh(); }), button('Open session ↗', () => viewSession(a))); card.append(actions);
      if (a.error) card.append(el('p', 'error', a.error)); root.append(card);
    }
    if (!b.agents.length) root.append(el('p', 'board-empty', 'Create an agent here, or register an existing session from Sessions.'));
  }
  async function renderInbox(more = false) {
    const data = await call('list_notifications', more && b.inboxCursor ? { before: b.inboxCursor } : {}); b.notifications = more ? [...b.notifications, ...data.data] : data.data; b.inboxCursor = data.data.length === 100 ? data.data.at(-1).id : null;
    const root = $('board-topic-list'); root.replaceChildren(); $('board-new-topic').replaceChildren(); $('board-detail').hidden = true; $('board-more').hidden = !b.inboxCursor;
    for (const n of b.notifications) root.append(button(`${n.read ? '' : '● '}${n.event.author.name} · ${n.event.kind} · #${n.event.channelName} / ${n.event.title}\n${n.event.body.slice(0, 140)}`, async () => { await call('mark_read', { id: n.id }); b.channel = n.event.channel; b.mode = 'conversations'; b.topic = n.event.topic; await refresh(); }, `board-inbox-item${n.read ? '' : ' unread'}`));
    if (!b.notifications.length) root.append(el('p', 'board-empty', 'No notifications. Subscribe to a channel or join a conversation.'));
  }
  async function refresh(more = false) {
    if (!b.visible) return;
    if (b.refreshing) { b.again = true; return; } b.refreshing = true;
    try {
      await loadOverview(); if (!b.visible) return; channels();
      for (const [id, mode] of [['board-conversations', 'conversations'], ['board-kanban', 'board'], ['board-agents', 'agents'], ['board-inbox', 'inbox']]) $(id).setAttribute('aria-current', String(b.mode === mode));
      const c = b.overview.channels.find(c => c.id === b.channel); $('board-channel-title').textContent = b.mode === 'agents' ? 'Agents' : b.mode === 'inbox' ? 'Your inbox' : `#${c?.name || ''}`;
      $('board-channel-subscribe').hidden = !['board', 'conversations'].includes(b.mode); $('board-filters').hidden = !['board', 'conversations'].includes(b.mode); $('board-show-discussion').closest('label').hidden = b.mode !== 'board';
      $('board-channel-subscribe').textContent = c?.subscribed ? 'Unsubscribe from channel' : 'Subscribe to channel';
      if (b.mode === 'agents') await renderAgents();
      else if (b.mode === 'inbox') await renderInbox(more);
      else { const channel = b.channel; const data = await call('list_topics', { channel, search: $('board-search').value, ...(more && b.cursor ? { before: b.cursor } : {}) }); if (channel !== b.channel) { b.again = true; return; } b.topics = more ? [...b.topics, ...data.data] : data.data; b.cursor = data.nextCursor; topics(); await renderTopic(); }
    } catch (e) { error(e); } finally { b.refreshing = false; if (b.again) { b.again = false; changed(); } }
  }
  function changed() { if (!b.visible) return; clearTimeout(b.timer); b.timer = setTimeout(() => refresh(), 180); }
  async function show() { saveDraft(); b.visible = true; $('app').hidden = true; $('board-view').hidden = false; $('board-error').hidden = true; await loadWorkers(); await refresh(); }
  function hide() { b.visible = false; clearTimeout(b.timer); $('board-view').hidden = true; }
  async function viewSession(a) { hide(); $('app').hidden = false; await chooseWorker(a.worker); const data = await rpc('thread/read', { threadId: a.thread }, a.worker); await openThread(data.thread); }
  async function agentDialog(agent = null, source = null) {
    await loadOverview(); await loadWorkers(); b.edit = agent; b.source = source;
    $('agent-form').reset(); $('agent-error').textContent = ''; $('agent-dialog-title').textContent = agent ? 'Edit agent' : source ? 'Register session as agent' : 'New agent'; $('agent-save').textContent = agent ? 'Save agent' : 'Create agent';
    $('agent-name').value = agent?.name || source?.name || ''; $('agent-handle').value = agent?.handle || ''; $('agent-handle').disabled = !!agent; $('agent-persona').value = agent?.persona || source?.persona || '';
    $('agent-location').hidden = !!(agent || source); $('agent-source').hidden = !(agent || source); $('agent-source').textContent = agent ? `Session ${agent.thread} · worker ${state.workers.find(w => w.id === agent.worker)?.name || agent.worker}` : source ? `Existing session · ${source.cwd}` : '';
    $('agent-worker').replaceChildren(...state.workers.map(w => new Option(`${w.name}${w.online ? '' : ' · offline'}`, w.id))); if (state.worker) $('agent-worker').value = state.worker;
    const subscriptions = $('agent-subscriptions'); subscriptions.replaceChildren(el('legend', '', 'Channel subscriptions · new topics'));
    for (const c of b.overview.channels) { const checkbox = el('input'); checkbox.type = 'checkbox'; checkbox.value = c.id; checkbox.dataset.channel = c.id; subscriptions.append(label(`#${c.name}`, checkbox)); }
    if (agent) { const overview = await api('/api/board/agent_subscriptions', { id: agent.id }); for (const s of overview.data) { const input = subscriptions.querySelector(`[data-channel="${s.target}"]`); if (input) input.checked = s.subscribed; } }
    $('agent-dialog').showModal();
  }
  $('agent-form').onsubmit = async event => {
    event.preventDefault(); const save = $('agent-save'); if (save.disabled) return; save.disabled = true; $('agent-error').textContent = '';
    try {
      let a = b.edit;
      if (a) await call('update_agent', { id: a.id, name: $('agent-name').value, persona: $('agent-persona').value });
      else {
        const settings = { approvalPolicy: $('agent-approval').value, sandbox: $('agent-sandbox').value }; if ($('agent-model').value.trim()) settings.model = $('agent-model').value.trim(); if ($('agent-effort').value) settings.config = { model_reasoning_effort: $('agent-effort').value };
        b.source ||= { requestId: crypto.randomUUID() };
        a = await call('create_agent', { requestId: b.source.requestId ||= crypto.randomUUID(), name: $('agent-name').value, handle: $('agent-handle').value, worker: b.source.worker || $('agent-worker').value, ...(b.source.thread ? { thread: b.source.thread } : { cwd: $('agent-cwd').value, settings }), persona: $('agent-persona').value });
        b.edit = a;
      }
      for (const input of $('agent-subscriptions').querySelectorAll('input')) await call('set_subscription', { actor: a.id, kind: 'channel', target: Number(input.value), subscribed: input.checked });
      $('agent-dialog').close(); if (b.visible) { b.mode = 'agents'; await refresh(); } else notice(`Registered ${a.name}. Open Board & agents to manage it.`);
    } catch (e) { $('agent-error').textContent = e.message; } finally { save.disabled = false; }
  };
  $('show-board').onclick = action(show); $('back-sessions').onclick = () => { hide(); $('app').hidden = false; };
  for (const [id, mode] of [['board-conversations', 'conversations'], ['board-kanban', 'board'], ['board-agents', 'agents'], ['board-inbox', 'inbox']]) $(id).onclick = action(async () => { b.mode = mode; b.topic = null; await refresh(); });
  $('board-add-channel').onclick = action(async () => { const name = await ask('Create channel', 'Channel name'); if (!name) return; const c = await call('create_channel', { name }); b.channel = c.id; b.mode = 'conversations'; await refresh(); });
  $('board-channel-subscribe').onclick = action(async () => { const c = b.overview.channels.find(c => c.id === b.channel); await call('set_subscription', { kind: 'channel', target: c.id, subscribed: !c.subscribed }); await refresh(); });
  $('board-show-discussion').onchange = () => topics(); $('board-show-archived').onchange = () => topics();
  $('board-search').oninput = changed; $('board-more').onclick = action(() => refresh(true));
  $('register-session').onclick = action(async () => { await loadOverview();const existing=b.agents.find(a=>a.worker===state.worker&&a.thread===state.thread.id);if(existing){await agentDialog(existing);return;}const data = await rpc('manager/instructions', { threadId: state.thread.id }); await agentDialog(null, { thread: state.thread.id, worker: state.worker, name: state.thread.name, cwd: state.thread.cwd, persona: data.persona }); });
  document.addEventListener('visibilitychange', () => { if (!document.hidden) changed(); });
  setInterval(() => { if (b.visible && !document.hidden) changed(); }, 20000);
  function reset(){hide();b.drafts.clear();b.files.clear();b.overview=null;b.agents=[];b.channel=null;b.topic=null;b.topics=[];b.cursor=null;b.edit=null;b.source=null;b.notifications=[];b.inboxCursor=null;$('board-topic-list').replaceChildren();$('board-new-topic').replaceChildren();$('board-detail').replaceChildren();delete $('board-topic-list').dataset.signature;delete $('board-detail').dataset.signature;}
  window.Board = { show, hide, changed, reset };
})();
