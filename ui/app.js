'use strict';

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (sel) => document.querySelector(sel);
const IS_WINDOWS = navigator.userAgent.includes('Windows');

const state = {
  clips: [],
  meta: new Map(), // path -> { thumb, duration }
  status: null,
  config: null,
  filter: '',
  game: '',
};

// Utilidades

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
  // Um <dialog> modal fica numa camada acima de tudo; o aviso vai junto para não sumir atrás dele.
  const box = $('#toasts');
  const host = [...document.querySelectorAll('dialog[open]')].pop() ?? document.body;
  if (box.parentElement !== host) host.append(box);
  box.append(t);
  setTimeout(() => t.remove(), kind === 'error' ? 7000 : 3500);
}

function debounce(fn, ms) {
  let id;
  return (...args) => {
    clearTimeout(id);
    id = setTimeout(() => fn(...args), ms);
  };
}

// Status do gravador

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
    recording: `Gravando · ${secs} s`,
    error: 'Buffer com erro',
  };
  if (!btn.classList.contains('flash')) text.textContent = labels[s.state] ?? s.state;
  btn.title = s.error || s.warning || (s.detail ? `Clique para desligar. ${s.detail}` : 'Clique para ligar o buffer');
  btn.classList.toggle('warn', Boolean(s.warning));
  $('#save-btn').disabled = s.state !== 'recording';
  for (const k of document.querySelectorAll('#hotkey-label, .hotkey-inline')) {
    k.textContent = s.hotkey || '—';
  }
  $('#hotkey-label').hidden = !s.hotkeyOk;

  const banner = $('#banner');
  if (s.state === 'error' && s.error) {
    banner.replaceChildren(s.error);
    if (s.action === 'install-gsr') banner.append(el('div', { class: 'banner-actions' }, installGsrButton()));
    banner.hidden = false;
  } else if (s.hotkey && !s.hotkeyOk) {
    banner.textContent =
      `Não foi possível registrar o atalho global "${s.hotkey}". No Wayland, crie um atalho no seu ` +
      `ambiente (GNOME/KDE) que rode:  medalha --save-clip`;
    banner.hidden = false;
  } else if (s.warning) {
    banner.textContent = s.warning;
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

let installingGsr = false;

function installGsrButton() {
  return el(
    'button',
    { class: 'primary', onclick: installGsr, disabled: installingGsr ? '' : null },
    installingGsr ? 'Instalando… (pode levar alguns minutos)' : 'Instalar gpu-screen-recorder',
  );
}

async function installGsr() {
  installingGsr = true;
  renderStatus();
  toast('Baixando o gpu-screen-recorder do Flathub…');
  try {
    await invoke('install_gsr');
    toast('gpu-screen-recorder instalado. Ligando o buffer…', 'ok');
  } catch (e) {
    toast(`Não deu para instalar: ${e}`, 'error');
  } finally {
    installingGsr = false;
    renderStatus();
  }
}

$('#status').addEventListener('click', () => {
  // Quando o erro tem correção (instalar o gsr), o aviso com o botão já está na tela; não tenta de novo.
  if (state.status?.state === 'error' && state.status.action) return;
  const s = state.status?.state;
  invoke(s === 'recording' || s === 'starting' ? 'stop_buffer' : 'start_buffer');
});
$('#save-btn').addEventListener('click', () => invoke('save_clip'));

// Clipes

/** `Jogo_2026-10-23_21-00-00` → `Jogo` (o Rust põe o jogo em foco no nome ao salvar). */
const GAME_RE = /^(.+?)_\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}/;

function gameOf(name) {
  const m = GAME_RE.exec(name);
  return m && m[1] !== 'Medalha' ? m[1] : null;
}

function renderGameFilter() {
  const games = [...new Set(state.clips.map((c) => gameOf(c.name)).filter(Boolean))].sort((a, b) =>
    a.localeCompare(b, 'pt-BR'),
  );
  if (state.game && !games.includes(state.game)) state.game = '';
  const select = $('#game-filter');
  select.replaceChildren(
    el('option', { value: '' }, 'Todos os jogos'),
    ...games.map((g) => el('option', { value: g }, g)),
  );
  select.value = state.game;
  select.hidden = games.length === 0;
}

async function refreshClips() {
  try {
    state.clips = await invoke('list_clips');
  } catch (e) {
    toast(String(e), 'error');
    return;
  }
  $('#clip-count').textContent = state.clips.length || '';
  renderGameFilter();
  renderClips();
}

const refreshSoon = debounce(refreshClips, 600);

function renderClips() {
  const grid = $('#grid');
  const q = state.filter.trim().toLowerCase();
  const clips = state.clips.filter(
    (c) => (!q || c.name.toLowerCase().includes(q)) && (!state.game || gameOf(c.name) === state.game),
  );
  metaQueue.reset();
  grid.replaceChildren(...clips.map(clipCard));
  $('#empty').hidden = state.clips.length > 0;
  $('#no-match').hidden = state.clips.length === 0 || clips.length > 0;
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
        el(
          'div',
          { class: 'card-sub' },
          [gameOf(clip.name), dateFmt.format(new Date(clip.modified)), formatSize(clip.size)].filter(Boolean).join(' · '),
        ),
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
          // Sem miniatura (arquivo ainda sendo escrito, por exemplo): tenta de novo na próxima vez.
          if (m.thumb) state.meta.set(path, m);
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
    /** Esquece os cards da renderização anterior (já saíram da página). */
    reset() {
      observer.disconnect();
      pending.clear();
    },
  };
})();

function play(clip) {
  const video = $('#player-video');
  $('#player-title').textContent = clip.name;
  Object.assign(trim, { clip, duration: 0, start: 0, end: 0 });
  renderTrim();
  video.src = convertFileSrc(clip.path);
  $('#player-open').onclick = () => invoke('open_clip', { path: clip.path });
  $('#player').showModal();
  video.play().catch(() => {});
}

$('#player').addEventListener('close', () => {
  const video = $('#player-video');
  video.pause();
  video.removeAttribute('src');
  video.load();
  trim.clip = null;
});

// Corte

const MIN_TRIM = 0.5;
const trim = { clip: null, duration: 0, start: 0, end: 0, saving: false };
const playerVideo = $('#player-video');
const trimTrack = $('#trim-track');

function formatTime(t) {
  const m = Math.floor(t / 60);
  return `${m}:${(t - m * 60).toFixed(1).padStart(4, '0')}`;
}

function trimmed() {
  return trim.duration > 0 && (trim.start > 0.05 || trim.end < trim.duration - 0.05);
}

function renderTrim() {
  const d = trim.duration;
  $('#trim').hidden = !(d > MIN_TRIM);
  if ($('#trim').hidden) return;
  const a = (trim.start / d) * 100;
  const b = (trim.end / d) * 100;
  Object.assign($('#trim-sel').style, { left: `${a}%`, width: `${b - a}%` });
  trimTrack.querySelector('[data-edge="start"]').style.left = `${a}%`;
  trimTrack.querySelector('[data-edge="end"]').style.left = `${b}%`;
  $('#trim-label').textContent =
    `${formatTime(trim.start)} – ${formatTime(trim.end)} · ${(trim.end - trim.start).toFixed(1)} s`;
  $('#trim-save').disabled = trim.saving || !trimmed();
  $('#trim-reset').disabled = trim.saving || !trimmed();
}

function renderPlayhead() {
  if (trim.duration > 0) $('#trim-head').style.left = `${(playerVideo.currentTime / trim.duration) * 100}%`;
}

function setEdge(edge, t) {
  if (!(trim.duration > MIN_TRIM)) return;
  if (edge === 'start') trim.start = Math.max(0, Math.min(t, trim.end - MIN_TRIM));
  else trim.end = Math.min(trim.duration, Math.max(t, trim.start + MIN_TRIM));
  renderTrim();
}

playerVideo.addEventListener('loadedmetadata', () => {
  if (!trim.clip) return;
  trim.duration = Number.isFinite(playerVideo.duration) ? playerVideo.duration : 0;
  trim.start = 0;
  trim.end = trim.duration;
  renderTrim();
  renderPlayhead();
});

playerVideo.addEventListener('timeupdate', () => {
  // Com um trecho marcado, o player fica em loop nele (prévia do corte).
  if (trimmed() && !playerVideo.paused && playerVideo.currentTime >= trim.end) playerVideo.currentTime = trim.start;
  renderPlayhead();
});

function timeAt(e) {
  const r = trimTrack.getBoundingClientRect();
  return Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * trim.duration;
}

trimTrack.addEventListener('pointerdown', (e) => {
  if (!(trim.duration > 0)) return;
  const handle = e.target.closest('.trim-handle');
  if (!handle) {
    playerVideo.currentTime = timeAt(e);
    renderPlayhead();
    return;
  }
  e.preventDefault();
  const edge = handle.dataset.edge;
  handle.setPointerCapture(e.pointerId);
  const move = (ev) => {
    setEdge(edge, timeAt(ev));
    playerVideo.currentTime = edge === 'start' ? trim.start : trim.end;
  };
  handle.addEventListener('pointermove', move);
  handle.addEventListener('lostpointercapture', () => handle.removeEventListener('pointermove', move), { once: true });
});

$('#trim-in').addEventListener('click', () => setEdge('start', playerVideo.currentTime));
$('#trim-out').addEventListener('click', () => setEdge('end', playerVideo.currentTime));
$('#trim-reset').addEventListener('click', () => {
  trim.start = 0;
  trim.end = trim.duration;
  renderTrim();
});

$('#trim-save').addEventListener('click', async () => {
  const button = $('#trim-save');
  trim.saving = true;
  button.textContent = 'Salvando…';
  renderTrim();
  try {
    const clip = await invoke('trim_clip', { path: trim.clip.path, start: trim.start, end: trim.end });
    toast(`Corte salvo: ${clip.name}`, 'ok');
    refreshClips();
  } catch (e) {
    toast(String(e), 'error');
  } finally {
    trim.saving = false;
    button.textContent = 'Salvar corte';
    renderTrim();
  }
});

document.addEventListener('keydown', (e) => {
  if (!$('#player').open || e.ctrlKey || e.altKey || e.metaKey || e.target.matches('input, textarea, select')) return;
  const key = e.key.toLowerCase();
  if (key === 'i') setEdge('start', playerVideo.currentTime);
  else if (key === 'o') setEdge('end', playerVideo.currentTime);
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
  okBtn.classList.toggle('danger', danger);
  return new Promise((resolve) => {
    dialog.returnValue = '';
    dialog.addEventListener(
      'close',
      () => resolve(dialog.returnValue === 'ok' ? (value === null ? true : input.value) : null),
      { once: true },
    );
    dialog.showModal();
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
$('#game-filter').addEventListener('change', (e) => {
  state.game = e.target.value;
  renderClips();
});
$('#open-dir').addEventListener('click', () => invoke('open_clips_dir'));

// Balão (i) da aba Clipes

const infoBtn = $('#info-btn');
const infoPop = $('#info-pop');

function setInfo(open) {
  infoPop.hidden = !open;
  infoBtn.setAttribute('aria-expanded', String(open));
}

infoBtn.addEventListener('click', () => setInfo(infoPop.hidden));
$('#info-medal').addEventListener('click', () => {
  setInfo(false);
  invoke('open_medal');
});
document.addEventListener('click', (e) => {
  if (!infoPop.hidden && !e.target.closest('.info')) setInfo(false);
});
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !infoPop.hidden) {
    setInfo(false);
    infoBtn.focus();
  }
});

// Configurações

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
  $('#settings').showModal();
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
      ['Log', d.log],
    ];
    dl.replaceChildren(...rows.flatMap(([k, v]) => [el('dt', {}, k), el('dd', {}, v)]));
    if (d.platform.includes('wayland')) {
      // O plugin registra o atalho via XWayland: só dispara com uma janela X11 em foco.
      const hint = $('#hotkey-hint');
      hint.hidden = false;
      hint.textContent =
        'No Wayland o atalho global só funciona com o foco em janelas XWayland (a maioria dos jogos). ' +
        'Para funcionar sempre, crie um atalho do sistema que rode "medalha --save-clip".';
    }
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
    refreshClips();
  } catch (err) {
    toast(String(err), 'error');
  }
});

$('#pick-dir').addEventListener('click', async () => {
  const dir = await invoke('pick_folder');
  if (dir) form.elements.clipsDir.value = dir;
});

// Captura do atalho. O atual fica solto enquanto isso; senão apertá-lo salvaria um clipe.
const hotkeyInput = $('#hotkey-input');
hotkeyInput.addEventListener('focus', () => {
  hotkeyInput.classList.add('capturing');
  invoke('suspend_hotkey', { suspended: true });
});
hotkeyInput.addEventListener('blur', () => {
  hotkeyInput.classList.remove('capturing');
  invoke('suspend_hotkey', { suspended: false });
});
hotkeyInput.addEventListener('keydown', (e) => {
  if (e.key === 'Tab' || e.key === 'Enter') return; // navegar e confirmar continuam funcionando
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

// Atualização

const UPDATE_HINTS = {
  appimage: 'O AppImage é trocado pelo novo e o Medalha abre de novo sozinho.',
  deb: 'O sistema vai pedir sua senha para instalar o pacote. Depois o Medalha abre de novo.',
  rpm: 'O sistema vai pedir sua senha para instalar o pacote. Depois o Medalha abre de novo.',
  installer: 'O Medalha fecha e o instalador abre.',
  manual: 'Este jeito de instalar não se atualiza sozinho. Baixe a versão nova na página dela.',
};
let update = null;
let installing = false;

function renderUpdate() {
  const btn = $('#update-btn');
  btn.hidden = !update;
  if (update) btn.textContent = `Atualizar para ${update.version}`;
}

/** As notas vêm do CHANGELOG em Markdown; aqui só tiramos a marcação. */
function plainNotes(md) {
  return md
    .replace(/^#+\s*v?\d+\.\d+\.\d+\s*$/m, '') // o título da seção repete a versão
    .replace(/\*\*(.+?)\*\*/g, '$1')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/^#+\s*/gm, '')
    .trim();
}

function setUpdateStatus(text, error = false) {
  const status = $('#update-status');
  status.textContent = text;
  status.classList.toggle('error', error);
}

function openUpdate() {
  if (!update) return;
  $('#update-title').textContent = `Medalha ${update.version}`;
  $('#update-notes').textContent = plainNotes(update.notes) || 'Sem notas para esta versão.';
  $('#update-go').textContent = update.method === 'manual' ? 'Abrir página da versão' : 'Atualizar agora';
  $('#update-go').disabled = installing;
  if (!installing) setUpdateStatus(UPDATE_HINTS[update.method] ?? '');
  $('#update').showModal();
}

$('#update-btn').addEventListener('click', openUpdate);

$('#update-go').addEventListener('click', async () => {
  if (update.method === 'manual') {
    invoke('open_release_page');
    return;
  }
  installing = true;
  $('#update-go').disabled = true;
  setUpdateStatus('Baixando…');
  try {
    await invoke('install_update');
    setUpdateStatus('Instalado. Abrindo a versão nova…');
  } catch (e) {
    installing = false;
    $('#update-go').disabled = false;
    setUpdateStatus(`Não deu para atualizar: ${e}`, true);
  }
});

listen('update-progress', (e) => {
  const { downloaded, total } = e.payload;
  const mb = (n) => (n / 1e6).toFixed(0);
  setUpdateStatus(`Baixando… ${Math.floor((downloaded / total) * 100)}% (${mb(downloaded)} de ${mb(total)} MB)`);
});

listen('update-available', (e) => {
  update = e.payload;
  renderUpdate();
});

$('#check-update').addEventListener('click', async () => {
  const text = $('#update-check-text');
  text.textContent = 'Procurando…';
  try {
    update = await invoke('check_update');
    renderUpdate();
    text.textContent = update ? `Versão ${update.version} disponível.` : 'Você está na versão mais nova.';
  } catch (e) {
    text.textContent = String(e);
  }
});

// Eventos do Rust

listen('recorder-status', (e) => {
  state.status = e.payload;
  renderStatus();
});
listen('clip-saving', () => flashStatus('Salvando clipe…'));
listen('clip-saved', (e) => {
  flashStatus('Clipe salvo');
  toast(`Clipe salvo: ${e.payload.name}`, 'ok');
  refreshClips();
});
listen('clip-error', (e) => {
  flashStatus('Falha ao salvar');
  toast(String(e.payload), 'error');
});
listen('clips-changed', () => refreshSoon());

// Início

(async () => {
  state.config = await invoke('get_config');
  state.status = await invoke('get_status');
  update = await invoke('get_update');
  renderUpdate();
  renderStatus();
  refreshClips();
})();
