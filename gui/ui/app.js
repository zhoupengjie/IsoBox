'use strict';

// ---------- Tauri 接口 ----------
const tauri = window.__TAURI__;
const invoke = tauri.core.invoke;
const pickFile = (opts) => tauri.dialog.open(opts);

// ---------- 状态 ----------
const S = {
  page: 'apps',
  ov: null,          // 后端 overview 的结果
  info: null,        // 版本、配置位置
  menu: null,        // 当前展开“更多”菜单的软件名
  add: null,         // 添加软件向导的状态
  protectDrive: '',  // 保护页里选中的盘
};

// ---------- 小工具 ----------
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const gb = (b) => {
  const g = b / 1073741824;
  return g >= 1024 ? (g / 1024).toFixed(1) + ' TB' : g >= 1 ? g.toFixed(0) + ' GB' : (b / 1048576).toFixed(0) + ' MB';
};
const mb = (b) => (b >= 1073741824 ? (b / 1073741824).toFixed(1) + ' GB' : (b / 1048576).toFixed(1) + ' MB');
const KIND = { Nsis: 'NSIS', Inno: 'Inno Setup', Msi: 'MSI', Unknown: '未知类型的' };
const when = (sec) => new Date(sec * 1000).toLocaleString('zh-CN', { hour12: false });
const sysDrive = () => (S.ov?.drives.find((d) => d.system)?.letter) || 'C';

const ICONS = {
  apps: '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/>',
  lib: '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 1.66 3.6 3 8 3s8-1.34 8-3V5"/><path d="M4 12c0 1.66 3.6 3 8 3s8-1.34 8-3"/>',
  shield: '<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>',
  settings: '<path d="M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3M14 2v4M8 10v4M16 18v4"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  more: '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
  folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
  back: '<path d="m12 19-7-7 7-7M19 12H5"/>',
  ok: '<circle cx="12" cy="12" r="9"/><path d="m8.5 12 2.5 2.5 4.5-5"/>',
  warn: '<path d="m21.7 18-8-14a2 2 0 0 0-3.4 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3Z"/><path d="M12 9v4M12 17h.01"/>',
  drive: '<path d="M22 12H2"/><path d="M5.5 5.1 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.5-6.9A2 2 0 0 0 16.8 4H7.2a2 2 0 0 0-1.7 1.1z"/><path d="M6 16h.01M10 16h.01"/>',
  lock: '<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
  box: '<path d="M21 8a2 2 0 0 0-1-1.7l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.7l7 4a2 2 0 0 0 2 0l7-4a2 2 0 0 0 1-1.7Z"/><path d="m3.3 7 8.7 5 8.7-5M12 22V12"/>',
  refresh: '<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/>',
};
const icon = (name) => `<svg class="i" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`;

function toast(msg, kind = '') {
  const el = document.createElement('div');
  el.className = 'toast ' + kind;
  el.textContent = msg;
  document.getElementById('toasts').appendChild(el);
  setTimeout(() => el.remove(), kind === 'error' ? 7000 : 3500);
}

// ---------- 数据 ----------
// force 为 false 时（定时刷新），数据没变就不重新渲染，避免打断正在进行的点击和悬停
async function refresh(force = true) {
  try {
    const ov = await invoke('overview');
    const json = JSON.stringify(ov);
    if (!force && json === S.lastJson) return;
    S.lastJson = json;
    S.ov = ov;
  } catch (e) {
    toast('读取状态失败：' + e, 'error');
  }
  render();
  invoke('refresh_tray').catch(() => {});
}

// 以管理员身份运行 isobox-setup：先显示等待框，结束后显示结果
async function runAdmin(title, args, { onSuccess } = {}) {
  showDialog(`<div class="busy"><div class="spinner"></div><div><h3>${esc(title)}</h3>
    <div class="muted small">请在弹出的 Windows 窗口里确认管理员权限。文件多时可能需要几分钟。</div></div></div>`);
  let r;
  try {
    r = await invoke('admin', { args });
  } catch (e) {
    r = { code: -1, output: String(e), cancelled: false };
  }
  if (r.cancelled) {
    closeDialog();
    toast('已取消');
    return false;
  }
  const ok = r.code === 0;
  showDialog(`<h3>${ok ? '完成' : '没有成功'}：${esc(title)}</h3>
    <div class="muted small">${ok ? '下面是执行记录。' : '下面是执行记录，可以根据提示处理后再试。'}</div>
    <pre>${esc(r.output.trim() || '（没有输出）')}</pre>
    <div class="footer-actions"><button class="btn primary" data-action="close-dialog">好</button></div>`);
  await refresh();
  if (ok && onSuccess) onSuccess();
  return ok;
}

function showDialog(html) {
  document.getElementById('overlay').innerHTML = `<div class="dialog">${html}</div>`;
}
function closeDialog() {
  document.getElementById('overlay').innerHTML = '';
}

// ---------- 渲染 ----------
const PAGES = [
  ['apps', '软件', 'apps'],
  ['libs', '库', 'lib'],
  ['protect', '保护状态', 'shield'],
  ['settings', '设置', 'settings'],
];

function render() {
  const current = S.page === 'add' ? 'apps' : S.page;
  document.getElementById('nav').innerHTML = PAGES.map(([id, label, ic]) =>
    `<button class="nav-item ${current === id ? 'active' : ''}" data-action="nav" data-page="${id}">${icon(ic)}${label}</button>`).join('');
  document.getElementById('side-foot').textContent = S.info ? `版本 ${S.info.version}` : '';
  const main = document.getElementById('main');
  if (!S.ov) {
    main.innerHTML = '<div class="loading">正在读取状态…</div>';
    return;
  }
  const fn = { apps: pageApps, add: pageAdd, libs: pageLibs, protect: pageProtect, settings: pageSettings }[S.page];
  main.innerHTML = fn();
}

function statusBanner() {
  const ov = S.ov;
  if (!ov.account.exists) {
    return `<div class="banner warn">${icon('warn')}<span>还没有隔离账户。安装后才能添加和运行隔离软件。</span>
      <span class="spacer"></span><button class="btn" data-action="install">立即安装</button></div>`;
  }
  // 账户建了，但安装中途失败：补上没做完的步骤（install 可以重复运行）
  if (ov.account.gap) {
    return `<div class="banner warn">${icon('warn')}<span>隔离账户没有完全建好：${esc(ov.account.gap)}。</span>
      <span class="spacer"></span><button class="btn" data-action="repair">修复</button></div>`;
  }
  const prot = ov.protected.filter((p) => p.active).map((p) => p.path);
  return `<div class="banner ok">${icon('ok')}<span>隔离账户正常 · ${prot.length ? '已保护 ' + esc(prot.join('、')) : '没有受保护的盘'}
    · ${ov.theme_watcher ? '深浅色同步中' : '深浅色同步将在启动软件后运行'}</span></div>`;
}

function kindLabel(a) {
  return { library: `${a.library} 盘库`, legacy: '旧位置', installed: '原版安装', appcontainer: 'AppContainer' }[a.kind] || a.kind;
}

function appIcon(a) {
  return a.icon ? `<div class="app-icon"><img src="${a.icon}" alt=""></div>`
    : `<div class="app-icon">${esc((a.display || a.name).slice(0, 1).toUpperCase())}</div>`;
}

function pageApps() {
  const ov = S.ov;
  let html = statusBanner();
  html += `<div class="page-head"><h1>已隔离的软件</h1><span class="spacer"></span>
    <button class="btn ghost icon" title="刷新" data-action="refresh">${icon('refresh')}</button>
    <button class="btn primary" data-action="add-start" ${ov.account.exists ? '' : 'disabled'}>${icon('plus')}添加软件</button></div>`;
  if (!ov.apps.length) {
    return html + `<div class="list"><div class="empty"><div class="big">还没有隔离软件</div>
      点“添加软件”，选择一个安装包，IsoBox 会把它装进库里。</div></div>`;
  }
  html += '<div class="list">';
  for (const a of ov.apps) {
    const sub = [
      `<span class="dot ${a.running ? 'on' : ''}"></span>${a.running ? '运行中' : '未运行'}`,
      kindLabel(a),
      a.startup ? '开机启动' : '',
      a.problem ? '⚠ ' + esc(a.problem) : '',
    ].filter(Boolean).join(' · ');
    html += `<div class="row">${appIcon(a)}<div class="grow"><div class="title">${esc(a.display)}</div><div class="sub">${sub}</div></div>
      <div class="actions"><button class="btn" data-action="launch" data-name="${esc(a.name)}" ${a.problem ? 'disabled' : ''}>${a.running ? '打开' : '启动'}</button>
      <button class="btn icon" title="更多" data-action="menu" data-name="${esc(a.name)}">${icon('more')}</button></div>
      ${S.menu === a.name ? appMenu(a) : ''}</div>`;
  }
  return html + '</div>';
}

function appMenu(a) {
  const n = esc(a.name);
  let html = `<div class="menu" data-keep-menu>
    <button data-action="toggle-startup" data-name="${n}" data-on="${a.startup ? 0 : 1}">${a.startup ? '取消开机启动' : '开机自动启动'}</button>`;
  if (a.kind === 'library' || a.kind === 'legacy') {
    const targets = S.ov.drives.filter((d) => d.letter !== a.library && (d.has_library || !d.library_blocker));
    if (targets.length) {
      html += '<hr><div class="label">移动到其他库</div>';
      for (const d of targets) {
        html += `<button data-action="move" data-name="${n}" data-drive="${d.letter}">${icon('drive')}${d.letter} 盘${d.has_library ? '' : '（新建库）'} · 剩余 ${gb(d.free)}</button>`;
      }
    }
  }
  if (a.exe) html += `<hr><button data-action="open-path" data-path="${esc(a.exe)}">${icon('folder')}打开程序所在位置</button>`;
  html += `<hr><button class="danger" data-action="remove-app" data-name="${n}" data-display="${esc(a.display)}">移除</button></div>`;
  return html;
}

// ---------- 添加软件向导 ----------
function startAdd() {
  const libs = S.ov.libraries.map((l) => l.drive);
  S.add = { step: 1, installer: '', preview: null, analyzing: false, name: '', display: '', method: 'quick', exe: '',
    library: libs.includes(sysDrive()) ? sysDrive() : (libs[0] || sysDrive()), startup: false, error: '' };
  S.page = 'add';
  render();
}

function libraryOptions(selected) {
  const opts = [];
  for (const d of S.ov.drives) {
    if (d.has_library) opts.push([d.letter, `${d.letter} 盘库 · 剩余 ${gb(d.free)}`]);
    else if (!d.library_blocker) opts.push([d.letter, `${d.letter} 盘（新建库）· 剩余 ${gb(d.free)}`]);
  }
  return opts.map(([v, t]) => `<option value="${v}" ${v === selected ? 'selected' : ''}>${esc(t)}</option>`).join('');
}

function pageAdd() {
  const A = S.add;
  let html = `<div class="page-head"><button class="btn ghost icon" title="返回" data-action="nav" data-page="apps">${icon('back')}</button>
    <h1>添加软件</h1><span class="spacer"></span><span class="steps">第 ${A.step} 步，共 2 步</span></div>`;
  if (A.step === 1) {
    return html + `<div class="card"><div class="empty">${A.analyzing
      ? '<div class="busy" style="justify-content:center"><div class="spinner"></div><div>正在分析安装包，可能需要十几秒…</div></div>'
      : `<div class="big">选择软件的官方安装包</div><div style="margin:4px 0 16px">支持 .exe 和 .msi。IsoBox 会先分析能否快速安装。</div>
         <button class="btn primary" data-action="add-pick">${icon('folder')}选择安装包</button>`}</div></div>`;
  }
  const P = A.preview;
  const quickOk = P.unpackable;
  const exeOptions = (P.candidates || []).map((c) =>
    `<option value="${esc(c.file)}" ${c.file === A.exe ? 'selected' : ''}>${esc(c.rel_path)} · ${mb(c.size)}${c.likely_helper ? '（像辅助程序）' : ''}</option>`).join('');
  html += `<div class="file">${icon('box')}<span class="name">${esc(A.installer)}</span><span class="muted small">${esc(KIND[P.kind] || P.kind)} 安装包</span></div>
    <div class="field"><span class="field-label">安装方式</span>
      <label class="option ${A.method === 'quick' ? 'checked' : ''} ${quickOk ? '' : 'disabled'}">
        <input type="radio" name="method" value="quick" data-bind="method" ${A.method === 'quick' ? 'checked' : ''} ${quickOk ? '' : 'disabled'}>
        <span><span class="t">快速安装（推荐）</span><br><span class="muted small">${quickOk
          ? `拆包放进库，不运行安装程序，不写注册表。解包后约 ${mb(P.size)}`
          : esc(P.error || '这个安装包不能拆包')}</span></span></label>
      <label class="option ${A.method === 'installer' ? 'checked' : ''}">
        <input type="radio" name="method" value="installer" data-bind="method" ${A.method === 'installer' ? 'checked' : ''}>
        <span><span class="t">使用原版安装程序</span><br><span class="muted small">在原版安装向导里安装${P.kind !== 'Unknown' ? '，目录已预填为所选库' : '，安装目录需要你自己选库里的位置'}。弹出要求管理员密码的窗口时请选“否”</span></span></label>
    </div>
    <div class="grid2">
      <div class="field"><label>安装到</label><select data-bind="library">${libraryOptions(A.library)}</select></div>
      ${A.method === 'quick' ? `<div class="field"><label>主程序</label><select data-bind="exe">${exeOptions}</select></div>` : '<div></div>'}
      <div class="field"><label>英文名（用作文件夹名）</label><input type="text" data-bind="name" value="${esc(A.name)}" spellcheck="false"></div>
      <div class="field"><label>显示名</label><input type="text" data-bind="display" value="${esc(A.display)}" placeholder="${esc(A.name)}"></div>
    </div>
    <label class="check"><input type="checkbox" data-bind="startup" ${A.startup ? 'checked' : ''}>开机自动启动</label>
    ${A.error ? `<div class="err">${esc(A.error)}</div>` : ''}
    <div class="footer-actions"><button class="btn" data-action="nav" data-page="apps">取消</button>
      <button class="btn primary" data-action="add-run">开始安装</button></div>`;
  return html;
}

async function addPick() {
  const path = await pickFile({ multiple: false, directory: false, filters: [{ name: '安装包', extensions: ['exe', 'msi'] }] });
  if (!path) return;
  const A = S.add;
  A.installer = path;
  A.analyzing = true;
  render();
  try {
    const p = await invoke('preview_installer', { path });
    A.preview = p;
    A.name = p.guessed_name;
    A.method = p.unpackable ? 'quick' : 'installer';
    A.exe = (p.candidates.find((c) => !c.likely_helper) || p.candidates[0] || {}).file || '';
    A.step = 2;
  } catch (e) {
    toast('分析安装包失败：' + e, 'error');
  }
  A.analyzing = false;
  render();
}

async function addRun() {
  const A = S.add;
  A.error = '';
  if (!/^[a-z0-9_-]{1,40}$/.test(A.name)) A.error = '英文名只能用小写字母、数字、短横线和下划线，例如 baidunetdisk';
  else if (S.ov.apps.some((x) => x.name === A.name)) A.error = `已经有一个叫 ${A.name} 的软件，请换个英文名`;
  else if (/["%]/.test(A.display)) A.error = '显示名里不能有双引号或百分号';
  render(); // 清掉上一次的错误提示
  if (A.error) return;
  const display = A.display.trim() || A.name;
  const args = ['add', A.name, A.installer, '--library', A.library, '--display', display];
  if (A.method === 'quick') args.push('--exe', A.exe);
  else args.push('--install');
  if (A.startup) args.push('--startup');
  await runAdmin(`安装 ${display}`, args, { onSuccess: () => { S.page = 'apps'; render(); } });
}

// ---------- 库 ----------
function pageLibs() {
  const ov = S.ov;
  let html = `<div class="page-head"><h1>库</h1></div>
    <div class="muted small" style="margin:-8px 0 14px">每个盘的库都在固定位置 Program Files\\IsoBox。隔离账户只能写库里的 apps 目录。</div>`;
  if (!ov.libraries.length) {
    html += '<div class="list"><div class="empty">还没有库。安装隔离账户时会自动在系统盘建库。</div></div>';
  } else {
    html += '<div class="list">';
    for (const l of ov.libraries) {
      const names = l.apps.map((n) => ov.apps.find((a) => a.name === n)?.display || n);
      html += `<div class="row"><div class="app-icon">${icon('drive')}</div><div class="grow">
        <div class="title">${l.drive} 盘</div><div class="sub">${esc(l.root)} · ${names.length ? esc(names.join('、')) : '空'}</div></div>
        <span class="muted small">剩余 ${gb(l.free)}</span>
        <div class="actions"><button class="btn ghost icon" title="打开" data-action="open-path" data-path="${esc(l.root)}">${icon('folder')}</button>
        ${l.apps.length ? '' : `<button class="btn" data-action="lib-remove" data-drive="${l.drive}">移除</button>`}</div></div>`;
    }
    html += '</div>';
  }
  const others = ov.drives.filter((d) => !d.has_library);
  if (others.length) {
    html += '<h2>其他盘</h2><div class="list">';
    for (const d of others) {
      html += `<div class="row ${d.library_blocker ? 'disabled' : ''}"><div class="app-icon">${icon('drive')}</div><div class="grow">
        <div class="title">${d.letter} 盘 ${d.label ? '· ' + esc(d.label) : ''}</div>
        <div class="sub">${esc(d.kind)}${d.ready ? ` · ${esc(d.fs)} · 剩余 ${gb(d.free)}` : ''}${d.library_blocker ? ' · 不能建库：' + esc(d.library_blocker) : ''}</div></div>
        ${d.library_blocker || !S.ov.account.exists ? '' : `<button class="btn" data-action="lib-add" data-drive="${d.letter}">${icon('plus')}在此建库</button>`}</div>`;
    }
    html += '</div>';
  }
  return html;
}

// ---------- 保护状态 ----------
function pageProtect() {
  const ov = S.ov;
  const active = ov.protected.filter((p) => p.active);
  const last = ov.backups[0];
  let html = `<div class="page-head"><h1>保护状态</h1></div>
    <div class="metrics">
      <div class="metric"><div class="k">隔离账户</div><div class="v">${ov.account.exists ? '正常' : '未安装'}</div></div>
      <div class="metric"><div class="k">受保护的位置</div><div class="v">${active.length} 个</div></div>
      <div class="metric"><div class="k">最近一次权限备份</div><div class="v" style="font-size:14px;padding-top:4px">${last ? esc(when(last.time)) : '无'}</div></div>
    </div>
    <h2>隔离软件读不到的位置</h2><div class="list">
    <div class="row"><div class="app-icon">${icon('user')}</div><div class="grow"><div class="title">你的用户目录</div>
      <div class="sub">桌面、文档、AppData 等，Windows 默认就不对其他账户开放</div></div><span class="muted small">系统默认</span></div>`;
  for (const p of ov.protected) {
    html += `<div class="row"><div class="app-icon">${icon('lock')}</div><div class="grow"><div class="title">${esc(p.path)}</div>
      <div class="sub">${p.active ? '已禁止隔离账户访问' : '在配置里，但禁止规则没有生效'}</div></div>
      ${ov.account.exists ? `<button class="btn" data-action="protect-remove" data-path="${esc(p.path)}">撤销</button>` : ''}</div>`;
  }
  html += '</div>';
  if (ov.account.exists) {
    const candidates = ov.drives.filter((d) => !d.protect_blocker && !d.protected);
    html += `<h2>添加保护</h2><div class="card"><div class="grid2">
      <div><div class="field-label">保护一整个盘</div><div style="display:flex;gap:8px">
        <select data-bind-protect>${candidates.length ? candidates.map((d) => `<option value="${d.letter}:\\" ${S.protectDrive === d.letter + ':\\' ? 'selected' : ''}>${d.letter} 盘 · ${esc(d.label || d.kind)}</option>`).join('') : '<option value="">没有可以保护的盘</option>'}</select>
        <button class="btn" data-action="protect-drive" ${candidates.length ? '' : 'disabled'}>保护</button></div></div>
      <div><div class="field-label">保护一个文件夹，例如系统盘上的项目目录</div>
        <button class="btn" data-action="protect-folder">${icon('folder')}选择文件夹</button></div>
    </div><div class="hint">修改前会自动备份这个位置的权限。文件多的盘需要几分钟。</div></div>`;
  }
  const removable = ov.drives.filter((d) => d.ready && (d.kind === '可移动磁盘' || d.kind === '外接硬盘'));
  html += `<h2>需要知道的限制</h2><div class="note">
    隔离软件和你在同一个桌面上，IsoBox 防不住截屏、键盘记录和读取剪贴板。<br>
    插着的 U 盘、存储卡多为 FAT32 或 exFAT 格式，没有权限控制，隔离软件也能读到${removable.length ? `（当前有：${removable.map((d) => d.letter + ' 盘').join('、')}）` : ''}。<br>
    强制要求管理员权限的安装包，没法装进隔离账户。</div>`;
  if (ov.backups.length) {
    html += '<h2>权限备份</h2><div class="list">';
    for (const b of ov.backups.slice(0, 8)) {
      html += `<div class="row"><div class="grow"><div class="title">${esc(b.target)}</div><div class="sub">${esc(when(b.time))}</div></div>
        <button class="btn" data-action="restore" data-dir="${esc(b.dir)}" data-target="${esc(b.target)}">还原</button></div>`;
    }
    html += '</div>';
  }
  return html;
}

// ---------- 设置 ----------
function pageSettings() {
  const ov = S.ov;
  const info = S.info || {};
  return `<div class="page-head"><h1>设置</h1></div>
    <div class="list">
      <div class="row"><div class="grow"><div class="title">交换文件夹</div><div class="sub">唯一你和隔离软件都能读写的地方：${esc(ov.exchange_dir || '未设置')}</div></div>
        <div class="actions"><button class="btn" data-action="open-path" data-path="${esc(ov.exchange_dir || '')}" ${ov.exchange_exists ? '' : 'disabled'}>打开</button>
        <button class="btn" data-action="exchange-change" ${ov.account.exists ? '' : 'disabled'}>更改</button></div></div>
      <div class="row"><div class="grow"><div class="title">开机启动 IsoBox 托盘</div><div class="sub">托盘可以快速启动隔离软件，并让它们跟随系统的深浅色</div></div>
        <input type="checkbox" data-action="tray-autostart" ${ov.tray_autostart ? 'checked' : ''}></div>
      <div class="row"><div class="grow"><div class="title">深浅色同步</div><div class="sub">${ov.theme_watcher ? '运行中：切换系统深浅色或强调色时，隔离软件会跟着变' : '未运行：启动任意隔离软件或打开托盘后自动开始'}</div></div></div>
      <div class="row"><div class="grow"><div class="title">安装位置</div><div class="sub path">${esc(info.install_dir || ov.install_dir)}</div></div>
        <button class="btn ghost icon" title="打开" data-action="open-path" data-path="${esc(info.install_dir || ov.install_dir)}">${icon('folder')}</button></div>
      <div class="row"><div class="grow"><div class="title">配置文件</div><div class="sub path">${esc(ov.config_path)}</div></div>
        <button class="btn ghost icon" title="在资源管理器中显示" data-action="open-path" data-path="${esc(ov.config_path)}">${icon('folder')}</button></div>
      ${info.notices ? `<div class="row"><div class="grow"><div class="title">第三方许可证</div><div class="sub">IsoBox 使用的开源组件（7-Zip、Rust 库等）及其许可证</div></div>
        <button class="btn ghost icon" title="在资源管理器中显示" data-action="open-path" data-path="${esc(info.notices)}">${icon('folder')}</button></div>` : ''}
    </div>
    <h2>彻底卸载</h2>
    ${info.uninstaller
    ? `<div class="danger-zone"><div class="grow" style="flex:1"><div class="title" style="color:var(--danger);font-weight:500">卸载 IsoBox</div>
      <div class="muted small">撤销所有禁止规则，删除隔离账户、所有库里的软件和 IsoBox 本身。可以选择把聊天记录等文件保留到交换文件夹</div></div>
      <button class="btn danger" data-action="uninstall">卸载</button></div>`
    : `<div class="danger-zone"><div class="grow" style="flex:1"><div class="title" style="color:var(--danger);font-weight:500">还原权限并删除隔离账户</div>
      <div class="muted small">撤销所有禁止规则，删除隔离账户、所有库里的软件和聊天记录等数据。交换文件夹里的文件保留</div></div>
      <button class="btn danger" data-action="uninstall" ${ov.account.exists ? '' : 'disabled'}>卸载</button></div>`}`;
}

// ---------- 事件 ----------
const actions = {
  nav: (el) => { S.page = el.dataset.page; S.menu = null; render(); },
  refresh: () => refresh(),
  'close-dialog': () => closeDialog(),
  install: () => runAdmin('安装隔离账户', ['install', '--apply']),
  repair: () => runAdmin('修复隔离账户', ['install', '--apply']),
  'add-start': () => startAdd(),
  'add-pick': () => addPick(),
  'add-run': () => addRun(),
  launch: async (el) => {
    try { toast(await invoke('launch_app', { name: el.dataset.name })); } catch (e) { toast('启动失败：' + e, 'error'); }
    setTimeout(refresh, 1500);
  },
  menu: (el) => { S.menu = S.menu === el.dataset.name ? null : el.dataset.name; render(); },
  'toggle-startup': async (el) => {
    S.menu = null;
    try { await invoke('set_app_startup', { name: el.dataset.name, enable: el.dataset.on === '1' }); } catch (e) { toast(String(e), 'error'); }
    refresh();
  },
  move: (el) => { S.menu = null; runAdmin(`移动到 ${el.dataset.drive} 盘`, ['move-app', el.dataset.name, '--to', el.dataset.drive]); },
  'remove-app': (el) => {
    S.menu = null;
    render();
    confirmDialog(`移除 ${el.dataset.display}？`, '会删除它的程序文件、快捷方式和配置。它在隔离账户里的数据（比如聊天记录）会保留，卸载整个 IsoBox 时才删除。', '移除',
      () => runAdmin(`移除 ${el.dataset.display}`, ['remove-app', el.dataset.name]));
  },
  'open-path': (el) => { if (el.dataset.path) invoke('open_path', { path: el.dataset.path }).catch((e) => toast(String(e), 'error')); },
  'lib-add': (el) => runAdmin(`在 ${el.dataset.drive} 盘建库`, ['library', 'add', el.dataset.drive]),
  'lib-remove': (el) => runAdmin(`移除 ${el.dataset.drive} 盘的库`, ['library', 'remove', el.dataset.drive]),
  'protect-remove': (el) => confirmDialog(`撤销对 ${el.dataset.path} 的保护？`, '撤销后隔离软件就能读写这个位置。', '撤销',
    () => runAdmin(`撤销保护 ${el.dataset.path}`, ['protect', 'remove', el.dataset.path])),
  'protect-drive': () => {
    const sel = document.querySelector('[data-bind-protect]');
    if (sel && sel.value) runAdmin(`保护 ${sel.value}`, ['protect', 'add', sel.value]);
  },
  'protect-folder': async () => {
    const dir = await pickFile({ directory: true, multiple: false });
    if (dir) runAdmin(`保护 ${dir}`, ['protect', 'add', dir]);
  },
  restore: (el) => confirmDialog(`还原 ${el.dataset.target} 的权限？`, '会把这个位置的权限恢复到备份时的状态，备份之后对权限的修改都会丢失。', '还原',
    () => runAdmin(`还原 ${el.dataset.target} 的权限`, ['restore-acl', el.dataset.dir])),
  'exchange-change': async () => {
    const dir = await pickFile({ directory: true, multiple: false });
    if (dir) runAdmin('更换交换文件夹', ['set-exchange', dir]);
  },
  'tray-autostart': async (el) => {
    try { await invoke('set_tray_autostart', { enable: el.checked }); toast(el.checked ? '已设置开机启动托盘' : '已取消开机启动托盘'); }
    catch (e) { toast(String(e), 'error'); }
    refresh();
  },
  uninstall: () => {
    // 用安装程序装的：交给系统的卸载程序，它会顺带删除 IsoBox 本身，并询问是否保留聊天记录
    if (S.info?.uninstaller) {
      confirmDialog('卸载 IsoBox？', '接下来打开卸载程序。它会撤销所有禁止规则，删除隔离账户、所有库里的软件和 IsoBox 本身，并让你选择是否把聊天记录等文件保留到交换文件夹。IsoBox 界面会随之关闭。', '打开卸载程序',
        async () => {
          try { if (!(await invoke('run_uninstaller'))) toast('已取消'); } catch (e) { toast(String(e), 'error'); }
        }, true);
      return;
    }
    confirmDialog('彻底卸载？', '会撤销所有禁止规则，删除隔离账户、所有库里的软件，以及微信、QQ 等软件的聊天记录和登录信息。这一步无法撤销。交换文件夹里的文件会保留。', '卸载',
      () => runAdmin('彻底卸载', ['uninstall', '--apply']), true);
  },
};

let pendingConfirm = null;
function confirmDialog(title, body, okText, onOk, danger = false) {
  pendingConfirm = onOk;
  showDialog(`<h3>${esc(title)}</h3><div class="muted">${esc(body)}</div>
    <div class="footer-actions"><button class="btn" data-action="close-dialog">取消</button>
    <button class="btn ${danger ? 'danger' : 'primary'}" data-action="confirm-ok">${esc(okText)}</button></div>`);
}
actions['confirm-ok'] = () => { const f = pendingConfirm; pendingConfirm = null; closeDialog(); if (f) f(); };

document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-action]');
  if (!el) {
    if (S.menu && !e.target.closest('[data-keep-menu]')) { S.menu = null; render(); }
    return;
  }
  if (el.tagName === 'INPUT' && el.type === 'checkbox' && el.dataset.action) {
    actions[el.dataset.action]?.(el);
    return;
  }
  e.preventDefault();
  if (el.disabled) return;
  actions[el.dataset.action]?.(el);
});

// 向导里的输入框：只更新状态。单选和下拉框会影响界面，需要重新渲染
document.addEventListener('input', (e) => {
  const key = e.target.dataset.bind;
  if (!key || !S.add) return;
  S.add[key] = e.target.type === 'checkbox' ? e.target.checked : e.target.value;
  if (e.target.type === 'radio' || e.target.tagName === 'SELECT') render();
});
document.addEventListener('change', (e) => {
  if (e.target.matches('[data-bind-protect]')) S.protectDrive = e.target.value;
});

// 软件页每 5 秒刷新一次运行状态；正在操作时不刷新，避免打断
setInterval(() => {
  if (S.page === 'apps' && !S.menu && !document.getElementById('overlay').innerHTML && document.visibilityState === 'visible') refresh(false);
}, 5000);

(async () => {
  try { S.info = await invoke('app_info'); } catch { /* 忽略 */ }
  await refresh();
})();
