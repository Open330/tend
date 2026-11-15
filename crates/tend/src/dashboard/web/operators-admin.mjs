// Operator-only "access" dialog: who is using the dashboard right now,
// accounts enrolled as operators with the token (see /auth/enroll), viewer
// rules for read-only sign-in, and paired devices (a phone that may only
// decide; see docs/DASHBOARD.md "Paired devices"). Removing an operator, a
// rule or a device ends what it allowed immediately.
const VIA = {
  group: "operator group",
  enrollment: "registered account",
  token: "access token",
  viewer_group: "viewer group",
  viewer_rule: "viewer rule",
};

// "ann@example.com" -> email:ann@example.com, "*@example.com" ->
// email:*@example.com; explicit email:/sub: rules pass through.
function toRule(input) {
  const text = input.trim();
  if (/^(email|sub):/.test(text)) return text;
  if (text.includes("@")) return `email:${text}`;
  return text;
}

function describeRule(rule) {
  if (rule.startsWith("email:*@")) return `anyone @${rule.slice("email:*@".length)}`;
  if (rule.startsWith("email:")) return rule.slice("email:".length);
  if (rule.startsWith("sub:")) return `subject ${rule.slice("sub:".length)}`;
  return rule;
}

export function openAccessManager(request, { login = true, devices: pairing = false } = {}) {
  const dialog = document.createElement("dialog");
  dialog.className = "share-dialog";
  dialog.innerHTML = `<h2>Access</h2>
    <p class="access-current"></p>
    <p class="share-message" role="status"></p>
    <section class="access-devices-section" hidden>
    <h3>Devices</h3>
    <p>A paired device (the Tend app on a phone) can read and answer what needs you, on every host this dashboard covers, and nothing else: no pane output, no typing into panes, no settings. To pair one, open the app, enter this dashboard's address, and type the code it shows here. Only enter a code shown on a device you are pairing yourself, right now; never a code someone sent you. Removing the operator who approved a device, or changing the access token, does not unpair it.</p>
    <form class="access-pair-device"><label>Code shown on the device<input name="code" autocomplete="off" autocapitalize="characters" maxlength="16" placeholder="BCDF-GHJK" required></label><button type="submit">Pair device</button></form>
    <div class="access-devices"></div>
    </section>
    <section class="access-login-section">
    <h3>Operators</h3>
    <p>Operators have full control. Members of the identity provider's operator group are allowed automatically and aren't listed. Other accounts become operators by entering the access token once.</p>
    <div class="access-operators"></div>
    <h3>Viewers</h3>
    <p>Viewers can see the dashboard but not change anything. Add an email address, or <code>*@domain</code> for everyone at that domain; the provider must have verified the address.</p>
    <form class="access-add-viewer"><label>Add viewer<input name="rule" autocomplete="off" maxlength="300" placeholder="ann@example.com or *@example.com" required></label><button type="submit">Add viewer</button></form>
    <div class="access-viewers"></div>
    </section>
    <button type="button" class="share-close">Close</button>`;
  const current = dialog.querySelector(".access-current");
  const message = dialog.querySelector(".share-message");
  const operators = dialog.querySelector(".access-operators");
  const viewers = dialog.querySelector(".access-viewers");
  const form = dialog.querySelector(".access-add-viewer");
  const deviceSection = dialog.querySelector(".access-devices-section");
  const deviceList = dialog.querySelector(".access-devices");
  const pairForm = dialog.querySelector(".access-pair-device");
  deviceSection.hidden = !pairing;
  dialog.querySelector(".access-login-section").hidden = !login;
  const when = (seconds) => new Date(seconds * 1000).toLocaleString();
  const note = (text) => {
    const p = document.createElement("p");
    p.textContent = text;
    return p;
  };

  function row(text, title, onRemove) {
    const item = document.createElement("div");
    item.className = "share-row";
    const label = document.createElement("p");
    label.textContent = text;
    if (title) label.title = title;
    item.append(label);
    if (onRemove) {
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "Remove";
      remove.addEventListener("click", async () => {
        remove.disabled = true;
        try {
          await onRemove();
        } catch (error) {
          message.textContent = error.message;
          remove.disabled = false;
        }
      });
      item.append(remove);
    }
    return item;
  }

  function renderOperators(data) {
    const account = data.account || { role: "operator", via: "token" };
    current.textContent = account.via === "token"
      ? "You are using the access token (no account) · operator"
      : `Signed in as ${account.email || `subject ${account.subject}`} · ${account.role} · ${VIA[account.via] || account.via}`;
    operators.replaceChildren();
    if (!data.enrollment) {
      operators.append(note("Registering accounts is turned off (dashboard.login.enrollment = false); only the operator group can operate."));
    }
    if (!data.operators.length) operators.append(note("No registered accounts yet."));
    for (const operator of data.operators) {
      const who = operator.email || `subject ${operator.subject}`;
      const flags = [
        operator.current ? "you" : null,
        operator.active ? null : "other issuer, not honored",
      ].filter(Boolean);
      operators.append(row(
        `${who}${flags.length ? ` (${flags.join(", ")})` : ""} · registered ${when(operator.created_at)} · last sign-in ${when(operator.last_seen_at)}`,
        `issuer ${operator.issuer} · subject ${operator.subject}`,
        async () => {
          if (operator.current && !window.confirm("Remove your own account? This browser will be signed out.")) {
            throw new Error("");
          }
          await request(`/api/operators/${encodeURIComponent(operator.id)}/remove`, { method: "POST" });
          if (operator.current) { window.location.reload(); return; }
          await refresh();
          message.textContent = "Operator removed and signed out.";
        },
      ));
    }
  }

  function renderViewers(data) {
    viewers.replaceChildren();
    if (data.viewer_group) {
      viewers.append(note(`Members of the provider group "${data.viewer_group}" are viewers (set in the configuration).`));
    }
    if (!data.rules.length) viewers.append(note("No viewer rules yet."));
    for (const rule of data.rules) {
      const flags = [
        rule.source === "config" ? "from configuration" : null,
        rule.active ? null : "other issuer, not honored",
      ].filter(Boolean);
      const added = rule.created_at ? ` · added ${when(rule.created_at)}` : "";
      viewers.append(row(
        `${describeRule(rule.rule)}${flags.length ? ` (${flags.join(", ")})` : ""}${added}`,
        rule.rule,
        rule.source === "config" ? null : async () => {
          await request(`/api/viewers/${encodeURIComponent(rule.id)}/remove`, { method: "POST" });
          await refresh();
          message.textContent = "Viewer rule removed; its sessions ended.";
        },
      ));
    }
  }

  function renderDevices(data) {
    deviceList.replaceChildren();
    if (!data.devices.length) {
      deviceList.append(note("No paired devices."));
      return;
    }
    for (const device of data.devices) {
      const by = device.approved_by?.email || (device.approved_by?.credential === "token" ? "the access token" : "an operator");
      deviceList.append(row(
        `${device.name} (${device.platform}) · approved by ${by} · paired ${when(device.created_at)} · last used ${when(device.last_seen_at)}`,
        device.id,
        async () => {
          await request(`/api/devices/${encodeURIComponent(device.id)}/remove`, { method: "POST" });
          await refresh();
          message.textContent = "Device removed; it can no longer decide.";
        },
      ));
    }
    const all = document.createElement("button");
    all.type = "button";
    all.textContent = "Remove all devices";
    all.addEventListener("click", async () => {
      all.disabled = true;
      try {
        await request("/api/devices/remove-all", { method: "POST" });
        await refresh();
        message.textContent = "Every device removed.";
      } catch (error) {
        message.textContent = error.message;
        all.disabled = false;
      }
    });
    deviceList.append(all);
  }

  async function refresh() {
    const [operatorData, viewerData, deviceData] = await Promise.all([
      login ? request("/api/operators", { method: "GET" }) : null,
      login ? request("/api/viewers", { method: "GET" }) : null,
      pairing ? request("/api/devices", { method: "GET" }) : null,
    ]);
    if (operatorData) renderOperators(operatorData);
    else current.textContent = "You are using the access token (no account) · operator";
    if (viewerData) renderViewers(viewerData);
    if (deviceData) renderDevices(deviceData);
  }

  pairForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const input = pairForm.elements.code;
    const button = pairForm.querySelector("button");
    button.disabled = true;
    try {
      const data = await request("/api/devices/approve", {
        method: "POST",
        body: JSON.stringify({ user_code: input.value }),
      });
      input.value = "";
      await refresh();
      message.textContent = `Paired "${data.approved.name}". It appears below once the app has picked up its key.`;
    } catch (error) {
      message.textContent = error.message;
    } finally {
      button.disabled = false;
    }
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const input = form.elements.rule;
    const button = form.querySelector("button");
    button.disabled = true;
    try {
      await request("/api/viewers", {
        method: "POST",
        body: JSON.stringify({ rule: toRule(input.value) }),
      });
      input.value = "";
      await refresh();
      message.textContent = "Viewer added.";
    } catch (error) {
      message.textContent = error.message;
    } finally {
      button.disabled = false;
    }
  });
  dialog.querySelector(".share-close").addEventListener("click", () => dialog.close());
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  refresh().catch((error) => { message.textContent = error.message; });
}
