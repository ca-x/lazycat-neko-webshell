import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const directory = fileURLToPath(new URL("./", import.meta.url));
const modelOnly = process.argv.includes("--model-only");
const baselineControls = process.argv.includes("--baseline-controls");
const skipControlsTests = process.argv.includes("--skip-controls-tests");
const client = resolve(directory, "../..");
const manifest = await readFile(join(client, "Cargo.toml"), "utf8");
const dependency = (name) => {
  const line = manifest.split("\n").find((line) => line.startsWith(`${name} = `));
  if (!line) throw new Error(`Missing dependency: ${name}`);
  return line;
};
const temporary = await mkdtemp(join(tmpdir(), "neko-preferences-"));
try {
  await writeFile(join(temporary, "Cargo.lock"), await readFile(join(client, "Cargo.lock")));
  await writeFile(join(temporary, "Cargo.toml"), `[package]
name = "neko-preferences-check"
version = "0.0.0"
edition = "2024"
[workspace]
[lib]
path = "lib.rs"
[dependencies]
neko-remoted-protocol = { path = ${JSON.stringify(resolve(client, "../protocol"))} }
neko-ghostty = { path = ${JSON.stringify(resolve(client, "../ghostty"))} }
${["anyhow", "serde", "serde_json", "tokio", "unicode-width", "nsh-core", "nsh-base"].map(dependency).join("\n")}
[dev-dependencies]
${["nsh-engine", "nsh-testing"].map(dependency).join("\n")}
`);
  const pathLiteral = (path) => JSON.stringify(path);
  let controlsPath = join(client, "src/ui/controls.rs");
  if (baselineControls) {
    const result = spawnSync("git", ["show", "HEAD:remoted/client/src/ui/controls.rs"], { cwd: client, encoding: "utf8" });
    if (result.status !== 0) throw new Error(result.stderr);
    controlsPath = join(temporary, "controls.rs");
    await writeFile(controlsPath, result.stdout);
  }
  await writeFile(join(temporary, "lib.rs"), `#![allow(dead_code)]
#[path = ${pathLiteral(join(client, "src/preferences.rs"))}]
pub mod preferences;
#[path = ${pathLiteral(join(client, "src/terminal.rs"))}]
pub mod terminal;
${modelOnly ? "" : `mod ui {
    #[path = ${pathLiteral(controlsPath)}]
    pub mod controls;
    #[path = ${pathLiteral(join(client, "src/ui/preferences.rs"))}]
    pub mod preferences;
    #[path = ${pathLiteral(join(client, "src/ui/terminal_colors.rs"))}]
    pub mod terminal_colors;
    #[path = ${pathLiteral(join(client, "src/ui/terminal_surface.rs"))}]
    pub mod terminal_surface;
    #[path = ${pathLiteral(join(client, "src/ui/terminal_observer.rs"))}]
    pub mod terminal_observer;
}`}
pub mod platform {
    use std::sync::Mutex;
    #[derive(Default)]
    pub struct Memory {
        pub value: Option<String>,
        pub writes: usize,
        pub fail_read: bool,
        pub fail_write: bool,
        pub read_gate: Option<std::sync::Arc<tokio::sync::Notify>>,
        pub write_gate: Option<std::sync::Arc<tokio::sync::Notify>>,
    }
    pub static STORE: Mutex<Memory> = Mutex::new(Memory { value: None, writes: 0, fail_read: false, fail_write: false, read_gate: None, write_gate: None });
    pub async fn secure_get(key: &str) -> anyhow::Result<Option<String>> {
        let _runtime = tokio::runtime::Handle::try_current()?;
        anyhow::ensure!(key == crate::preferences::STORAGE_KEY, "wrong storage key");
        let gate = { STORE.lock().expect("test store lock").read_gate.clone() };
        if let Some(gate) = gate { gate.notified().await; }
        let store = STORE.lock().expect("test store lock");
        anyhow::ensure!(!store.fail_read, "read denied");
        Ok(store.value.clone())
    }
    pub async fn secure_set(key: &str, value: &str) -> anyhow::Result<()> {
        let _runtime = tokio::runtime::Handle::try_current()?;
        anyhow::ensure!(key == crate::preferences::STORAGE_KEY, "wrong storage key");
        let gate = { STORE.lock().expect("test store lock").write_gate.clone() };
        if let Some(gate) = gate { gate.notified().await; }
        let mut store = STORE.lock().expect("test store lock");
        anyhow::ensure!(!store.fail_write, "write denied");
        store.value = Some(value.into());
        store.writes += 1;
        Ok(())
    }
}
${modelOnly ? "" : `#[cfg(test)]
#[path = ${pathLiteral(join(directory, "headless_tests.rs"))}]
mod headless_tests;
#[cfg(test)]
#[path = ${pathLiteral(join(directory, "terminal_ui_tests.rs"))}]
mod terminal_ui_tests;`}
`);
  for (const command of [
    ["test", "--offline", "--lib", "--", "--test-threads=1",
      ...(skipControlsTests ? ["--skip", "ui::controls::translation_tests"] : [])],
    ["clippy", "--offline", "--lib", "--", "-D", "warnings"],
  ]) {
    const result = spawnSync("cargo", command, {
      cwd: temporary,
      env: { ...process.env, CARGO_TARGET_DIR: join(client, "target") },
      stdio: "inherit",
    });
    if (result.error) throw result.error;
    if (result.status !== 0) { process.exitCode = result.status ?? 1; break; }
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}
