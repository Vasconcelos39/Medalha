'use strict';

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (sel) => document.querySelector(sel);
const IS_WINDOWS = navigator.userAgent.includes('Windows');

const state = {
  tab: 'medal',
  clips: [],
  meta: new Map(), // path -> { thumb, duration }
  status: null,
  config: null,
  filter: '',
};

// ---------- utilidades ----------

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') node.className = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null) node.setAttribute(k, v);
  }
  for (const c of children) node.append(c);
  return node;
}

function icon(path) {
  const span = document.createElement('span');
  span.innerHTML = `<svg viewBox="0 0 24 24">${path}</svg>`;
  return span.firstElementChild;
}

const ICONS = {
  play: '<path d="M8 5v14l11-7z"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
};

function formatDuration(secs) {
  if (secs == null) return '';
  const s = Math.round(secs);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

function formatSize(bytes) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  return `${(bytes / 1e6).toFixed(bytes >= 1e8 ? 0 : 1)} MB`;
}

const dateFmt = new Intl.DateTimeFormat('pt-BR', { dateStyle: 'short', timeStyle: 'short' });

function toast(message, kind = '') {
  const t = el('div', { class: `toast ${kind}` }, message);
  $('#toasts').append(t);
  setTimeout(() => t.remove(), kind === 'error' ? 7000 : 3500);
}

function debounce(fn, ms) {
  let id;
  return (...args) => {
    clearTimeout(id);
    id = setTimeout(() => fn(...args), ms);
  };
}

// ---------- abas ----------

function setTab(tab) {
  state.tab = tab;
  for (const b of document.querySelectorAll('.tab')) {
    b.setAttribute('aria-selected', String(b.dataset.tab === tab));
  }
  $('#view-medal').hidden = tab !== 'medal';
  $('#view-clips').hidden = tab !== 'clips';
  $('#webnav').hidden = tab !== 'medal';
  invoke('set_medal_visible', { visible: tab === 'medal' && !anyModalOpen() });
  if (tab === 'clips') refreshClips();
}

function anyModalOpen() {
  return [...document.querySelectorAll('dialog')].some((d) => d.open);
}

/** O webview do Medal fica por cima da página, então some enquanto há modal aberto. */
function openModal(dialog) {
  invoke('set_medal_visible', { visible: false });
  dialog.showModal();
}

for (const d of document.querySelectorAll('dialog')) {
  d.addEventListener('close', () => {
    if (state.tab === 'medal' && !anyModalOpen()) invoke('set_medal_visible', { visible: true });
  });
}

document.querySelectorAll('.tab').forEach((b) => b.addEventListener('click', () => setTab(b.dataset.tab)));
document.querySelectorAll('[data-nav]').forEach((b) =>
  b.addEventListener('click', () => invoke('medal_nav', { action: b.dataset.nav })),
);

// ---------- status do gravador ----------

let flashTimer;

function renderStatus() {
  const s = state.status;
  if (!s) return;
  const btn = $('#status');
  const text = $('#status-text');
  btn.dataset.state = s.state;
  const secs = state.config?.bufferSecs ?? 30;
  const labels = {
    idle: 'Buffer desligado',
    starting: 'Iniciando…',
    recording: `Buffer ${secs}s · ${s.backend ?? ''}`,
    error: 'Erro no buffer',
  };
  if (!btn.classList.contains('flash')) text.textContent = labels[s.state] ?? s.state;
  btn.title = s.error || s.detail || 'Ligar/desligar o replay buffer';
  $('#save-btn').disabled = s.state !== 'recording';
  for (const k of document.querySelectorAll('#hotkey-label, .hotkey-inline')) {
    k.textContent = s.hotkey || '—';
  }
  $('#hotkey-label').hidden = !s.hotkeyOk;

  const banner = $('#banner');
  if (s.state === 'error' && s.error) {
    banner.textContent = s.error;
    banner.hidden = false;
  } else if (s.hotkey && !s.hotkeyOk) {
    banner.textContent =
      `Não foi possível registrar o atalho global "${s.hotkey}". No Wayland, crie um atalho no seu ` +
      `ambiente (GNOME/KDE) que rode:  medalha --save-clip`;
    banner.hidden = false;
  } else {
    banner.hidden = true;
  }
}

function flashStatus(message) {
  const btn = $('#status');
  btn.classList.add('flash');
  $('#status-text').textContent = message;
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => {
    btn.classList.remove('flash');
    renderStatus();
  }, 2500);
}

$('#status').addEventListener('click', () => {
  const s = state.status?.state;
  invoke(s === 'recording' || s === 'starting' ? 'stop_buffer' : 'start_buffer');
});
$('#save-btn').addEventListener('click', () => invoke('save_clip'));

// ---------- clipes ----------

async function refreshClips() {
  try {
    state.clips = await invoke('list_clips');
  } catch (e) {
    toast(String(e), 'error');
    return;
  }
  $('#clip-count').textContent = state.clips.length || '';
  renderClips();
}

const refreshSoon = debounce(refreshClips, 600);

function renderClips() {
  const grid = $('#grid');
  const q = state.filter.trim().toLowerCase();
  const clips = q ? state.clips.filter((c) => c.name.toLowerCase().includes(q)) : state.clips;
  grid.replaceChildren(...clips.map(clipCard));
  $('#empty').hidden = state.clips.length > 0;
}

function clipCard(clip) {
  const img = el('img', { alt: '', loading: 'lazy' });
  const dur = el('span', { class: 'duration' });
  const thumb = el(
    'button',
    { class: 'thumb', title: 'Assistir', onclick: () => play(clip) },
    img,
    el('span', { class: 'play' }, icon(ICONS.play)),
    dur,
  );
  const applyMeta = (m) => {
    if (m.thumb) img.src = convertFileSrc(m.thumb);
    else img.remove();
    dur.textContent = formatDuration(m.duration);
    dur.hidden = m.duration == null;
  };
  const cached = state.meta.get(clip.path);
  if (cached) applyMeta(cached);
  else {
    dur.hidden = true;
    metaQueue.request(clip.path, thumb, applyMeta);
  }

  return el(
    'article',
    { class: 'card' },
    thumb,
    el(
      'div',
      { class: 'card-body' },
      el(
        'div',
        { class: 'card-text' },
        el('div', { class: 'card-title', title: clip.name }, clip.name),
        el('div', { class: 'card-sub' }, `${dateFmt.format(new Date(clip.modified))} · ${formatSize(clip.size)}`),
      ),
      el('button', { class: 'icon-btn', title: 'Mostrar na pasta', onclick: () => invoke('reveal_clip', { path: clip.path }) }, icon(ICONS.folder)),
      el('button', { class: 'icon-btn', title: 'Renomear', onclick: () => renameClip(clip) }, icon(ICONS.edit)),
      el('button', { class: 'icon-btn danger', title: 'Mover para a lixeira', onclick: () => deleteClip(clip) }, icon(ICONS.trash)),
    ),
  );
}

/** Gera miniaturas só para os cards visíveis, poucas por vez. */
const metaQueue = (() => {
  const pending = new Map(); // elemento -> { path, apply }
  const queue = [];
  let running = 0;
  const observer = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      observer.unobserve(e.target);
      const job = pending.get(e.target);
      pending.delete(e.target);
      if (job) queue.push(job);
    }
    pump();
  }, { rootMargin: '200px' });

  function pump() {
    while (running < 2 && queue.length) {
      const { path, apply } = queue.shift();
      running++;
      invoke('clip_meta', { path })
        .then((m) => {
          state.meta.set(path, m);
          apply(m);
        })
        .catch(() => {})
        .finally(() => {
          running--;
          pump();
        });
    }
  }

  return {
    request(path, element, apply) {
      pending.set(element, { path, apply });
      observer.observe(element);
    },
  };
})();

function play(clip) {
  const video = $('#player-video');
  $('#player-title').textContent = clip.name;
  video.src = convertFileSrc(clip.path);
  $('#player-open').onclick = () => invoke('open_clip', { path: clip.path });
  openModal($('#player'));
  video.play().catch(() => {});
}

$('#player').addEventListener('close', () => {
  const video = $('#player-video');
  video.pause();
  video.removeAttribute('src');
  video.load();
});

for (const b of document.querySelectorAll('[data-close]')) {
  b.addEventListener('click', () => b.closest('dialog').close());
}

/** Modal simples; resolve com o texto (ou true) ou null se cancelar. */
function ask({ title, text = '', value = null, ok = 'OK', danger = false }) {
  const dialog = $('#prompt');
  $('#prompt-title').textContent = title;
  $('#prompt-text').textContent = text;
  const input = $('#prompt-input');
  input.hidden = value === null;
  input.value = value ?? '';
  const okBtn = $('#prompt-ok');
  okBtn.textContent = ok;
  okBtn.style.background = danger ? 'var(--rec)' : '';
  okBtn.style.color = danger ? '#fff' : '';
  return new Promise((resolve) => {
    dialog.returnValue = '';
    dialog.addEventListener(
      'close',
      () => resolve(dialog.returnValue === 'ok' ? (value === null ? true : input.value) : null),
      { once: true },
    );
    openModal(dialog);
    if (value !== null) {
      input.focus();
      input.select();
    }
  });
}

async function renameClip(clip) {
  const name = await ask({ title: 'Renomear clipe', value: clip.name, ok: 'Renomear' });
  if (name === null || name.trim() === clip.name) return;
  try {
    await invoke('rename_clip', { path: clip.path, name });
    refreshClips();
  } catch (e) {
    toast(String(e), 'error');
  }
}

async function deleteClip(clip) {
  const yes = await ask({
    title: 'Mover para a lixeira?',
    text: `"${clip.name}" vai para a lixeira do sistema.`,
    ok: 'Mover',
    danger: true,
  });
  if (!yes) return;
  try {
    await invoke('delete_clip', { path: clip.path });
    refreshClips();
  } catch (e) {
    toast(String(e), 'error');
  }
}

$('#search').addEventListener('input', (e) => {
  state.filter = e.target.value;
  renderClips();
});
$('#open-dir').addEventListener('click', () => invoke('open_clips_dir'));

// ---------- configurações ----------

const form = $('#settings-form');

for (const opt of form.querySelectorAll('[data-linux]')) if (IS_WINDOWS) opt.hidden = true;
for (const opt of form.querySelectorAll('[data-windows]')) if (!IS_WINDOWS) opt.hidden = true;

function fillForm(cfg) {
  for (const [key, value] of Object.entries(cfg)) {
    const input = form.elements[key];
    if (!input) continue;
    if (input.type === 'checkbox') input.checked = Boolean(value);
    else input.value = value;
  }
}

function readForm() {
  const cfg = { ...state.config };
  for (const key of Object.keys(cfg)) {
    const input = form.elements[key];
    if (!input) continue;
    if (input.type === 'checkbox') cfg[key] = input.checked;
    else if (typeof cfg[key] === 'number') cfg[key] = Number(input.value) || 0;
    else cfg[key] = input.value;
  }
  return cfg;
}

async function openSettings() {
  state.config = await invoke('get_config');
  fillForm(state.config);
  const hint = $('#hotkey-hint');
  hint.hidden = state.status?.hotkeyOk !== false;
  hint.textContent = 'Este atalho não pôde ser registrado. No Wayland use um atalho do sistema rodando "medalha --save-clip".';
  openModal($('#settings'));
  renderDiagnostics();
}

async function renderDiagnostics() {
  const dl = $('#diag');
  dl.replaceChildren(el('dt', {}, 'Verificando'), el('dd', {}, '…'));
  try {
    const d = await invoke('diagnostics');
    const rows = [
      ['Versão', d.version],
      ['Sistema', d.platform],
      ['ffmpeg', d.ffmpeg ?? 'não encontrado'],
      ['gpu-screen-recorder', d.gsr ?? (IS_WINDOWS ? 'n/a' : 'não encontrado')],
      ['Gravador (automático)', d.backend],
      ['Pipeline ativo', state.status?.detail ?? '—'],
    ];
    dl.replaceChildren(...rows.flatMap(([k, v]) => [el('dt', {}, k), el('dd', {}, v)]));
  } catch (e) {
    dl.replaceChildren(el('dt', {}, 'Erro'), el('dd', {}, String(e)));
  }
}

$('#settings-btn').addEventListener('click', openSettings);

form.addEventListener('submit', async (e) => {
  if (e.submitter?.value !== 'save') return;
  e.preventDefault();
  try {
    state.config = await invoke('set_config', { config: readForm() });
    $('#settings').close();
    toast('Configurações salvas', 'ok');
    renderStatus();
    if (state.tab === 'clips') refreshClips();
  } catch (err) {
    toast(String(err), 'error');
  }
});

$('#pick-dir').addEventListener('click', async () => {
  const dir = await invoke('pick_folder');
  if (dir) form.elements.clipsDir.value = dir;
});

// Captura de atalho: clique no campo e aperte a combinação.
const hotkeyInput = $('#hotkey-input');
hotkeyInput.addEventListener('focus', () => hotkeyInput.classList.add('capturing'));
hotkeyInput.addEventListener('blur', () => hotkeyInput.classList.remove('capturing'));
hotkeyInput.addEventListener('keydown', (e) => {
  e.preventDefault();
  if (e.key === 'Escape') return hotkeyInput.blur();
  if (['Control', 'Shift', 'Alt', 'Meta', 'OS'].includes(e.key)) return;
  const parts = [];
  if (e.ctrlKey) parts.push('Ctrl');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  if (e.metaKey) parts.push('Super');
  parts.push(e.code);
  hotkeyInput.value = parts.join('+');
  hotkeyInput.blur();
});

// ---------- eventos do Rust ----------

listen('recorder-status', (e) => {
  state.status = e.payload;
  renderStatus();
});
listen('clip-saving', () => flashStatus('Salvando clipe…'));
listen('clip-saved', (e) => {
  flashStatus('Clipe salvo ✓');
  if (state.tab === 'clips') toast(`Clipe salvo: ${e.payload.name}`, 'ok');
  refreshClips();
});
listen('clip-error', (e) => {
  flashStatus('Falha ao salvar');
  toast(String(e.payload), 'error');
});
listen('clips-changed', () => refreshSoon());

// ---------- início ----------

(async () => {
  state.config = await invoke('get_config');
  state.status = await invoke('get_status');
  renderStatus();
  refreshClips();
  setTab('medal');
})();
