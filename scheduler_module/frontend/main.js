import "./styles.css";

const form = document.querySelector("#task-form");
const payloadPreview = document.querySelector("#payload-preview");
const submitState = document.querySelector("#submit-state");
const attachmentInput = document.querySelector("#attachment-input");
const attachmentStatus = document.querySelector("#attachment-status");
const attachmentList = document.querySelector("#attachment-list");
const clearAttachmentsButton = document.querySelector("#clear-attachments");
const taskId = document.querySelector("#task-id");
const taskStatus = document.querySelector("#task-status");
const taskClaimed = document.querySelector("#task-claimed");
const lastPoll = document.querySelector("#last-poll");
const taskError = document.querySelector("#task-error");
const stdoutStream = document.querySelector("#stdout-stream");
const replyFrame = document.querySelector("#reply-frame");
const runButton = document.querySelector("#run-button");
const accountIdInput = document.querySelector("#account-id");
const accountStatus = document.querySelector("#account-status");
const generateAccountIdButton = document.querySelector("#generate-account-id");

const fieldSelectors = [
  "#customer-email",
  "#reply-to",
  "#account-id",
  "#subject",
  "#prompt",
];

let pollHandle = null;
let attachmentRefs = [];
let registerAccountId = false;

function buildPayload() {
  const customerEmail = document.querySelector("#customer-email").value.trim();
  const replyTo = document.querySelector("#reply-to").value.trim();
  const accountId = accountIdInput.value.trim();
  const subject = document.querySelector("#subject").value.trim();
  const prompt = document.querySelector("#prompt").value.trim();

  return {
    customer_email: customerEmail,
    subject,
    prompt,
    channel: "email",
    reply_to: replyTo,
    account_id: accountId,
    register_account_id: registerAccountId && accountId.length > 0,
    attachment_refs: attachmentRefs,
  };
}

function renderPayloadPreview() {
  payloadPreview.textContent = JSON.stringify(buildPayload(), null, 2);
}

function renderAttachmentList() {
  attachmentList.innerHTML = "";

  if (attachmentRefs.length === 0) {
    attachmentStatus.textContent = "No attachments uploaded.";
    return;
  }

  attachmentStatus.textContent = `${attachmentRefs.length} attachment${attachmentRefs.length === 1 ? "" : "s"} staged in the gateway.`;
  for (const attachment of attachmentRefs) {
    const item = document.createElement("li");
    item.className = "attachment-pill";
    item.textContent = `${attachment.file_name} (${formatBytes(attachment.size_bytes)})`;
    attachmentList.appendChild(item);
  }
}

async function uploadSelectedFiles(event) {
  const files = Array.from(event.target.files || []);
  if (files.length === 0) {
    return;
  }

  attachmentInput.disabled = true;
  clearAttachmentsButton.disabled = true;
  attachmentStatus.textContent = `Uploading ${files.length} file${files.length === 1 ? "" : "s"}...`;

  try {
    const body = new FormData();
    for (const file of files) {
      body.append("file", file, file.name);
    }

    const response = await fetch("/uploads", {
      method: "POST",
      body,
    });

    if (!response.ok) {
      throw new Error(await response.text());
    }

    const result = await response.json();
    attachmentRefs = attachmentRefs.concat(result.attachments || []);
    renderAttachmentList();
    renderPayloadPreview();
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
    attachmentStatus.textContent = "Attachment upload failed.";
  } finally {
    attachmentInput.value = "";
    attachmentInput.disabled = false;
    clearAttachmentsButton.disabled = attachmentRefs.length === 0;
  }
}

function clearAttachments() {
  attachmentRefs = [];
  renderAttachmentList();
  renderPayloadPreview();
  clearAttachmentsButton.disabled = true;
}

async function generateAccountId() {
  generateAccountIdButton.disabled = true;
  accountStatus.textContent = "Generating a fresh account ID...";
  resetError();

  try {
    const response = await fetch("/account-ids/suggest", {
      method: "POST",
    });

    if (!response.ok) {
      throw new Error(await response.text());
    }

    const result = await response.json();
    accountIdInput.value = result.account_id;
    registerAccountId = true;
    accountStatus.textContent = "Generated a new account ID. The next task submission will reserve it.";
    renderPayloadPreview();
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
    accountStatus.textContent = "Unable to generate an account ID right now.";
  } finally {
    generateAccountIdButton.disabled = false;
  }
}

function handleAccountIdInput() {
  registerAccountId = false;

  if (accountIdInput.value.trim().length === 0) {
    accountStatus.textContent = "Leave blank to use the legacy email-only lookup flow.";
  } else {
    accountStatus.textContent = "Using the provided account ID. Existing accounts will be reused.";
  }

  renderPayloadPreview();
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

function formatBytes(size) {
  if (size < 1024) {
    return `${size} B`;
  }
  if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(1)} KB`;
  }
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

for (const selector of fieldSelectors) {
  if (selector === "#account-id") {
    document.querySelector(selector).addEventListener("input", handleAccountIdInput);
  } else {
    document.querySelector(selector).addEventListener("input", renderPayloadPreview);
  }
}

attachmentInput.addEventListener("change", uploadSelectedFiles);
clearAttachmentsButton.addEventListener("click", clearAttachments);
generateAccountIdButton.addEventListener("click", generateAccountId);
form.addEventListener("submit", submitTask);
clearAttachmentsButton.disabled = true;
renderAttachmentList();
renderPayloadPreview();
