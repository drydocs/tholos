import type { AlertPayload, ClassifiedEvent, LivenessAlert } from "./types.js";

export interface SendAlertOptions {
  webhookUrl: string;
  timeoutMs: number;
  /** Attempts before giving up on one alert. A failed alert is logged to
   * stderr, never thrown — a webhook outage must not take the poller down
   * with it, since that would be strictly worse than the alert just not
   * arriving. */
  maxAttempts?: number;
}

function backoffMs(attempt: number): number {
  return Math.min(1000 * 2 ** attempt, 10_000);
}

async function postJson(
  url: string,
  body: unknown,
  timeoutMs: number,
): Promise<void> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new Error(
        `webhook returned ${res.status} ${res.statusText}${text ? `: ${text.slice(0, 300)}` : ""}`,
      );
    }
  } finally {
    clearTimeout(timeout);
  }
}

/** POSTs an alert payload to the configured webhook, retrying transient
 * failures with capped exponential backoff. Never throws: this is the one
 * function in the service where "the alert didn't send" must not become
 * "the whole poller crashed" — a failure here is logged and swallowed, and
 * the poll loop moves on. */
export async function sendAlert(
  payload: AlertPayload,
  opts: SendAlertOptions,
): Promise<void> {
  const maxAttempts = opts.maxAttempts ?? 3;
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    try {
      await postJson(opts.webhookUrl, payload, opts.timeoutMs);
      return;
    } catch (err) {
      const isLastAttempt = attempt === maxAttempts - 1;
      console.error(
        `[alerts] webhook attempt ${attempt + 1}/${maxAttempts} failed: ${
          (err as Error).message
        }${isLastAttempt ? " — giving up on this alert" : ", retrying"}`,
      );
      if (isLastAttempt) return;
      await new Promise((r) => setTimeout(r, backoffMs(attempt)));
    }
  }
}

export function buildEventAlertPayload(event: ClassifiedEvent): AlertPayload {
  return {
    type: "contract_event",
    event,
    timestamp: new Date().toISOString(),
  };
}

export function buildLivenessAlertPayload(alert: LivenessAlert): AlertPayload {
  return {
    type: "monitor_liveness",
    alert,
    timestamp: new Date().toISOString(),
  };
}
