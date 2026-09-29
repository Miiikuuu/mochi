const { desktopError } = await chrome.storage.session.get('desktopError');
if (typeof desktopError === 'string') document.getElementById('message').textContent = desktopError;
await chrome.storage.session.remove('desktopError');
