import { ConfigError, loadConfig } from "./config.js";
import { runOnce } from "./poller.js";

function redactUrl(url: string): string {
  try {
    const u = new URL(url);
    return `${u.protocol}//${u.host}${u.pathname !== "/" ? "/…" : ""}`;
  } catch {
    return "(invalid URL)";
  }
}

async function main(): Promise<void> {
  let config;
  try {
    config = loadConfig();
  } catch (err) {
    if (err instanceof ConfigError) {
      console.error(`[config] ${err.message}`);
      console.error("See packages/tholos-monitor/README.md for configuration.");
      process.exitCode = 1;
      return;
    }
    throw err;
  }

  console.log(
    `[main] tholos-monitor run starting. Alert webhook: ${redactUrl(config.alertWebhookUrl)}`,
  );

  const hadFailures = await runOnce(config);
  if (hadFailures) {
    process.exitCode = 1;
  }
}

main().catch((err) => {
  console.error("[main] fatal error:", err);
  process.exitCode = 1;
});
