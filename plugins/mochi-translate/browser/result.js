const $ = id => document.getElementById(id);
let port;
let requestId;
let busy = false;
function state(active) {
  busy = active;
  $('translate').disabled = active;
  $('target').disabled = active;
  $('model').disabled = active;
  $('source').readOnly = active;
  $('cancel').hidden = !active;
}
function disconnect() {
  if (port) { const old = port; port = undefined; old.disconnect(); }
}
function error(message, setup = false) {
  $('error').textContent = message;
  $('error').hidden = false;
  $('setup').hidden = !setup;
  $('status').textContent = '未完成';
  state(false);
}
function start() {
  if (busy) return;
  const text = $('source').value.trim();
  if (!text || text.length > 12000) { error('请选择 1–12000 个字符。'); return; }
  $('error').hidden = true;
  $('setup').hidden = true;
  $('result').textContent = '';
  delete $('result').dataset.model;
  $('copy').disabled = true;
  $('status').textContent = '正在连接…';
  state(true);
  requestId = crypto.randomUUID();
  const connected = chrome.runtime.connectNative('org.mochi.translate');
  port = connected;
  connected.onMessage.addListener(message => {
    if (port !== connected || message.id !== requestId) return;
    if (message.type === 'status') {
      $('status').textContent = message.status === 'translating' ? '正在翻译…' : '正在连接…';
    } else if (message.type === 'result') {
      $('result').textContent = message.translated_text;
      $('result').dataset.model = message.model;
      const modelName = message.model === 'gpt-5.6-luna' ? 'Luna' : message.model === 'gpt-6-astra' ? 'Astra' : message.model;
      $('status').textContent = message.detected_language + ' → ' + $('target').selectedOptions[0].textContent + ' · ' + modelName;
      $('copy').disabled = false;
      state(false);
      disconnect();
    } else if (message.type === 'error') {
      error(message.error);
      disconnect();
    }
  });
  connected.onDisconnect.addListener(() => {
    const message = chrome.runtime.lastError?.message;
    if (port !== connected) return;
    port = undefined;
    if (busy) error(message || '本地翻译连接已断开，请重试。', true);
  });
  connected.postMessage({ type: 'translate', id: requestId, text, target: $('target').value, model: $('model').value });
  void chrome.storage.local.set({ targetLanguage: $('target').value, translationModel: $('model').value });
}
$('source').addEventListener('input', () => { $('count').textContent = `${$('source').value.length} / 12000`; });
$('translate').addEventListener('click', start);
$('cancel').addEventListener('click', () => {
  port?.postMessage({ type: 'cancel' });
  disconnect();
  state(false);
  $('status').textContent = '已取消';
});
$('copy').addEventListener('click', async () => {
  try {
    await navigator.clipboard.writeText($('result').textContent);
    $('copy').textContent = '已复制';
    setTimeout(() => { $('copy').textContent = '复制译文'; }, 1500);
  } catch { error('复制失败，请直接选中译文复制。'); }
});
window.addEventListener('pagehide', disconnect);
const { targetLanguage, translationModel } = await chrome.storage.local.get(['targetLanguage', 'translationModel']);
if ([...$('target').options].some(option => option.value === targetLanguage)) $('target').value = targetLanguage;
if ([...$('model').options].some(option => option.value === translationModel)) $('model').value = translationModel;
const id = new URLSearchParams(location.search).get('id');
if (id && /^[a-f0-9-]{36}$/.test(id)) {
  const key = 'request:' + id;
  const saved = (await chrome.storage.session.get(key))[key];
  await chrome.storage.session.remove(key);
  if (saved && Date.now() - saved.created < 60000 && typeof saved.text === 'string') {
    $('source').value = saved.text;
    $('count').textContent = `${saved.text.length} / 12000`;
    if (saved.text.trim()) start();
  }
}
