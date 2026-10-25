const CATFLAP_APPS = new Set([
  "community.lazycat.app.catflap",
  "community.lazycat.app.catflap.dev",
]);

export function isPhysicalClientSelector(value: unknown): boolean {
  const selector = String(value ?? "").trim();
  if (!selector.startsWith("client:")) return false;
  const target = selector.slice(7);
  if (target.startsWith("catflap:")) {
    const parts = target.split(":");
    return parts.length === 3 && CATFLAP_APPS.has(parts[1]) && /^pc-[0-9a-f]{32}$/.test(parts[2]);
  }
  return /^[A-Za-z0-9._-]{1,256}$/.test(target);
}

export function launchSelector(params: URLSearchParams): string {
  const selector = (params.get("name") ?? "").trim();
  const broker = params.get("broker");
  const app = params.get("catflap_app_id");
  if (!broker && !app) return selector;
  if (broker === "catflap" && app && CATFLAP_APPS.has(app)) {
    const canonical = selector.startsWith("client:catflap:")
      ? selector : `client:catflap:${app}:${selector.slice(7)}`;
    if (selector.startsWith("client:") && isPhysicalClientSelector(canonical)
      && canonical.startsWith(`client:catflap:${app}:`)) return canonical;
  }
  if (broker === "lightos" && !app && !selector.startsWith("client:catflap:")) return selector;
  return "client:invalid-broker:unsupported";
}
