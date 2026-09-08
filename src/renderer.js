const $ = id => document.getElementById(id);
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
for (const [button, field] of [['pickNode','node'],['pickCwd','cwd']]) $(button).onclick = async () => { const value = await action(button); if (value) $(field).value = value; };
$('save').onclick = async () => { $('message').textContent = ''; await action('save', {node:$('node').value,cwd:$('cwd').value,autoOpen:$('autoOpen').checked}); };
let polling = false;
setInterval(async () => { if (polling) return; polling = true; try { const result = await action('state'); if (result) render(result.state); } finally { polling = false; } }, 700);
action('state').then(result => { if (!result) return; $('node').value = result.config.node; $('cwd').value = result.config.cwd; $('autoOpen').checked = result.config.autoOpen; render(result.state); });
