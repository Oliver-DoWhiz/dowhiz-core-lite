import "./styles.css";

const form = document.querySelector("#task-form");
const payloadPreview = document.querySelector("#payload-preview");
const submitState = document.querySelector("#submit-state");
const taskId = document.querySelector("#task-id");
const taskStatus = document.querySelector("#task-status");
const taskClaimed = document.querySelector("#task-claimed");
const lastPoll = document.querySelector("#last-poll");
const taskError = document.querySelector("#task-error");
const stdoutStream = document.querySelector("#stdout-stream");
const replyFrame = document.querySelector("#reply-frame");
const runButton = document.querySelector("#run-button");

const fieldSelectors = [
  "#customer-email",
  "#reply-to",
  "#subject",
  "#prompt",
];

let pollHandle = null;

function buildPayload() {
  const customerEmail = document.querySelector("#customer-email").value.trim();
  const replyTo = document.querySelector("#reply-to").value.trim();
  const subject = document.querySelector("#subject").value.trim();
  const prompt = document.querySelector("#prompt").value.trim();

  return {
    customer_email: customerEmail,
    subject,
    prompt,
    channel: "email",
    reply_to: replyTo,
  };
}

function renderPayloadPreview() {
  payloadPreview.textContent = JSON.stringify(buildPayload(), null, 2);
}

async function submitTask(event) {
  event.preventDefault();
  resetError();
  stopPolling();
  stdoutStream.textContent = "Submitting request...";
  replyFrame.srcdoc = "<p style='font-family: sans-serif; padding: 16px;'>Waiting for reply HTML.</p>";
  setTaskMeta("Submitting...", "pending", "No", "Just now");

  runButton.disabled = true;
  submitState.textContent = "Sending POST /tasks request...";

  try {
    const response = await fetch("/tasks", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify(buildPayload()),
    });

    if (!response.ok) {
      throw new Error(await response.text());
    }

    const queuedTask = await response.json();
    submitState.textContent = "Task accepted. Polling GET /tasks/{id} every second.";
    setTaskMeta(queuedTask.id, queuedTask.status, "No", timestamp());
    stdoutStream.textContent = "Task queued. Waiting for worker pickup.";
    startPolling(queuedTask.id);
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
    stdoutStream.textContent = "Request failed before the worker started.";
  } finally {
    runButton.disabled = false;
  }
}

function startPolling(id) {
  pollTask(id);
  pollHandle = window.setInterval(() => pollTask(id), 1000);
}

function stopPolling() {
  if (pollHandle !== null) {
    window.clearInterval(pollHandle);
    pollHandle = null;
  }
}

async function pollTask(id) {
  try {
    const response = await fetch(`/tasks/${id}`);
    if (!response.ok) {
      throw new Error(await response.text());
    }

    const snapshot = await response.json();
    const claimed = snapshot.status !== "pending" ? "Yes" : "No";

    setTaskMeta(snapshot.id, snapshot.status, claimed, timestamp());
    stdoutStream.textContent = snapshot.stdout || "No streamed output yet.";

    if (snapshot.replyHtml) {
      replyFrame.srcdoc = snapshot.replyHtml;
    }

    if (snapshot.error) {
      showError(snapshot.error);
    } else {
      resetError();
    }

    if (snapshot.status === "completed" || snapshot.status === "failed") {
      submitState.textContent = `Task ${snapshot.status}. Polling stopped.`;
      stopPolling();
    }
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
    submitState.textContent = "Polling failed.";
    stopPolling();
  }
}

function setTaskMeta(id, status, claimed, polledAt) {
  taskId.textContent = id;
  taskStatus.textContent = status;
  taskClaimed.textContent = claimed;
  lastPoll.textContent = polledAt;
}

function showError(message) {
  taskError.hidden = false;
  taskError.textContent = message;
}

function resetError() {
  taskError.hidden = true;
  taskError.textContent = "";
}

function timestamp() {
  return new Date().toLocaleTimeString();
}

for (const selector of fieldSelectors) {
  document.querySelector(selector).addEventListener("input", renderPayloadPreview);
}

form.addEventListener("submit", submitTask);
renderPayloadPreview();
