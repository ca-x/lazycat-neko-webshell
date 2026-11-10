import assert from "node:assert/strict";
import test from "node:test";
import { sanitizeAiVoiceProviderProfile, aiVoiceProfileConfigured, normalizeAiVoiceProviderProfiles } from "./voice-profiles.ts";
import { sanitizeAiVoiceSpeechProviderProfile } from "./voice-speech-profiles.ts";
import { asrProfileAvailable } from "../asr/providers.ts";

test("local ASR profiles persist without inheriting cloud credentials or a compressed format", () => {
  const profile = sanitizeAiVoiceProviderProfile({ id: "local", provider: "plugin", pluginId: "wechat-asr", apiKey: "old-secret", baseUrl: "https://api.invalid", model: "old", format: "mp3" }, 0);
  assert.equal(profile.apiKey, ""); assert.equal(profile.baseUrl, ""); assert.equal(profile.model, "");
  assert.equal(profile.format, "wav"); assert.equal(aiVoiceProfileConfigured(profile), true);
  assert.deepEqual(normalizeAiVoiceProviderProfiles(JSON.parse(JSON.stringify([profile]))).find(item => item.id === "local"), profile);
  assert.equal(aiVoiceProfileConfigured({ ...profile, pluginId: "" }), false);
});

test("plugin profiles cannot fall back to another provider when unavailable", () => {
  const profile = sanitizeAiVoiceProviderProfile({ provider: "plugin", pluginId: "wechat-asr" }, 0);
  assert.equal(asrProfileAvailable(profile, []), false);
  assert.equal(asrProfileAvailable(profile, [{ id: "wechat-asr", metadata: { asrProvider: "true" }, enabled: false }]), false);
  assert.equal(asrProfileAvailable(profile, [{ id: "wechat-asr", metadata: {}, enabled: true }]), false);
  assert.equal(asrProfileAvailable(profile, [{ id: "wechat-asr", metadata: { asrProvider: "true" }, enabled: true }]), true);
});

test("API input and speech output profiles retain their independent normalization", () => {
  const profile = sanitizeAiVoiceProviderProfile({ id: "api", provider: "openai-compatible", baseUrl: "https://api.example/v1", apiKey: "key", model: "asr-model", format: "mp3" }, 0);
  assert.equal(profile.apiKey, "key"); assert.equal(profile.format, "mp3"); assert.equal(profile.model, "asr-model");
  assert.equal(aiVoiceProfileConfigured(profile), true); assert.equal(asrProfileAvailable(profile, []), true);
  assert.notEqual(sanitizeAiVoiceSpeechProviderProfile({ provider: "plugin", pluginId: "wechat-asr" }, 0).provider, "plugin");
});
