import '/src/frontend/src/styles.css';
import '/src/frontend/src/mobile/styles.css';
import '/src/frontend/src/terminal-input-actions.css';
import { createIcons, icons } from 'lucide';
import { renderShell } from '/src/frontend/src/shell.ts';
import { translate } from '/src/frontend/src/i18n.ts';
import { createTerminalInputActionController } from '/src/frontend/src/terminal-input-actions.ts';
const elements = renderShell(document.querySelector('#app'));
const local = { id: 'local', name: 'WeChat', provider: 'plugin', pluginId: 'wechat-asr', format: 'wav', baseUrl: '', apiKey: '', model: '', language: 'auto', endpointType: 'audio-transcriptions' };
const qa = window.qa = { enabled: true, uploads: [], sent: [], streams: [], statuses: [], settings: { aiVoiceInputEnabled: true, aiVoiceProviderProfiles: [local], aiVoiceActiveProviderProfileId: 'local' } };
const getUserMedia = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
navigator.mediaDevices.getUserMedia = async constraints => { const stream = await getUserMedia(constraints); qa.streams.push(stream); return stream; };
window.fetch = async (_, options) => {
  const form = options.body;
  qa.uploads.push({ profileId: form.get('profileId'), provider: form.get('provider'), pluginId: form.get('pluginId'), audioSize: form.get('audio').size, mime: form.get('audio').type });
  return new Response(JSON.stringify({ text: 'echo hello' }), { headers: { 'content-type': 'application/json' } });
};
const pane = { id: 'test-pane' };
const controller = createTerminalInputActionController({
  root: elements.terminalInputActionsSurface, settings: () => qa.settings, voiceProviderAvailable: () => qa.enabled,
  activePane: () => pane, sendText: (_, text) => { qa.sent.push(text); return true; },
  uploadFilesToTemporaryDirectory: () => [], temporaryDirectoryFileUploadVisible: () => false,
  temporaryDirectoryFileUploadAvailable: () => false, imageUploadAvailable: () => false,
  focusTerminal: () => {}, tr: (key, values) => translate('en', key, values),
  onStatus: (text, tone) => qa.statuses.push({ text, tone }), updateIcons: () => createIcons({ icons }),
});
qa.disable = () => { qa.enabled = false; controller.render(); };
qa.enable = () => { qa.enabled = true; controller.render(); };
qa.mobile = enabled => { document.body.classList.toggle('mobile-controls-enabled', enabled); controller.render(); };
qa.switchProfile = () => { qa.settings.aiVoiceProviderProfiles = [{ id: 'cloud', provider: 'openai-compatible', apiKey: 'fixture', baseUrl: 'https://example.invalid', model: 'fixture', format: 'wav' }]; qa.settings.aiVoiceActiveProviderProfileId = 'cloud'; controller.render(); };
qa.resetProfile = () => { qa.settings.aiVoiceProviderProfiles = [local]; qa.settings.aiVoiceActiveProviderProfileId = 'local'; controller.render(); };
qa.stopped = () => qa.streams.every(stream => stream.getTracks().every(track => track.readyState === 'ended'));
controller.render(); qa.ready = true;
