import assert from "node:assert/strict";
import { test } from "node:test";
import { classify } from "./events.js";

test("classify: pausing is critical", () => {
  const result = classify("PauseUpdated", { paused: true });
  assert.equal(result.severity, "critical");
});

test("classify: unpausing is info, not critical", () => {
  const result = classify("PauseUpdated", { paused: false });
  assert.equal(result.severity, "info");
});

test("classify: PauseUpdated with an unrecognized payload stays critical (fail safe, not fail open)", () => {
  const result = classify("PauseUpdated", {});
  assert.equal(result.severity, "critical");
});

test("classify: issue #189's named events are all critical", () => {
  for (const name of [
    "AdminUpdated",
    "AdminRotationProposed",
    "RotationCancelled",
    "RoundCancelled",
  ]) {
    assert.equal(classify(name, {}).severity, "critical", name);
  }
});

test("classify: liveness events added upstream after the issue was filed are also critical", () => {
  for (const name of ["StalledDisputeReclaimed", "RoundVoided"]) {
    assert.equal(classify(name, {}).severity, "critical", name);
  }
});

test("classify: ordinary protocol lifecycle events are info", () => {
  for (const name of ["Asserted", "Disputed", "Finalized", "Resolved"]) {
    assert.equal(classify(name, {}).severity, "info", name);
  }
});

test("classify: an unregistered event name degrades to warning, not silence", () => {
  const result = classify("SomeFutureEventNotYetRegistered", { x: 1 });
  assert.equal(result.severity, "warning");
  assert.match(result.description, /not in this service's registry/);
});
