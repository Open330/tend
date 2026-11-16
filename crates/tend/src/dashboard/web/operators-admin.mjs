// Operator-only "access" dialog: who is using the dashboard right now,
// accounts enrolled as operators with the token (see /auth/enroll), viewer
// rules for read-only sign-in, and paired devices (a phone that decides,
// and, if granted, sees hosts and starts work; see docs/DASHBOARD.md
// "Paired devices"). Removing an operator, a
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

/// Lookup, then approve that exact request. The code approved is the one
/// looked up, held here: never re-read from an input that may have changed
/// while the lookup was in flight. A newer lookup or a cancel makes an older
/// lookup's late answer void.
export function devicePairing(request) {
  let pending = null;
  let attempt = 0;
  return {
    /// The request behind `code` (name, platform, scope, age), or null when
    /// a newer lookup or a cancel replaced this one.
    async lookup(code) {
      const mine = ++attempt;
      pending = null;
      const data = await request("/api/devices/lookup", {
        method: "POST",
        body: JSON.stringify({ user_code: code }),
      });
      if (mine !== attempt) return null;
      pending = code;
      return data.request;
    },
    async approve(scope) {
      if (pending === null) throw new Error("Look the code up first.");
      const code = pending;
      pending = null;
      attempt += 1;
      return request("/api/devices/approve", {
        method: "POST",
        body: JSON.stringify({ user_code: code, scope }),
      });
    },
    cancel() {
      pending = null;
      attempt += 1;
    },
  };
}

export function openAccessManager(request, { login = true, devices: pairing = false } = {}) {
  const dialog = document.createElement("dialog");
  dialog.className = "share-dialog";
  dialog.innerHTML = `<h2>Access</h2>
    <p class="access-current"></p>
    <p class="share-message" role="status"></p>
    <section class="access-devices-section" hidden>
    <h3>Devices</h3>
    <p>A paired device (the Tend app on a phone) can read and answer what needs you, on every host this dashboard covers. If you allow it, it can also see every host and agent, read their recent output, and start new work. It never types into panes or changes settings. To pair one, open the app, enter this dashboard's address, and type the code it shows here. Only enter a code shown on a device you are holding yourself, right now; never a code someone sent you. Removing the operator who approved a device, or changing the access token, does not unpair it.</p>
    <form class="access-pair-device"><label>Code shown on the device<input name="code" autocomplete="off" autocapitalize="characters" maxlength="16" placeholder="BCDF-GHJK" required></label><button type="submit">Look up</button></form>
    <form class="access-approve-device" hidden>
      <p class="access-approve-what"></p>
      <label class="access-approve-operate" hidden><input type="checkbox" name="operate"> Also let this device see every host and agent, read their recent output, and start agents that run with permissions bypassed on every covered control-mode host. Only tick this for a code on a device you are holding right now.</label>
      <div class="access-approve-buttons"><button type="submit">Approve</button> <button type="button" class="access-approve-cancel">Cancel</button></div>
    </form>
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
  const approveForm = dialog.querySelector(".access-approve-device");
  const approveWhat = approveForm.querySelector(".access-approve-what");
  const approveOperate = approveForm.querySelector(".access-approve-operate");
  const pairingFlow = devicePairing(request);
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
      const operates = device.scope === "operate";
      const item = row(
        `${device.name} (${device.platform}) · ${operates ? "decides, sees hosts, starts work" : "decides only"} · approved by ${by} · paired ${when(device.created_at)} · last used ${when(device.last_seen_at)}`,
        device.id,
        async () => {
          await request(`/api/devices/${encodeURIComponent(device.id)}/remove`, { method: "POST" });
          await refresh();
          message.textContent = "Device removed; it can no longer decide.";
        },
      );
      if (operates) {
        const limit = document.createElement("button");
        limit.type = "button";
        limit.textContent = "Limit to deciding";
        limit.addEventListener("click", async () => {
          limit.disabled = true;
          try {
            await request(`/api/devices/${encodeURIComponent(device.id)}/limit`, { method: "POST" });
            await refresh();
            message.textContent = "Device limited to deciding; it can no longer see hosts or start work.";
          } catch (error) {
            message.textContent = error.message;
            limit.disabled = false;
          }
        });
        item.append(limit);
      }
      deviceList.append(item);
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

  // Look the code up first (grants nothing), show what asked and for
  // what, then approve that exact request.
  pairForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const input = pairForm.elements.code;
    const button = pairForm.querySelector("button");
    // The code approved later is exactly the one looked up now, never what
    // the field holds when the answer arrives.
    const code = input.value;
    button.disabled = true;
    input.disabled = true;
    try {
      const asked = await pairingFlow.lookup(code);
      if (!asked) return;
      const minutes = Math.max(0, Math.round((asked.age_secs || 0) / 60));
      approveWhat.textContent = `"${asked.name}" (${asked.platform}) asked ${minutes ? `${minutes} min ago` : "just now"} to ${
        asked.scope === "operate" ? "decide, see hosts and start work" : "decide only"
      }. Check that this is the name on the device you are holding.`;
      approveOperate.hidden = asked.scope !== "operate";
      approveForm.elements.operate.checked = false;
      approveForm.hidden = false;
      pairForm.hidden = true;
      message.textContent = "";
    } catch (error) {
      message.textContent = error.message;
    } finally {
      button.disabled = false;
      input.disabled = false;
    }
  });

  function closeApproval() {
    pairingFlow.cancel();
    approveForm.hidden = true;
    pairForm.hidden = false;
    pairForm.elements.code.value = "";
  }

  approveForm.querySelector(".access-approve-cancel").addEventListener("click", closeApproval);

  approveForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const button = approveForm.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      const scope = !approveOperate.hidden && approveForm.elements.operate.checked ? "operate" : "decide";
      const data = await pairingFlow.approve(scope);
      closeApproval();
      await refresh();
      message.textContent = `Paired "${data.approved.name}" to ${
        data.approved.scope === "operate" ? "decide, see hosts and start work" : "decide only"
      }. It appears below once the app has picked up its key.`;
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
