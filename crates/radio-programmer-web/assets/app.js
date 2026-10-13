const token = document.querySelector('meta[name="afik-session-token"]').content;
const statusText = document.querySelector("#status");
const statusDot = document.querySelector("#status-dot");

function setStatus(message, kind = "ok") {
  statusText.textContent = message;
  statusDot.className = `status-dot ${kind}`;
}

async function checked(response) {
  if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
  return response;
}

function download(blob, name) {
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = name;
  link.click();
  URL.revokeObjectURL(link.href);
}

async function refresh() {
  try {
    const state = await checked(await fetch("/api/state")).then(r => r.json());
    document.querySelector("#generation").textContent = `Generation ${state.generation}`;
    document.querySelector("#capabilities").innerHTML = Object.entries(state.capabilities)
      .map(([key, value]) => `<dt>${key}</dt><dd>${value}</dd>`).join("");
    document.querySelector("#objects").innerHTML = state.objects.length
      ? state.objects.map(o => `<tr><td>${o.kind}</td><td>${o.id}</td><td>${o.bytes}</td></tr>`).join("")
      : '<tr><td colspan="3">No configuration objects installed</td></tr>';
    setStatus("Device state refreshed");
  } catch (error) { setStatus(error.message, "error"); }
}

async function sendProject(path, downloadName) {
  const body = document.querySelector("#project").value;
  const response = await checked(await fetch(path, {
    method: "POST",
    headers: {"Content-Type": "text/plain; charset=utf-8", "X-Afik-Session": token, "X-Afik-Confirm": "replace-configuration"},
    body,
  }));
  if (downloadName) download(await response.blob(), downloadName);
  else setStatus(await response.text());
}

document.querySelector("#refresh").addEventListener("click", refresh);
document.querySelector("#compile").addEventListener("click", async () => {
  try { await sendProject("/api/compile", "afik-configuration.afik"); setStatus("Compiled image downloaded"); }
  catch (error) { setStatus(error.message, "error"); }
});
document.querySelector("#write").addEventListener("click", async () => {
  if (!document.querySelector("#confirm-write").checked) return setStatus("Confirm write intent first", "error");
  try { await sendProject("/api/write"); document.querySelector("#confirm-write").checked = false; await refresh(); }
  catch (error) { setStatus(error.message, "error"); }
});
document.querySelector("#backup").addEventListener("click", async () => {
  try { download(await checked(await fetch("/api/backup")).then(r => r.blob()), "afik-backup.afik"); setStatus("Backup downloaded"); }
  catch (error) { setStatus(error.message, "error"); }
});
document.querySelector("#restore").addEventListener("click", async () => {
  const file = document.querySelector("#restore-file").files[0];
  if (!file) return setStatus("Choose a restore image first", "error");
  if (!document.querySelector("#confirm-restore").checked) return setStatus("Confirm restore intent first", "error");
  try {
    const response = await checked(await fetch("/api/restore", {method: "POST", headers: {"Content-Type": "application/octet-stream", "X-Afik-Session": token, "X-Afik-Confirm": "replace-configuration"}, body: file}));
    setStatus(await response.text());
    document.querySelector("#confirm-restore").checked = false;
    await refresh();
  } catch (error) { setStatus(error.message, "error"); }
});

refresh();
