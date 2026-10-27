import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { createServer } from 'vite';
import { createBrowserDriver, uniqueBrowserSession } from '../browser-driver.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const dock = '#mobileShortcuts';
const main = `${dock} [data-mobile-panel="main"]`;
const chord = (value) => `${main} [data-mobile-chord="${value}"]`;
const shortcut = (value) => `${main} [data-mobile-shortcut="${value}"]`;
const quick = (value) => `${dock} .mobile-keyboard-quick-keys [data-mobile-shortcut="${value}"]`;
const page = (value) => `${dock} .mobile-keyboard-page-tabs [data-mobile-page="${value}"]`;
const touch = (type, point, delayMs = 0) => ({
  type, delayMs,
  touchPoints: type === 'touchEnd' || type === 'touchCancel' ? [] : [{ ...point, id: 1, radiusX: 2, radiusY: 2, force: 1 }],
});

export async function runMobileAgentKeyboardScenario() {
  const artifactDirectory = join(root, 'tests-auto/artifacts/mobile-agent-keyboard');
  await mkdir(artifactDirectory, { recursive: true });
  const browser = createBrowserDriver({ session: uniqueBrowserSession('mobile-agent-keyboard'), headed: false });
  const server = await createServer({ configFile: false, root, server: { host: '127.0.0.1', port: 0 }, logLevel: 'error' });
  const results = [];
  const measurements = [];
  const cleanupErrors = [];
  let failure;
  const evaluate = (source) => browser.evaluate(source);
  const state = () => evaluate('qa.state()');
  const clear = () => evaluate('qa.clearOutput()');
  const settle = () => evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
  const tap = async (selector) => {
    const point = await evaluate(`qa.point(${JSON.stringify(selector)})`);
    await browser.dispatchTouch([touch('touchStart', point, 25), touch('touchEnd', point)]);
  };
  const expand = async () => {
    if (!(await state()).expanded) await tap(`${dock} [data-mobile-action="toggle-expanded-keyboard"]`);
  };
  const toggle = async (enabled) => {
    if ((await state()).enabled === enabled) return;
    await evaluate('qa.openSettings()');
    await browser.command([enabled ? 'check' : 'uncheck', '[data-mobile-agent-keyboard-enabled]']);
    assert.equal((await state()).enabled, enabled, 'the real settings controller applies the opt-in');
    await browser.command(['click', '#agentKeyboardFixtureCloseSettings']);
    await settle();
  };
  const profile = async (agent) => {
    await toggle(true);
    await evaluate(`qa.useNative("native-a", ${JSON.stringify(agent)})`);
    await expand();
    await tap(page('main'));
    await clear();
  };
  const exact = async (selector, expected, label) => {
    await clear();
    await evaluate(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'nearest',inline:'nearest'})`);
    await tap(selector);
    const current = await state();
    assert.deepEqual(current.output.map((event) => event.data), [expected], `${label}: one activation sends exactly the representative bytes`);
    assert.equal(current.output[0].paneId, current.paneId, `${label}: input uses the current terminal`);
    if (current.output[0].backend === 'herdr') {
      assert.equal(current.output[0].remotePaneId, current.remotePaneId, `${label}: input uses the focused Herdr pane`);
    }
  };
  const screenshot = (name) => browser.screenshot(join(artifactDirectory, `${name}.png`));
  const measure = async (label) => {
    const data = await evaluate(`(()=>{
      const root=document.querySelector('#mobileShortcuts'),panel=document.querySelector(${JSON.stringify(main)});
      const bounds=node=>{const r=node.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height}};
      return {viewport:{width:innerWidth,height:innerHeight},dock:bounds(root),panel:bounds(panel),contentHeight:panel.scrollHeight,scrollTop:panel.scrollTop,
        groups:[...panel.querySelectorAll('section')].map(node=>({label:node.getAttribute('aria-label'),...bounds(node)})),
        keys:[...panel.querySelectorAll('button')].map(button=>{
          const hint=button.querySelector('small'),style=getComputedStyle(hint),label=button.querySelector('span');
          return {label:label?.textContent,...bounds(button),contentHeight:button.scrollHeight,clientHeight:button.clientHeight,scrollWidth:button.scrollWidth,clientWidth:button.clientWidth,
            labelBox:{...bounds(label),scrollWidth:label.scrollWidth,clientWidth:label.clientWidth},
            hint:{text:hint.textContent,...bounds(hint),whiteSpace:style.whiteSpace,textOverflow:style.textOverflow,overflow:style.overflow,scrollWidth:hint.scrollWidth,clientWidth:hint.clientWidth,scrollHeight:hint.scrollHeight,clientHeight:hint.clientHeight}};
        })};
    })()`);
    measurements.push({ label, ...data });
    return data;
  };
  const assertWrapping = (data, label) => {
    for (const key of data.keys) {
      assert.ok(key.labelBox.scrollWidth <= key.labelBox.clientWidth + 1, `${label} ${key.label}: action label fits horizontally (${key.labelBox.scrollWidth}px content / ${key.labelBox.clientWidth}px box)`);
      assert.ok(key.scrollWidth <= key.clientWidth + 1, `${label} ${key.label}: all key text stays inside its target`);
      assert.equal(key.hint.whiteSpace, 'normal', `${label} ${key.label}: hints wrap instead of clipping`);
      assert.equal(key.hint.overflow, 'visible', `${label} ${key.label}: the hint is not clipped by its own box`);
      assert.ok(key.hint.scrollWidth <= key.hint.clientWidth + 1, `${label} ${key.label}: hint fits horizontally`);
      assert.ok(key.hint.scrollHeight <= key.hint.clientHeight + 1, `${label} ${key.label}: hint fits vertically`);
      assert.ok(key.contentHeight <= key.clientHeight + 1, `${label} ${key.label}: automatic key height contains its content`);
      assert.ok(key.hint.y + key.hint.height <= key.y + key.height + 1, `${label} ${key.label}: complete hint fits inside the key`);
    }
  };
  const check = async (name, run) => {
    try {
      await run();
      results.push({ name, status: 'passed' });
      console.log(`PASS mobile-agent-keyboard: ${name}`);
    } catch (error) {
      const diagnostic = await evaluate(`({state:qa.state(),focus:document.activeElement?.outerHTML.slice(0,220),dock:document.querySelector(${JSON.stringify(dock)}).getBoundingClientRect().toJSON(),viewport:[innerWidth,innerHeight]})`).catch(() => null);
      results.push({ name, status: 'failed', error: error.message, diagnostic });
      await screenshot(`failure-${name}`).catch(() => {});
      console.error(`FAIL mobile-agent-keyboard: ${name}: ${error.message}`);
      await evaluate('qa.closeSettings()').catch(() => {});
    }
  };
  try {
    await server.listen();
    await browser.open(`http://127.0.0.1:${server.httpServer.address().port}/tests-auto/mobile-agent-keyboard/fixture.html`);
    await browser.command(['set', 'viewport', '390', '844']);
    await browser.waitFor('window.qa?.ready');

    await check('default-off-and-settings-toggle', async () => {
      const before = await state();
      assert.equal(before.enabled, false, 'the production default is off');
      assert.equal(before.context, null);
      assert.equal(await evaluate(`document.querySelectorAll("#mobileShortcuts .mobile-agent-keyboard, #mobileShortcuts .mobile-agent-keyboard-heading, #mobileShortcuts .mobile-agent-keyboard-group").length`), 0);
      await expand();
      const savedLayout = JSON.stringify(before.layout);
      const base = await evaluate(`document.querySelector(${JSON.stringify(main)}).innerHTML`);
      await toggle(true);
      assert.equal((await state()).mainLabel, 'Codex CLI');
      assert.equal(JSON.stringify((await state()).layout), savedLayout, 'temporary Agent profile does not mutate the saved layout');
      assert.equal((await state()).saves, before.saves + 1, 'opt-in saves through the existing settings controller');
      await toggle(false);
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(main)}).innerHTML`), base, 'turning off restores the base keys exactly');
      assert.deepEqual((await state()).output, [], 'settings changes send no terminal bytes');
      await screenshot('default-off');
    });

    await check('four-semantic-profiles-no-gating-ui', async () => {
      for (const [agent, name] of [['codex', 'Codex CLI'], ['claude', 'Claude Code'], ['opencode', 'OpenCode'], ['pi', 'Pi']]) {
        await profile(agent);
        const current = await state();
        assert.equal(current.mainLabel, name);
        assert.deepEqual(current.groupLabels, ['发送与排队', '问题与控制', 'Agent 与模式', '编辑与查看']);
        assert.ok(current.labels.includes('发送') && current.labels.includes('换行') && current.labels.includes('中断／取消'), `${agent}: actions carry semantic labels`);
        assert.doesNotMatch(current.mainText, /版本|version|≥|\b2\.1\.\d+|\bv2\b/i, `${agent}: keyboard has no version copy`);
        assert.equal(await evaluate(`document.querySelectorAll('#mobileShortcuts input[type="checkbox"], #mobileShortcuts [role="checkbox"], #mobileShortcuts [data-mobile-agent-new-keys]').length`), 0, `${agent}: no extra checkbox or opt-in is rendered`);
        assert.equal(await evaluate(`document.querySelectorAll(${JSON.stringify(`${main} button:disabled`)}).length`), 0, `${agent}: every profile action is available immediately`);
        const accessible = await evaluate(`[...document.querySelectorAll(${JSON.stringify(`${main} button`)})].every(button => button.getAttribute('aria-label') && button.querySelector('small')?.textContent)`);
        assert.equal(accessible, true, `${agent}: each semantic action exposes its physical chord`);
        await screenshot(`profile-${agent}`);
      }
      await profile('codex');
      const text = (await state()).mainText;
      assert.match(text, /异步提问/);
      assert.match(text, /不批准权限/, 'skipping a question is not presented as permission approval');
      assert.match(text, /运行中/, 'busy Agent actions explain their queue semantics');
    });

    await check('agent-category-rows-swipe-and-preserve-position', async () => {
      await browser.command(['set', 'viewport', '320', '568']);
      await profile('claude');
      const row = `${main} [data-agent-group="edit"] .mobile-agent-keyboard-keys`;
      const before = await evaluate(`(()=>{const el=document.querySelector(${JSON.stringify(row)});return {width:el.clientWidth,content:el.scrollWidth}})()`);
      assert.ok(before.content > before.width, 'the category contains horizontally reachable shortcuts');
      await evaluate(`document.querySelector(${JSON.stringify(row)}).scrollIntoView({block:'nearest'})`);
      const point = await evaluate(`qa.point(${JSON.stringify(`${row} button:first-child`)})`);
      await browser.dispatchTouch([
        touch('touchStart', point, 25),
        touch('touchMove', { x: point.x - 30, y: point.y }, 25),
        touch('touchMove', { x: point.x - 90, y: point.y }, 25),
        touch('touchEnd', point),
      ]);
      const scrolled = await evaluate(`document.querySelector(${JSON.stringify(row)}).scrollLeft`);
      assert.ok(scrolled > 0, 'horizontal touch gesture moves the category');
      assert.deepEqual((await state()).output, [], 'swiping a key sends no terminal input');
      await evaluate('qa.setLocale("en")');
      assert.ok(await evaluate(`document.querySelector(${JSON.stringify(row)}).scrollLeft`) > 0, 'language rerender retains the category position');
      await evaluate(`document.querySelector(${JSON.stringify(chord('alt-y'))}).scrollIntoView({block:'nearest',inline:'nearest'})`);
      await exact(chord('alt-y'), '\x1by', 'last shortcut in horizontally scrolled category');
      await browser.command(['set', 'viewport', '390', '844']);
    });

    await check('codex-async-navigation-exact-bytes', async () => {
      await profile('codex');
      for (const [selector, expected, label] of [
        [shortcut('enter'), '\r', 'send'], [chord('ctrl-j'), '\n', 'new line'], [shortcut('tab'), '\t', 'queue'],
        [chord('alt-up'), '\x1b[1;3A', 'retrieve/edit async question'], [chord('alt-down'), '\x1b[1;3B', 'return toward input'],
        [chord('shift-left'), '\x1b[1;2D', 'retrieve/edit async question alternative'],
        [chord('shift-right'), '\x1b[1;2C', 'return toward input alternative'],
        [chord('shift-down'), '\x1b[1;2B', 'lower reasoning effort'],
        [chord('shift-up'), '\x1b[1;2A', 'raise reasoning effort'],
        [chord('ctrl-bracket-right'), '\x1d', 'skip focused question'], [shortcut('escape'), '\x1b', 'interrupt/cancel'],
        [chord('alt-left'), '\x1b[1;3D', 'previous Agent'], [chord('alt-right'), '\x1b[1;3C', 'next Agent'],
        [chord('shift-tab'), '\x1b[Z', 'collaboration'], [chord('ctrl-t'), '\x14', 'transcript'],
        [chord('ctrl-slash'), '\x1f', 'side conversation'],
      ]) await exact(selector, expected, `Codex ${label}`);
    });

    await check('agent-usage-promotes-within-category-on-return', async () => {
      await profile('codex');
      const row = `${main} [data-agent-group="send"] .mobile-agent-keyboard-keys`;
      for (let index = 0; index < 3; index++) await exact(chord('shift-left'), '\x1b[1;2D', 'Codex used shortcut');
      await tap(page('nav'));
      await tap(page('main'));
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(row)})?.firstElementChild?.dataset.mobileChord`), 'shift-left', 'frequently used key moves to the front');
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(`${main} [data-agent-group="control"] .mobile-agent-keyboard-keys`)})?.firstElementChild?.dataset.mobileShortcut`), 'escape', 'other categories retain their order');
      await profile('claude');
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(`${main} [data-agent-group="send"] .mobile-agent-keyboard-keys`)})?.firstElementChild?.dataset.mobileShortcut`), 'enter', 'usage is isolated by Agent');
    });

    await check('server-usage-loads-and-replaces-another-users-order', async () => {
      const row = `${main} [data-agent-group="send"] .mobile-agent-keyboard-keys`;
      await evaluate(`window.__agentKeyboardFetch = window.fetch;
        window.__agentKeyboardUser = 'alice';
        window.fetch = (input, init) => String(input).includes('/api/agent-keyboard/usage') && !init?.method
          ? Promise.resolve(new Response(JSON.stringify({userId:window.__agentKeyboardUser,usage:[{
              agent:'codex',group:'send',shortcut:window.__agentKeyboardUser === 'alice' ? 'alt-enter' : 'enter',count:100
            }]}),{headers:{'content-type':'application/json'}}))
          : window.__agentKeyboardFetch(input,init);`);
      try {
        await profile('claude');
        await profile('codex');
        await evaluate('new Promise(resolve => setTimeout(resolve, 500))');
        assert.equal(await evaluate(`document.querySelector(${JSON.stringify(row)})?.firstElementChild?.dataset.mobileChord`), 'alt-enter');
        await evaluate(`window.__agentKeyboardUser = 'bob'`);
        await profile('claude');
        await profile('codex');
        await evaluate('new Promise(resolve => setTimeout(resolve, 500))');
        assert.equal(await evaluate(`document.querySelector(${JSON.stringify(row)})?.firstElementChild?.dataset.mobileShortcut`), 'enter');
      } finally {
        await evaluate('window.fetch = window.__agentKeyboardFetch');
      }
    });

    await check('claude-opencode-pi-exact-chords-without-extra-opt-in', async () => {
      await profile('claude');
      await exact(chord('ctrl-x-ctrl-s'), '\x18\x13', 'Claude send queued now');
      await exact(chord('ctrl-x-enter'), '\x18\r', 'Claude queue without interrupting');
      await exact(chord('ctrl-enter'), '\x1b[13;5u', 'Claude enhanced send queued now');
      await exact(chord('shift-left'), '\x1b[1;2D', 'Claude selection or side answer');
      await exact(chord('alt-p'), '\x1bp', 'Claude choose model');
      await exact(chord('ctrl-b'), '\x02', 'Claude background task');
      await exact(chord('shift-tab'), '\x1b[Z', 'Claude permission mode');
      await exact(shortcut('up'), '\x1b[A', 'Claude retrieve queued');
      await profile('opencode');
      await exact(chord('ctrl-x-enter'), '\x18\r', 'OpenCode queue');
      await exact(chord('ctrl-x-a'), '\x18a', 'OpenCode Agent list');
      await exact(chord('ctrl-x-m'), '\x18m', 'OpenCode model list');
      await exact(chord('shift-tab'), '\x1b[Z', 'OpenCode v2 Agent cycle');
      await exact(chord('ctrl-x-t'), '\x18t', 'OpenCode v2 terminal toggle');
      await exact(chord('ctrl-x-i'), '\x18i', 'OpenCode v2 prompt images');
      await exact(chord('ctrl-x-1'), '\x181', 'OpenCode v2 first tab');
      await exact(chord('ctrl-shift-tab'), '\x1b[9;6u', 'OpenCode v2 previous tab');
      await exact(chord('shift-right'), '\x1b[1;2C', 'OpenCode v2 extend selection');
      await exact(chord('ctrl-x-n'), '\x18n', 'OpenCode new session');
      await exact(shortcut('down'), '\x1b[B', 'OpenCode next input');
      await profile('pi');
      await exact(chord('alt-enter'), '\x1b\r', 'Pi follow up');
      await exact(chord('alt-up'), '\x1b[1;3A', 'Pi retrieve queued');
      await exact(chord('ctrl-l'), '\x0c', 'Pi choose model');
      await exact(chord('ctrl-p'), '\x10', 'Pi next model');
      await exact(chord('ctrl-shift-p'), '\x1b[112;6u', 'Pi previous model');
      await exact(chord('ctrl-o'), '\x0f', 'Pi tool output');
    });

    await check('semantic-actions-ignore-and-clear-sticky-modifiers', async () => {
      await profile('codex');
      for (const modifier of ['ctrl', 'alt', 'shift']) {
        await tap(quick(modifier));
        assert.equal(await evaluate(`document.querySelector(${JSON.stringify(quick(modifier))}).getAttribute('aria-pressed')`), 'true');
        await exact(shortcut('enter'), '\r', `${modifier} does not alter semantic send`);
        assert.equal(await evaluate(`document.querySelector(${JSON.stringify(quick(modifier))}).getAttribute('aria-pressed')`), 'false');
      }
      await tap(quick('alt'));
      await exact(chord('ctrl-bracket-right'), '\x1d', 'sticky Alt does not alter Ctrl+]');
      await tap(quick('ctrl'));
      await exact(shortcut('escape'), '\x1b', 'sticky Ctrl does not alter semantic cancel');
    });

    await check('saved-presets-and-custom-layout-survive-off-and-unknown', async () => {
      await toggle(false);
      for (const preset of ['default', 'vim', 'emacs', 'custom']) {
        await evaluate(`qa.setBase(${JSON.stringify(preset)}, ${preset === 'custom'});qa.useNative('native-a','codex')`);
        const before = await state();
        const mainHTML = await evaluate(`document.querySelector(${JSON.stringify(main)}).innerHTML`);
        const navHTML = await evaluate(`document.querySelector('#mobileShortcuts [data-mobile-panel="nav"]').innerHTML`);
        await toggle(true);
        assert.equal((await state()).mainLabel, 'Codex CLI');
        assert.equal(await evaluate(`document.querySelector('#mobileShortcuts [data-mobile-panel="nav"]').innerHTML`), navHTML, 'Agent profile changes only the main page');
        await evaluate('qa.useNative("native-a", "unknown-agent")');
        assert.equal((await state()).context, null);
        assert.equal(await evaluate(`document.querySelector(${JSON.stringify(main)}).innerHTML`), mainHTML, `${preset}: unknown Agent restores the saved preset`);
        await evaluate('qa.useNative("native-a", "codex")');
        await toggle(false);
        assert.equal(await evaluate(`document.querySelector(${JSON.stringify(main)}).innerHTML`), mainHTML, `${preset}: off restores the saved preset`);
        assert.deepEqual((await state()).layout, before.layout, `${preset}: saved keys are intact`);
        assert.equal((await state()).preset, preset);
        if (preset === 'custom') {
          await expand(); await tap(page('main'));
          await exact(`${main} [data-mobile-text]`, 'fixture-custom\x1b!', 'restored custom key');
        }
      }
      await evaluate('qa.setBase("default");qa.clearOutput()');
    });

    await check('native-context-cache-tracks-current-pane-and-process', async () => {
      await profile('codex');
      const original = (await state()).context.key;
      await tap(quick('ctrl'));
      await evaluate('qa.useNative("native-b", "claude")');
      assert.equal((await state()).mainLabel, 'Claude Code');
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(quick('ctrl'))}).getAttribute('aria-pressed')`), 'false', 'pane switch clears transient modifier state');
      await evaluate('qa.select("native-a")');
      assert.equal((await state()).mainLabel, 'Codex CLI');
      assert.equal((await state()).context.key, original, 'returning to a terminal uses its current cached status');
      await tap(quick('alt'));
      await evaluate('qa.useNative("native-a", "codex", "replacement-process")');
      assert.notEqual((await state()).context.key, original, 'a replacement process changes the context identity');
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(quick('alt'))}).getAttribute('aria-pressed')`), 'false');
      for (const invalid of [
        { programStatus: [] },
        { programStatus: [{ id: 'one', app: 'codex', state: 'working' }, { id: 'two', app: 'claude', state: 'working' }] },
        { connectionState: 'reconnecting' }, { replaying: true }, { closing: true }, { exited: true },
      ]) {
        await evaluate('qa.patchPane({connectionState:"connected",replaying:false,closing:false,exited:false});qa.useNative("native-a","codex")');
        await evaluate(`qa.patchPane(${JSON.stringify(invalid)})`);
        assert.equal((await state()).context, null, `unsafe or ambiguous native status falls back: ${JSON.stringify(invalid)}`);
      }
      await evaluate('qa.patchPane({connectionState:"connected",replaying:false,closing:false,exited:false});qa.useNative("native-a","codex")');
      assert.deepEqual((await state()).output, [], 'context changes do not send bytes');
    });

    await check('herdr-focus-cache-never-borrows-another-pane', async () => {
      await profile('codex');
      await evaluate('qa.useHerdr("remote-a")');
      assert.equal((await state()).context.agent, 'codex');
      await exact(chord('ctrl-bracket-right'), '\x1d', 'Herdr Codex current focused pane');
      await clear();
      await tap(quick('ctrl'));
      await evaluate('qa.focusHerdr("remote-b")');
      assert.equal((await state()).context.agent, 'claude');
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(quick('ctrl'))}).getAttribute('aria-pressed')`), 'false');
      await exact(chord('ctrl-x-ctrl-s'), '\x18\x13', 'Herdr Claude current focused pane');
      await clear();
      await evaluate('qa.focusHerdr("remote-shell")');
      assert.equal((await state()).context, null, 'a plain focused shell does not borrow a sibling Agent');
      await evaluate('qa.focusHerdr("remote-b")');
      assert.equal((await state()).mainLabel, 'Claude Code', 'cached focus change restores the focused profile');
      for (const invalid of [{ available: false }, { resources_complete: false }, { selector: 'other@owner' }, { focused_pane_id: 'missing' }]) {
        await evaluate('qa.patchHerdr({available:true,resources_complete:true,selector:"dev@owner"});qa.focusHerdr("remote-b")');
        await evaluate(`qa.patchHerdr(${JSON.stringify(invalid)})`);
        assert.equal((await state()).context, null, `unusable Herdr focus falls back: ${JSON.stringify(invalid)}`);
      }
      await evaluate('qa.patchHerdr({available:true,resources_complete:true,selector:"dev@owner"});qa.focusHerdr("remote-a")');
      await evaluate('qa.select("native-a")');
      assert.equal((await state()).context.agent, 'codex');
      assert.deepEqual((await state()).output, []);
    });

    await check('herdr-snapshot-validity-falls-back-cancels-and-recovers', async () => {
      await profile('codex');
      const nativeContext = (await state()).context;
      try {
        await evaluate('qa.useHerdr("remote-a")');
        const originalContext = (await state()).context;
        const cachedSnapshot = await evaluate('qa.herdrSnapshot()');
        assert.equal(cachedSnapshot.available, true);
        assert.equal((await state()).connectionState, 'connected');
        const point = await evaluate(`qa.point(${JSON.stringify(chord('alt-up'))})`);
        await browser.dispatchMouse([
          { type: 'mouseMoved', ...point },
          { type: 'mousePressed', ...point, button: 'left', clickCount: 1 },
        ]);
        await evaluate('qa.setHerdrSnapshotValid(false)');
        assert.equal((await state()).context, null, 'metadata failure immediately removes the Agent profile while the PTY stays connected');
        assert.equal((await state()).connectionState, 'connected');
        assert.equal(await evaluate('document.querySelectorAll("#mobileShortcuts .mobile-agent-keyboard").length'), 0);
        assert.deepEqual(await evaluate('qa.herdrSnapshot()'), cachedSnapshot, 'invalidating metadata retains the same cached available snapshot');
        await browser.dispatchMouse([{ type: 'mouseReleased', ...point, button: 'left', clickCount: 1 }]);
        assert.deepEqual((await state()).output, [], 'metadata invalidation cancels the in-flight press without sending input');
        await evaluate('qa.setHerdrSnapshotValid(false);qa.setHerdrSnapshotValid(true)');
        assert.deepEqual((await state()).context, originalContext, 'successful unchanged metadata restores the same Agent identity');
        assert.deepEqual(await evaluate('qa.herdrSnapshot()'), cachedSnapshot);
        await exact(chord('ctrl-bracket-right'), '\x1d', 'restored unchanged Herdr snapshot');
        await clear();

        await evaluate('qa.setHerdrSnapshotValid(false);qa.select("native-a")');
        assert.deepEqual((await state()).context, nativeContext, 'invalid Herdr metadata leaves native Agent context usable');
        await exact(chord('ctrl-bracket-right'), '\x1d', 'native input while Herdr metadata is invalid');
        await clear();
        await evaluate('qa.setHerdrSnapshotValid(true);qa.setHerdrSnapshotValid(false)');
        assert.deepEqual((await state()).context, nativeContext, 'Herdr validity changes do not replace native context');
        await evaluate('qa.useHerdr("remote-a")');
        assert.equal((await state()).context, null, 'returning to Herdr requires a successful metadata snapshot');
        await evaluate('qa.setHerdrSnapshotValid(true)');
        assert.deepEqual((await state()).context, originalContext);

        await toggle(false);
        for (const valid of [false, true]) {
          await evaluate(`qa.setHerdrSnapshotValid(${valid})`);
          assert.equal((await state()).context, null, 'metadata events respect the disabled setting');
          assert.equal(await evaluate('document.querySelectorAll("#mobileShortcuts .mobile-agent-keyboard").length'), 0);
        }
        assert.equal((await state()).connectionState, 'connected');
        assert.deepEqual(await evaluate('qa.herdrSnapshot()'), cachedSnapshot);
        assert.deepEqual((await state()).output, [], 'metadata and setting transitions emit no terminal input');
      } finally {
        await evaluate('qa.setHerdrSnapshotValid(true)');
      }
    });

    await check('stale-press-cannot-send-after-target-or-identity-change', async () => {
      for (const [label, change] of [
        ['rendered native pane switch', 'qa.useNative("native-b","codex")'],
        ['native getter changes before rerender', 'qa.useNative("native-b","codex","process-1",false)'],
        ['same pane replacement process', 'qa.useNative("native-a","codex","new-process",false)'],
        ['Herdr getter focus changes before rerender', 'qa.focusHerdr("remote-b",false)'],
      ]) {
        await profile('codex');
        if (label.startsWith('Herdr')) await evaluate('qa.useHerdr("remote-a")');
        const point = await evaluate(`qa.point(${JSON.stringify(chord('alt-up'))})`);
        await browser.dispatchMouse([{ type: 'mouseMoved', ...point }, { type: 'mousePressed', ...point, button: 'left', clickCount: 1 }]);
        await evaluate(change);
        await browser.dispatchMouse([{ type: 'mouseReleased', ...point, button: 'left', clickCount: 1 }]);
        assert.deepEqual((await state()).output, [], `${label}: releasing an old press must send no bytes to either terminal`);
      }
    });

    await check('runtime-language-preserves-page-focus-scroll-and-intent', async () => {
      await browser.command(['set', 'viewport', '320', '568']);
      await profile('codex');
      await tap(page('nav'));
      await evaluate('qa.setLocale("en")');
      assert.equal((await state()).page, 'nav', 'language change preserves the selected page');
      assert.deepEqual((await state()).groupLabels, ['Send and queue', 'Questions and control', 'Agent and modes', 'Edit and view']);
      await tap(page('main'));
      await evaluate(`document.querySelector(${JSON.stringify(chord('ctrl-t'))}).focus({preventScroll:true});document.querySelector(${JSON.stringify(main)}).scrollTop=65`);
      const before = await evaluate(`({scroll:document.querySelector(${JSON.stringify(main)}).scrollTop,focus:document.activeElement?.getAttribute('data-mobile-chord')})`);
      assert.ok(before.scroll > 0, 'the profile actually scrolls in a narrow viewport');
      await evaluate('qa.setLocale("zh-CN")');
      const after = await evaluate(`({scroll:document.querySelector(${JSON.stringify(main)}).scrollTop,focus:document.activeElement?.getAttribute('data-mobile-chord')})`);
      assert.deepEqual(after, before, 'language rerender preserves scroll and the focused action in the same context');
      for (const backend of ['native', 'herdr']) {
        await evaluate(backend === 'native' ? 'qa.useNative("native-a","codex")' : 'qa.useHerdr("remote-b")');
        await expand();
        await tap(`${dock} [data-mobile-action="toggle-system-keyboard"]`);
        const focused = await state();
        assert.equal(focused.keyboardIntent, 'true');
        assert.equal(focused.activeElement, `terminal-${focused.paneId}`);
        await evaluate('qa.setLocale("en");qa.refresh()');
        const updated = await state();
        assert.equal(updated.keyboardIntent, 'true', `${backend}: rerender keeps explicit system keyboard intent`);
        assert.equal(updated.activeElement, focused.activeElement, `${backend}: rerender keeps terminal focus`);
        await tap(`${dock} [data-mobile-action="toggle-system-keyboard"]`);
        await evaluate('qa.setLocale("zh-CN")');
        assert.equal((await state()).keyboardIntent, 'false', `${backend}: closed system keyboard stays closed`);
      }
      assert.deepEqual((await state()).output, [], 'language, page and focus changes send no terminal input');
    });

    await check('opencode-locale-rerender-keeps-nav-up-focus', async () => {
      await profile('opencode');
      const navUp = `${dock} [data-mobile-panel="nav"] [data-mobile-shortcut="up"]`;
      assert.equal(await evaluate(`Boolean(document.querySelector(${JSON.stringify(shortcut('up'))}))`), true, 'OpenCode also has a main-page history Up action');
      await tap(page('nav'));
      await evaluate(`document.querySelector(${JSON.stringify(navUp)}).focus({preventScroll:true})`);
      const focused = () => evaluate(`(()=>{
        const active=document.activeElement,nav=document.querySelector('#mobileShortcuts [data-mobile-panel="nav"]');
        return {page:qa.state().page,shortcut:active?.getAttribute('data-mobile-shortcut'),panel:active?.closest('[data-mobile-panel]')?.dataset.mobilePanel,
          exact:active===document.querySelector(${JSON.stringify(navUp)}),visible:active?.getClientRects().length>0,navHidden:nav.hidden,mainHidden:document.querySelector(${JSON.stringify(main)}).hidden};
      })()`);
      const expected = { page: 'nav', shortcut: 'up', panel: 'nav', exact: true, visible: true, navHidden: false, mainHidden: true };
      assert.deepEqual(await focused(), expected);
      try {
        for (const locale of ['en', 'zh-CN']) {
          await evaluate(`qa.setLocale(${JSON.stringify(locale)})`);
          assert.deepEqual(await focused(), expected, `${locale}: focus stays on visible Nav Up instead of hidden main history Up`);
          assert.deepEqual((await state()).output, [], 'language rerender and focus restoration send no input');
        }
        await browser.press('Enter');
        assert.deepEqual((await state()).output.map((event) => event.data), ['\x1b[A'], 'the retained visible Nav Up remains keyboard-activatable');
        await clear();
      } finally {
        await evaluate('qa.setLocale("zh-CN")');
      }
    });

    await check('responsive-targets-and-scroll-gestures-send-no-input', async () => {
      for (const [width, height] of [[320, 568], [390, 844], [844, 390], [390, 280]]) {
        await browser.command(['set', 'viewport', String(width), String(height)]);
        for (const agent of ['codex', 'claude', 'opencode', 'pi']) {
          await profile(agent);
          await settle();
          const geometry = await evaluate(`(()=>{
            const dock=document.querySelector('#mobileShortcuts'),panel=document.querySelector(${JSON.stringify(main)}),rect=dock.getBoundingClientRect();
            const buttons=[...panel.querySelectorAll('button')].map(button=>{const r=button.getBoundingClientRect();return {label:button.getAttribute('aria-label'),width:r.width,height:r.height,overflow:button.scrollWidth-button.clientWidth}});
            return {pageOverflow:document.documentElement.scrollWidth-innerWidth,left:rect.left,right:rect.right,top:rect.top,bottom:rect.bottom,panelOverflow:panel.scrollWidth-panel.clientWidth,buttons};
          })()`);
          assert.ok(geometry.pageOverflow <= 1 && geometry.panelOverflow <= 1, `${agent} ${width}x${height}: no horizontal overflow: ${JSON.stringify(geometry)}`);
          assert.ok(geometry.left >= -1 && geometry.right <= width + 1 && geometry.top >= -1 && geometry.bottom <= height + 1, `${agent} ${width}x${height}: dock fits the viewport`);
          assert.ok(geometry.buttons.every((button) => button.width >= 44 && button.height >= 44), `${agent} ${width}x${height}: all Agent touch targets are at least 44px: ${JSON.stringify(geometry.buttons)}`);
          assert.ok(geometry.buttons.every((button) => button.overflow <= 1), `${agent} ${width}x${height}: key text fits within its target`);
          assertWrapping(await measure(`${agent}-${width}x${height}`), `${agent} ${width}x${height}`);
        }
        await profile('codex');
        await screenshot(`viewport-${width}x${height}`);
        await evaluate(`document.querySelector(${JSON.stringify(main)}).scrollTop=0`);
        const point = await evaluate(`qa.point(${JSON.stringify(shortcut('enter'))})`);
        await browser.dispatchTouch([
          touch('touchStart', point, 25),
          touch('touchMove', { x: point.x, y: point.y - 20 }, 25),
          touch('touchMove', { x: point.x, y: point.y - 55 }, 25),
          touch('touchMove', { x: point.x, y: Math.max(2, point.y - 95) }, 25),
          touch('touchEnd', point),
        ]);
        assert.deepEqual((await state()).output, [], `${width}x${height}: scrolling over a semantic key sends no bytes`);
        assert.ok(await evaluate(`document.querySelector(${JSON.stringify(main)}).scrollTop`) > 0, `${width}x${height}: touch gesture scrolls the profile`);
        await clear();
      }
    });

    await check('native-codex-english-320-visual-evidence', async () => {
      await browser.command(['set', 'viewport', '320', '568']);
      await profile('codex');
      await evaluate(`qa.setLocale("en");document.querySelector('[data-mobile-panel="main"]').scrollTop=0`);
      await settle();
      const data = await measure('native-codex-en-320');
      assertWrapping(data, 'native Codex English 320');
      assert.ok(data.keys.every((key) => key.width >= 44 && key.height >= 56), 'wrapped English keys remain full touch targets');
      await screenshot('native-codex-en-320-top');
      await evaluate(`document.querySelector('[data-mobile-panel="main"]').scrollTop=10000`);
      await screenshot('native-codex-en-320-bottom');
      console.log(`MEASURE native Codex English 320: panel=${data.panel.width}x${data.panel.height}, contentHeight=${data.contentHeight}, keys=${Math.min(...data.keys.map(key=>key.width))}px wide, heights=${Math.min(...data.keys.map(key=>key.height))}..${Math.max(...data.keys.map(key=>key.height))}px`);
      assert.deepEqual((await state()).output, []);
      await evaluate('qa.setLocale("zh-CN")');
    });

    await check('vertical-named-tabs-measured-target-bounds', async () => {
      await evaluate('document.querySelector("#webshell").dataset.tabLayout="vertical";document.querySelector("#webshell").classList.add("has-named-tabs")');
      try {
        for (const [width, height] of [[320, 568], [390, 844], [600, 568], [760, 568], [761, 568], [844, 390]]) {
          await browser.command(['set', 'viewport', String(width), String(height)]);
          for (const agent of width === 320 ? ['codex', 'claude', 'opencode', 'pi'] : ['codex']) {
            await profile(agent);
            await settle();
            const data = await measure(`vertical-named-${agent}-${width}x${height}`);
            assertWrapping(data, `vertical named ${agent} ${width}x${height}`);
            assert.ok(data.keys.every((key) => key.width >= 44 && key.height >= 44), `vertical named ${width}: targets retain at least 44px: ${JSON.stringify(data.keys.map(key=>({label:key.label,width:key.width,height:key.height})))}`);
            if (width <= 760) {
              assert.ok(Math.abs(data.dock.x) <= 1 && Math.abs(data.dock.width - width) <= 1, `vertical named ${width}: existing mobile override restores full dock width`);
            } else {
              assert.ok(data.dock.x >= 155 && data.dock.width <= width - 155, `vertical named ${width}: desktop sidebar reservation is active`);
            }
            assert.ok(await evaluate('document.documentElement.scrollWidth<=innerWidth+1'), 'vertical tabs add no page overflow');
            if (width === 320 && agent === 'codex') await screenshot('vertical-named-codex-320');
            console.log(`MEASURE vertical named ${agent} ${width}x${height}: dock x=${data.dock.x}, width=${data.dock.width}, minKey=${Math.min(...data.keys.map(key=>key.width))}x${Math.min(...data.keys.map(key=>key.height))}`);
          }
        }
        assert.deepEqual((await state()).output, []);
      } finally {
        await evaluate('document.querySelector("#webshell").dataset.tabLayout="horizontal";document.querySelector("#webshell").classList.remove("has-named-tabs")');
      }
    });

    await check('off-clears-transient-state-and-browser-errors', async () => {
      await browser.command(['set', 'viewport', '390', '844']);
      await profile('codex');
      await tap(quick('alt'));
      await toggle(false);
      assert.equal((await state()).context, null);
      assert.equal(await evaluate(`document.querySelector(${JSON.stringify(quick('alt'))}).getAttribute('aria-pressed')`), 'false');
      assert.equal(await evaluate('document.querySelectorAll("#mobileShortcuts .mobile-agent-keyboard").length'), 0);
      await evaluate('qa.useNative("native-a","claude")');
      await toggle(true);
      assert.equal((await state()).mainLabel, 'Claude Code');
      await exact(chord('ctrl-x-ctrl-s'), '\x18\x13', 're-enabled profile uses the current Agent immediately');
      const errors = JSON.parse(await browser.command(['errors', '--json']));
      assert.deepEqual(errors.data.errors, [], 'browser has no uncaught runtime errors');
      await screenshot('complete');
    });

    const failed = results.filter((result) => result.status === 'failed');
    assert.equal(failed.length, 0, failed.map((result) => `${result.name}: ${result.error}`).join('\n'));
  } catch (error) {
    failure = error;
    await screenshot('failure').catch(() => {});
  } finally {
    try { await browser.close(); } catch (error) { cleanupErrors.push(error.message); }
    try { await server.close(); } catch (error) { cleanupErrors.push(error.message); }
    if (cleanupErrors.length && !failure) failure = new Error(`Cleanup failed: ${cleanupErrors.join('; ')}`);
    await writeFile(join(artifactDirectory, 'measurements.json'), JSON.stringify(measurements, null, 2));
    await writeFile(join(artifactDirectory, 'results.json'), JSON.stringify({ session: browser.session, results, error: failure?.message ?? null, cleanupErrors }, null, 2));
  }
  if (failure) { failure.message += `\nArtifacts: ${artifactDirectory}`; throw failure; }
  return { scenario: 'mobile-agent-keyboard', status: 'passed', checks: results.length, artifactDirectory };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  runMobileAgentKeyboardScenario().then((result) => console.log(JSON.stringify(result))).catch((error) => {
    console.error(error.stack ?? error); process.exitCode = 1;
  });
}
