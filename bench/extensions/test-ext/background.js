// Answers the popup: a ping (is the service worker alive?).
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message === 'ping') reply('pong');
});
