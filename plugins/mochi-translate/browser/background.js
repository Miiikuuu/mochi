const MENU = 'mochi-translate-selection';
async function installMenu() {
  await chrome.contextMenus.removeAll();
  chrome.contextMenus.create({ id: MENU, title: '用 Mochi 翻译', contexts: ['selection'] });
}
chrome.runtime.onInstalled.addListener(installMenu);
chrome.runtime.onStartup.addListener(installMenu);

export function openTranslation(text = '') {
  const id = crypto.randomUUID();
  const port = chrome.runtime.connectNative('org.mochi.translate');
  let completed = false;
  function fail(error) {
    if (completed) return;
    completed = true;
    port.disconnect();
    void chrome.storage.session.set({ desktopError: error }).then(() =>
      chrome.windows.create({ url: chrome.runtime.getURL('desktop-error.html'), type: 'popup', width: 420, height: 230 }));
  }
  port.onMessage.addListener(message => {
    if (message.id !== id) return;
    if (message.type === 'accepted') {
      completed = true;
      port.disconnect();
    } else if (message.type === 'error') fail(message.error);
  });
  port.onDisconnect.addListener(() => {
    const error = chrome.runtime.lastError?.message;
    if (!completed) fail(error || '无法连接 Mochi 桌宠。');
  });
  port.postMessage({ type: 'show_translation', id, text });
}
chrome.contextMenus.onClicked.addListener(info => {
  if (info.menuItemId === MENU) openTranslation(info.selectionText || '');
});
chrome.action.onClicked.addListener(() => openTranslation());
