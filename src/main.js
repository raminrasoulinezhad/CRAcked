const { invoke } = window.__TAURI__.core;

const currentYear = new Date().getFullYear();

// The currently selected family member. All account data is scoped to this.
let personId = null;

// ---- money & text helpers --------------------------------------------------

function toCents(dollars) {
  const n = Number(dollars);
  return Number.isFinite(n) ? Math.round(n * 100) : 0;
}

function fmt(cents) {
  return (cents / 100).toLocaleString("en-CA", { style: "currency", currency: "CAD" });
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])
  );
}

/** Closing-room cell class: red when negative. */
function closingClass(cents) {
  return cents < 0 ? "num warn" : "num";
}

function delBtn(action, data) {
  const attrs = [`data-action="${action}"`, 'class="link-btn"'];
  for (const [k, v] of Object.entries(data)) {
    if (v !== undefined && v !== null) attrs.push(`data-${k}="${escapeHtml(v)}"`);
  }
  return `<button ${attrs.join(" ")}>delete</button>`;
}

// ---- confirmation modal ----------------------------------------------------

function confirmDialog(message) {
  return new Promise((resolve) => {
    const overlay = document.querySelector("#confirm-overlay");
    document.querySelector("#confirm-message").textContent = message;
    overlay.hidden = false;
    const ok = document.querySelector("#confirm-ok");
    const cancel = document.querySelector("#confirm-cancel");
    const cleanup = (result) => {
      overlay.hidden = true;
      ok.removeEventListener("click", onOk);
      cancel.removeEventListener("click", onCancel);
      resolve(result);
    };
    const onOk = () => cleanup(true);
    const onCancel = () => cleanup(false);
    ok.addEventListener("click", onOk);
    cancel.addEventListener("click", onCancel);
  });
}

function promptDialog(message, defaultValue = "") {
  return new Promise((resolve) => {
    const overlay = document.querySelector("#prompt-overlay");
    document.querySelector("#prompt-message").textContent = message;
    const input = document.querySelector("#prompt-input");
    input.value = defaultValue;
    overlay.hidden = false;
    input.focus();
    const ok = document.querySelector("#prompt-ok");
    const cancel = document.querySelector("#prompt-cancel");
    const cleanup = (result) => {
      overlay.hidden = true;
      ok.removeEventListener("click", onOk);
      cancel.removeEventListener("click", onCancel);
      input.removeEventListener("keydown", onKey);
      resolve(result);
    };
    const onOk = () => cleanup(input.value.trim() || null);
    const onCancel = () => cleanup(null);
    const onKey = (e) => {
      if (e.key === "Enter") onOk();
      else if (e.key === "Escape") onCancel();
    };
    ok.addEventListener("click", onOk);
    cancel.addEventListener("click", onCancel);
    input.addEventListener("keydown", onKey);
  });
}

// ---- people ---------------------------------------------------------------

async function refreshPersons() {
  const people = await invoke("list_persons");
  const sel = document.querySelector("#person-select");
  sel.innerHTML = people
    .map((p) => `<option value="${p.id}">${escapeHtml(p.name)}</option>`)
    .join("");
  // Keep current selection if it still exists, else default to the first.
  if (!people.some((p) => p.id === personId)) {
    personId = people.length ? people[0].id : null;
  }
  if (personId !== null) sel.value = String(personId);
  // Can't delete the last remaining person.
  document.querySelector("#person-delete").disabled = people.length <= 1;
}

// ---- RRSP -----------------------------------------------------------------

async function refreshRrsp() {
  const s = await invoke("get_rrsp_summary", { personId, currentYear });

  const roomEl = document.querySelector("#rrsp-room");
  roomEl.textContent = fmt(s.current_room);
  roomEl.classList.toggle("warn", s.current_room < 0);
  document.querySelector("#rrsp-total").textContent = fmt(s.total_contributed);

  const overCard = document.querySelector("#rrsp-over-card");
  overCard.hidden = s.current_over_contribution <= 0;
  if (s.current_over_contribution > 0) {
    document.querySelector("#rrsp-over").textContent = fmt(s.current_over_contribution);
  }

  document.querySelector("#rrsp-latest-year").textContent = s.latest_built_in_year;

  // Year table
  const body = document.querySelector("#rrsp-year-body");
  if (s.years.length === 0) {
    body.innerHTML =
      '<tr><td colspan="6" class="empty">No data yet — add income and contributions below.</td></tr>';
  } else {
    body.innerHTML = s.years
      .map((y) => {
        let flag = "";
        if (y.dollar_limit_missing) {
          flag = y.year <= currentYear
            ? ' <span class="flag err" title="No dollar limit on record for this year — add it below.">needs limit</span>'
            : ' <span class="flag" title="No published limit yet — estimated from income only.">est.</span>';
        }
        return `<tr>
          <td>${y.year}${flag}</td>
          <td class="num">${fmt(y.opening_room)}</td>
          <td class="num">${fmt(y.new_room)}</td>
          <td class="num">${fmt(y.available_room)}</td>
          <td class="num">${fmt(y.contribution)}</td>
          <td class="${closingClass(y.closing_room)}">${fmt(y.closing_room)}</td>
        </tr>`;
      })
      .join("");
  }

  // Warnings
  const msgs = [];
  if (s.missing_limit_years.length > 0) {
    msgs.push(
      `<strong>Update needed:</strong> no RRSP dollar limit on record for ${s.missing_limit_years.join(", ")}. ` +
      `Room for ${s.missing_limit_years.length > 1 ? "those years is" : "that year is"} uncapped and may be overstated. ` +
      `Add the CRA figure under “Annual dollar limits” below.`
    );
  }
  if (s.current_over_contribution > 0) {
    const last = s.years[s.years.length - 1];
    msgs.push(
      `You're over your room by ${fmt(s.current_over_contribution)} beyond the $2,000 buffer. ` +
      `Estimated penalty: <strong>${fmt(last.estimated_monthly_penalty)}/month</strong>.`
    );
  }
  document.querySelector("#rrsp-warnings").innerHTML = msgs
    .map((m) => `<div class="warning">${m}</div>`)
    .join("");

  // Income list
  const incomes = await invoke("list_annual_income", { personId });
  const incomeEl = document.querySelector("#rrsp-income-list");
  incomeEl.innerHTML = incomes.length === 0
    ? '<li class="muted">No income entered yet.</li>'
    : incomes
        .map((i) => {
          const pa = i.pension_adjustment_cents > 0
            ? ` <span class="muted">(PA ${fmt(i.pension_adjustment_cents)})</span>`
            : "";
          return `<li><span><strong>${i.year}</strong> — ${fmt(i.earned_income_cents)}${pa}</span> ${delBtn("del-income", { year: i.year })}</li>`;
        })
        .join("");

  // Contribution log
  await renderContribLog("RRSP", "#rrsp-contrib-body");

  document.querySelector("#rrsp-opening-room").value = (s.opening_room / 100).toString();
}

// ---- shared log renderers --------------------------------------------------

async function renderContribLog(account, selector) {
  const list = await invoke("list_contributions", { personId, account });
  const body = document.querySelector(selector);
  body.innerHTML = list.length === 0
    ? '<tr><td colspan="5" class="empty">No contributions logged yet.</td></tr>'
    : list
        .map((c) => `<tr>
          <td>${c.tax_year}</td><td>${c.date}</td>
          <td class="num">${fmt(c.amount_cents)}</td><td>${escapeHtml(c.note)}</td>
          <td class="num">${delBtn("del-contrib", { id: c.id, account, label: `${fmt(c.amount_cents)} on ${c.date}` })}</td>
        </tr>`)
        .join("");
}

async function renderWithdrawLog(account, selector) {
  const list = await invoke("list_withdrawals", { personId, account });
  const body = document.querySelector(selector);
  body.innerHTML = list.length === 0
    ? '<tr><td colspan="5" class="empty">No withdrawals logged yet.</td></tr>'
    : list
        .map((w) => `<tr>
          <td>${w.tax_year}</td><td>${w.date}</td>
          <td class="num">${fmt(w.amount_cents)}</td><td>${escapeHtml(w.note)}</td>
          <td class="num">${delBtn("del-withdrawal", { id: w.id, account, label: `${fmt(w.amount_cents)} on ${w.date}` })}</td>
        </tr>`)
        .join("");
}

// ---- TFSA -----------------------------------------------------------------

async function refreshTfsa() {
  const settings = await invoke("get_tfsa_settings", { personId });
  const configured = settings.start_year !== null;
  document.querySelector("#tfsa-setup").hidden = configured;
  document.querySelector("#tfsa-body").hidden = !configured;
  if (!configured) return;

  const s = await invoke("get_tfsa_summary", { personId, currentYear });

  const roomEl = document.querySelector("#tfsa-room");
  roomEl.textContent = fmt(s.current_room);
  roomEl.classList.toggle("warn", s.current_room < 0);
  document.querySelector("#tfsa-total").textContent = fmt(s.total_contributed);
  document.querySelector("#tfsa-withdrawn").textContent = fmt(s.total_withdrawn);
  const overCard = document.querySelector("#tfsa-over-card");
  overCard.hidden = s.current_over_contribution <= 0;
  if (s.current_over_contribution > 0) {
    document.querySelector("#tfsa-over").textContent = fmt(s.current_over_contribution);
  }

  document.querySelector("#tfsa-year-body").innerHTML = s.years
    .map((y) => `<tr>
      <td>${y.year}${y.dollar_limit_missing ? ' <span class="flag" title="Limit estimated (beyond shipped data).">est.</span>' : ""}</td>
      <td class="num">${fmt(y.opening_room)}</td>
      <td class="num">${fmt(y.new_room)}</td>
      <td class="num">${y.withdrawals_readded ? fmt(y.withdrawals_readded) : "—"}</td>
      <td class="num">${fmt(y.available_room)}</td>
      <td class="num">${fmt(y.contribution)}</td>
      <td class="num">${y.withdrawal ? fmt(y.withdrawal) : "—"}</td>
      <td class="${closingClass(y.closing_room)}">${fmt(y.closing_room)}</td>
    </tr>`)
    .join("");

  const msgs = [];
  if (s.current_over_contribution > 0) {
    const last = s.years[s.years.length - 1];
    msgs.push(
      `You're over your TFSA room by ${fmt(s.current_over_contribution)} (there's no buffer). ` +
      `Estimated penalty: <strong>${fmt(last.estimated_monthly_penalty)}/month</strong>.`
    );
  }
  document.querySelector("#tfsa-warnings").innerHTML = msgs
    .map((m) => `<div class="warning">${m}</div>`)
    .join("");

  await renderContribLog("TFSA", "#tfsa-contrib-body");
  await renderWithdrawLog("TFSA", "#tfsa-withdraw-body");

  document.querySelector("#tfsa-start-year").value = settings.start_year ?? "";
  document.querySelector("#tfsa-opening-room").value = (settings.opening_room / 100).toString();
}

// ---- FHSA -----------------------------------------------------------------

async function refreshFhsa() {
  const settings = await invoke("get_fhsa_settings", { personId });
  const configured = settings.open_year !== null;
  document.querySelector("#fhsa-setup").hidden = configured;
  document.querySelector("#fhsa-body").hidden = !configured;
  if (!configured) return;

  const s = await invoke("get_fhsa_summary", { personId, currentYear });

  const roomEl = document.querySelector("#fhsa-room");
  roomEl.textContent = fmt(s.current_room);
  roomEl.classList.toggle("warn", s.current_room < 0);
  document.querySelector("#fhsa-lifetime").textContent = fmt(s.lifetime_remaining);
  document.querySelector("#fhsa-total").textContent = fmt(s.total_contributed);
  const overCard = document.querySelector("#fhsa-over-card");
  overCard.hidden = s.current_over_contribution <= 0;
  if (s.current_over_contribution > 0) {
    document.querySelector("#fhsa-over").textContent = fmt(s.current_over_contribution);
  }

  document.querySelector("#fhsa-year-body").innerHTML = s.years
    .map((y) => `<tr>
      <td>${y.year}${y.past_participation_window ? ' <span class="flag err" title="Past the ~15-year FHSA window.">past window</span>' : ""}</td>
      <td class="num">${fmt(y.carryforward_in)}</td>
      <td class="num">${fmt(y.new_room)}</td>
      <td class="num">${fmt(y.available_room)}</td>
      <td class="num">${fmt(y.contribution)}</td>
      <td class="${closingClass(y.closing_room)}">${fmt(y.closing_room)}</td>
      <td class="num">${fmt(y.lifetime_contributed)}</td>
    </tr>`)
    .join("");

  const msgs = [];
  if (s.current_over_contribution > 0) {
    const last = s.years[s.years.length - 1];
    msgs.push(
      `You're over your FHSA room by ${fmt(s.current_over_contribution)}. ` +
      `Estimated penalty: <strong>${fmt(last.estimated_monthly_penalty)}/month</strong>.`
    );
  }
  if (s.past_window) {
    msgs.push(`This FHSA is past its ~15-year participation window — it should be closed or transferred to an RRSP/RRIF.`);
  }
  document.querySelector("#fhsa-warnings").innerHTML = msgs
    .map((m) => `<div class="warning">${m}</div>`)
    .join("");

  await renderContribLog("FHSA", "#fhsa-contrib-body");
  await renderWithdrawLog("FHSA", "#fhsa-withdraw-body");

  document.querySelector("#fhsa-open-year").value = settings.open_year ?? "";
}

// ---- account dispatch & tabs ----------------------------------------------

function refreshAccount(account) {
  if (account === "RRSP") return refreshRrsp();
  if (account === "TFSA") return refreshTfsa();
  if (account === "FHSA") return refreshFhsa();
  if (account === "Backup") return refreshBackupSettings();
}

let activeAccount = "RRSP";

function switchTo(account) {
  activeAccount = account;
  document.querySelectorAll(".tab").forEach((t) =>
    t.classList.toggle("active", t.dataset.account === account)
  );
  document.querySelectorAll(".account-view").forEach((v) => {
    v.hidden = v.dataset.view !== account;
  });
  // Backup is global — the family-member selector doesn't apply there.
  document.querySelector("#person-bar").hidden = account === "Backup";
  refreshAccount(account);
}

// ---- backup ---------------------------------------------------------------

async function refreshBackupSettings() {
  const s = await invoke("get_backup_settings");
  document.querySelector("#backup-dir").textContent = `Data repo: ${s.dir}`;
  document.querySelector("#backup-remote").value = s.remote;
  document.querySelector("#backup-folder").value = s.folder;
  document.querySelector("#backup-status").textContent = s.enabled
    ? "Google Drive backup is configured."
    : "Google Drive not configured — local git history only.";
}

// ---- wiring ----------------------------------------------------------------

function onSubmit(id, handler) {
  const form = document.querySelector(id);
  if (form) form.addEventListener("submit", async (e) => {
    e.preventDefault();
    await handler(e.target);
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  // Tabs
  document.querySelectorAll(".tab").forEach((t) =>
    t.addEventListener("click", () => switchTo(t.dataset.account))
  );

  // Person selector
  document.querySelector("#person-select").addEventListener("change", (e) => {
    personId = Number(e.target.value);
    refreshAccount(activeAccount);
  });
  document.querySelector("#person-add").addEventListener("click", async () => {
    const name = await promptDialog("Name of the family member to add:");
    if (!name) return;
    personId = await invoke("add_person", { name });
    await refreshPersons();
    await refreshAccount(activeAccount);
  });
  document.querySelector("#person-rename").addEventListener("click", async () => {
    const sel = document.querySelector("#person-select");
    const current = sel.options[sel.selectedIndex]?.text || "";
    const name = await promptDialog("Rename this family member:", current);
    if (!name) return;
    await invoke("rename_person", { id: personId, name });
    await refreshPersons();
  });
  document.querySelector("#person-delete").addEventListener("click", async () => {
    const sel = document.querySelector("#person-select");
    const current = sel.options[sel.selectedIndex]?.text || "this person";
    if (await confirmDialog(`Delete ${current} and ALL of their RRSP/TFSA/FHSA data? This cannot be undone.`)) {
      await invoke("delete_person", { id: personId });
      personId = null;
      await refreshPersons();
      await refreshAccount(activeAccount);
    }
  });

  // --- RRSP forms ---
  onSubmit("#rrsp-income-form", async (f) => {
    await invoke("upsert_annual_income", {
      personId,
      year: Number(f.querySelector("#rrsp-income-year").value),
      earnedIncomeCents: toCents(f.querySelector("#rrsp-income-amount").value),
      pensionAdjustmentCents: toCents(f.querySelector("#rrsp-income-pa").value || 0),
    });
    f.reset();
    await refreshRrsp();
  });
  onSubmit("#rrsp-contrib-form", async (f) => {
    await invoke("add_contribution", {
      personId,
      account: "RRSP",
      taxYear: Number(f.querySelector("#rrsp-contrib-year").value),
      date: f.querySelector("#rrsp-contrib-date").value,
      amountCents: toCents(f.querySelector("#rrsp-contrib-amount").value),
      note: f.querySelector("#rrsp-contrib-note").value || "",
    });
    f.reset();
    await refreshRrsp();
  });
  onSubmit("#rrsp-opening-form", async (f) => {
    await invoke("set_rrsp_opening_room", { personId, cents: toCents(f.querySelector("#rrsp-opening-room").value || 0) });
    await refreshRrsp();
  });
  onSubmit("#rrsp-limit-form", async (f) => {
    await invoke("set_rrsp_dollar_limit", {
      year: Number(f.querySelector("#rrsp-limit-year").value),
      amountCents: toCents(f.querySelector("#rrsp-limit-amount").value),
    });
    f.reset();
    await refreshRrsp();
  });

  // --- TFSA forms ---
  const saveTfsa = async (yearSel, openingSel) => {
    await invoke("set_tfsa_settings", {
      personId,
      startYear: Number(document.querySelector(yearSel).value),
      openingRoomCents: toCents(document.querySelector(openingSel)?.value || 0),
    });
    await refreshTfsa();
  };
  onSubmit("#tfsa-setup-form", async () => saveTfsa("#tfsa-setup-year", "#tfsa-opening-room"));
  onSubmit("#tfsa-settings-form", async () => saveTfsa("#tfsa-start-year", "#tfsa-opening-room"));
  onSubmit("#tfsa-contrib-form", async (f) => {
    await invoke("add_contribution", {
      personId,
      account: "TFSA",
      taxYear: Number(f.querySelector("#tfsa-contrib-year").value),
      date: f.querySelector("#tfsa-contrib-date").value,
      amountCents: toCents(f.querySelector("#tfsa-contrib-amount").value),
      note: f.querySelector("#tfsa-contrib-note").value || "",
    });
    f.reset();
    await refreshTfsa();
  });
  onSubmit("#tfsa-withdraw-form", async (f) => {
    await invoke("add_withdrawal", {
      personId,
      account: "TFSA",
      taxYear: Number(f.querySelector("#tfsa-withdraw-year").value),
      date: f.querySelector("#tfsa-withdraw-date").value,
      amountCents: toCents(f.querySelector("#tfsa-withdraw-amount").value),
      note: f.querySelector("#tfsa-withdraw-note").value || "",
    });
    f.reset();
    await refreshTfsa();
  });

  // --- FHSA forms ---
  const saveFhsa = async (sel) => {
    await invoke("set_fhsa_settings", { personId, openYear: Number(document.querySelector(sel).value) });
    await refreshFhsa();
  };
  onSubmit("#fhsa-setup-form", async () => saveFhsa("#fhsa-setup-year"));
  onSubmit("#fhsa-settings-form", async () => saveFhsa("#fhsa-open-year"));
  onSubmit("#fhsa-contrib-form", async (f) => {
    await invoke("add_contribution", {
      personId,
      account: "FHSA",
      taxYear: Number(f.querySelector("#fhsa-contrib-year").value),
      date: f.querySelector("#fhsa-contrib-date").value,
      amountCents: toCents(f.querySelector("#fhsa-contrib-amount").value),
      note: f.querySelector("#fhsa-contrib-note").value || "",
    });
    f.reset();
    await refreshFhsa();
  });
  onSubmit("#fhsa-withdraw-form", async (f) => {
    await invoke("add_withdrawal", {
      personId,
      account: "FHSA",
      taxYear: Number(f.querySelector("#fhsa-withdraw-year").value),
      date: f.querySelector("#fhsa-withdraw-date").value,
      amountCents: toCents(f.querySelector("#fhsa-withdraw-amount").value),
      note: f.querySelector("#fhsa-withdraw-note").value || "",
    });
    f.reset();
    await refreshFhsa();
  });

  // --- Deletes (delegated, all require confirmation) ---
  document.querySelector("main").addEventListener("click", async (e) => {
    const btn = e.target.closest("button[data-action]");
    if (!btn) return;
    const { action, id, year, account, label } = btn.dataset;

    if (action === "del-income") {
      if (await confirmDialog(`Delete the ${year} income record? This changes your RRSP room.`)) {
        await invoke("delete_annual_income", { personId, year: Number(year) });
        await refreshRrsp();
      }
    } else if (action === "del-contrib") {
      if (await confirmDialog(`Delete this contribution (${label})?`)) {
        await invoke("delete_contribution", { id: Number(id) });
        await refreshAccount(account);
      }
    } else if (action === "del-withdrawal") {
      if (await confirmDialog(`Delete this withdrawal (${label})?`)) {
        await invoke("delete_withdrawal", { id: Number(id) });
        await refreshAccount(account);
      }
    }
  });

  // --- Backup ---
  onSubmit("#backup-form", async (f) => {
    await invoke("set_backup_settings", {
      remote: f.querySelector("#backup-remote").value || "",
      folder: f.querySelector("#backup-folder").value || "CRAcked",
    });
    await refreshBackupSettings();
  });
  document.querySelector("#backup-now-btn").addEventListener("click", async () => {
    const status = document.querySelector("#backup-status");
    status.textContent = "Backing up…";
    status.classList.remove("warn");
    try {
      const r = await invoke("backup_now");
      const parts = [r.committed ? "Committed new snapshot." : "No changes to commit."];
      if (r.rclone_attempted) {
        parts.push(r.rclone_ok ? "Pushed to Google Drive." : `Drive push failed: ${r.rclone_message}`);
      } else {
        parts.push(r.rclone_message);
      }
      status.textContent = parts.join(" ");
      status.classList.toggle("warn", r.rclone_attempted && !r.rclone_ok);
    } catch (err) {
      status.textContent = `Backup error: ${err}`;
      status.classList.add("warn");
    }
  });

  await refreshPersons();
  await refreshBackupSettings();
  switchTo("RRSP");
});
