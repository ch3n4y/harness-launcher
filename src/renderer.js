const $ = id => document.getElementById(id);
let nodePreference = '';
let detectedNode = '';
const labels = {stopped:'已停止',starting:'启动中',running:'运行中',stopping:'停止中',installing:'安装中',error:'异常'};
function page(id) {
  document.querySelectorAll('.page').forEach(el => { el.hidden = el.id !== id; });
  $('menu').hidden = true;
  $('menuToggle').setAttribute('aria-expanded', 'false');
  document.querySelector(`#${id} button`)?.focus();
}
document.querySelectorAll('[data-page]').forEach(button => { button.onclick = () => page(button.dataset.page); });
$('menuToggle').onclick = () => { $('menu').hidden = !$('menu').hidden; $('menuToggle').setAttribute('aria-expanded', String(!$('menu').hidden)); };
document.addEventListener('click', event => { if (!event.target.closest('.menu-wrap')) { $('menu').hidden = true; $('menuToggle').setAttribute('aria-expanded', 'false'); } });
document.addEventListener('keydown', event => { if (event.key === 'Escape') { page('home'); $('menuToggle').focus(); } });
async function action(name, data) {
  try { return await window.__TAURI__.core.invoke('action', {name, data: data ?? null}); }
  catch (error) { $('message').textContent = String(error); return null; }
}
function render(s) {
  $('status').textContent = labels[s.status] || s.status;
  const rt = s.runtime;
  $('indicator').dataset.status = s.status;
  $('summary').textContent = rt ? `Node ${rt.version} · Harness ${rt.harnessVersion || '未安装'}` : '需要配置 Node 环境';
  const active = ['running','starting','stopping'].includes(s.status);
  $('start').hidden = active;
  $('stop').hidden = !active;
  $('runtime').textContent = rt ? `Node ${rt.version} · ${rt.node}\nHarness ${rt.harnessVersion || '未安装'}\n全局目录：${rt.root}` : '未找到可用运行环境，请查看日志或手动选择 Node。';
  detectedNode = rt?.node || '';
  if (!nodePreference && document.activeElement !== $('node')) $('node').value = detectedNode;
  $('selectedNode').textContent = rt ? `${nodePreference ? '指定环境' : '自动选择（优先 dsh）'} · Node ${rt.version}${rt.entry ? ' · 已安装 dsh' : ' · 未安装 dsh'}\n当前：${rt.node}` : '尚未找到兼容环境，可输入路径或选择文件';
  const candidateKey = JSON.stringify(s.candidates);
  if ($('nodeCandidates').dataset.paths !== candidateKey) {
    $('nodeCandidates').dataset.paths = candidateKey;
    $('nodeCandidates').replaceChildren(...s.candidates.map(path => new Option(path, path)));
  }
  $('autoNode').disabled = !!s.busy;
  $('logs').textContent = s.logs.join('\n');
  if ($('follow').checked) $('logs').scrollTop = $('logs').scrollHeight;
  for (const id of ['start','restart','install','detect','save','node','cwd','pickNode','pickCwd','autoOpen']) $(id).disabled = !!s.busy;
  $('start').disabled ||= ['running','starting','stopping'].includes(s.status) || !rt?.entry;
  $('stop').disabled = !['running','starting'].includes(s.status);
  $('restart').disabled ||= s.status !== 'running';
  $('install').disabled ||= ['running','starting','stopping'].includes(s.status) || !rt;
  $('install').textContent = rt?.harnessVersion ? '更新 Harness' : '安装 Harness';
}
for (const id of ['start','stop','restart','open','detect','install','clear','export']) $(id).onclick = async () => { $('message').textContent = ''; if (id === 'install') page('logPage'); await action(id); };
$('copy').onclick = async () => { try { await navigator.clipboard.writeText($('logs').textContent); $('message').textContent = '日志已复制'; } catch { $('message').textContent = '复制失败，请使用导出'; } };
for (const [button, field] of [['pickNode','node'],['pickCwd','cwd']]) $(button).onclick = async () => { const value = await action(button); if (value) { $(field).value = value; if (field === 'node') { nodePreference = value;  } } };
$('save').onclick = async () => { $('message').textContent = ''; await action('save', {node:nodePreference,cwd:$('cwd').value,autoOpen:$('autoOpen').checked}); };
let polling = false;
setInterval(async () => { if (polling) return; polling = true; try { const result = await action('state'); if (result) render(result.state); } finally { polling = false; } }, 700);
action('state').then(result => { if (!result) return; nodePreference = result.config.node; $('node').value = nodePreference; $('cwd').value = result.config.cwd; $('autoOpen').checked = result.config.autoOpen; render(result.state); });

$('autoNode').onclick = () => { nodePreference = ''; $('node').value = detectedNode; };
$('node').oninput = () => { nodePreference = $('node').value.trim(); };
let updateBusy = false;
async function updateLauncher(install) {
  if (updateBusy) return;
  updateBusy = true;
  $('checkUpdate').disabled = $('downloadUpdate').disabled = true;
  $('updateStatus').textContent = install ? '正在下载并校验，请稍候…' : '正在连接 GitHub…';
  try {
    const result = await window.__TAURI__.core.invoke('launcher_update', {install});
    if (install) {
      $('updateStatus').textContent = `安装包已校验并打开：${result.path}。请从托盘退出启动器后完成安装。`;
    } else {
      $('updateStatus').textContent = `当前 v${result.current} · 最新 v${result.latest}。` + (result.available ? (result.asset && result.checksum ? '有新版本可用。' : '此版本暂未提供本机安装包或校验文件。') : '已是最新版本。');
      $('downloadUpdate').hidden = !(result.available && result.asset && result.checksum);
      $('releaseNotes').textContent = result.notes || '暂无更新说明';
      $('releaseNotes').hidden = !result.available;
    }
  } catch (error) { $('updateStatus').textContent = `更新失败：${error}`; }
  finally { updateBusy = false; $('checkUpdate').disabled = $('downloadUpdate').disabled = false; }
}
$('checkUpdate').onclick = () => updateLauncher(false);
$('downloadUpdate').onclick = () => updateLauncher(true);

async function windowAction(name, height) {
  try { await window.__TAURI__.core.invoke('window_action', {name, height: height ?? null}); }
  catch (error) { $('message').textContent = String(error); }
}
$('hideWindow').onclick = () => windowAction('hide');
$('minimizeWindow').onclick = () => windowAction('minimize');
$('titlebar').onmousedown = event => {
  if (event.button === 0 && !event.target.closest('button, input, select, nav')) windowAction('drag');
};
if (typeof ResizeObserver !== 'undefined') {
  let lastHeight = 0;
  let resizeTimer;
  new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      const height = Math.ceil($('windowShell').getBoundingClientRect().height);
      if (height !== lastHeight) { lastHeight = height; windowAction('resize', height); }
    }, 80);
  }).observe($('windowShell'));
}
