const { invoke } = window.__TAURI__.core;

// The only account wired up so far. TFSA/FHSA tabs are disabled in the UI.
const ACCOUNT = "RRSP";

// ---- money helpers ---------------------------------------------------------

/** Dollars string/number -> integer cents (safe rounding). */
function toCents(dollars) {
  const n = Number(dollars);
  if (!Number.isFinite(n)) return 0;
  return Math.round(n * 100);
}

/** Integer cents -> "$1,234.56" (CAD). */
function fmt(cents) {
  return (cents / 100).toLocaleString("en-CA", {
    style: "currency",
    currency: "CAD",
  });
}

// ---- rendering -------------------------------------------------------------

async function refreshSummary() {
  const s = await invoke("get_rrsp_summary");

  document.querySelector("#current-room").textContent = fmt(s.current_room);
  document.querySelector("#current-room").classList.toggle("warn", s.current_room < 0);
  document.querySelector("#total-contributed").textContent = fmt(s.total_contributed);

  const overCard = document.querySelector("#over-card");
  if (s.current_over_contribution > 0) {
    overCard.hidden = false;
    document.querySelector("#over-amount").textContent = fmt(s.current_over_contribution);
  } else {
    overCard.hidden = true;
  }

  // Year table
  const body = document.querySelector("#year-body");
  if (s.years.length === 0) {
    body.innerHTML =
      '<tr><td colspan="6" class="empty">No data yet — add income and contributions below.</td></tr>';
  } else {
    body.innerHTML = s.years
      .map((y) => {
        const closingClass = y.closing_room < 0 ? "num warn" : "num";
        return `<tr>
          <td>${y.year}${y.dollar_limit_missing ? ' <span class="flag" title="No published dollar limit for this year — new room is estimated from income only.">est.</span>' : ""}</td>
          <td class="num">${fmt(y.opening_room)}</td>
          <td class="num">${fmt(y.new_room)}</td>
          <td class="num">${fmt(y.available_room)}</td>
          <td class="num">${fmt(y.contribution)}</td>
          <td class="${closingClass}">${fmt(y.closing_room)}</td>
        </tr>`;
      })
      .join("");
  }

  // Warnings
  const warnings = document.querySelector("#warnings");
  const msgs = [];
  if (s.current_over_contribution > 0) {
    const last = s.years[s.years.length - 1];
    msgs.push(
      `You are over your contribution room by ${fmt(s.current_over_contribution)} beyond the $2,000 buffer. Estimated penalty: <strong>${fmt(last.estimated_monthly_penalty)}/month</strong> (1% of the excess) until corrected.`
    );
  }
  if (s.years.some((y) => y.dollar_limit_missing)) {
    msgs.push(
      `Some years have no published RRSP dollar limit on record, so their new room is estimated from income alone. Verify those against the CRA.`
    );
  }
  warnings.innerHTML = msgs.map((m) => `<div class="warning">${m}</div>`).join("");
}

async function refreshIncome() {
  const list = await invoke("list_annual_income");
  const el = document.querySelector("#income-list");
  if (list.length === 0) {
    el.innerHTML = '<li class="muted">No income entered yet.</li>';
    return;
  }
  el.innerHTML = list
    .map((i) => {
      const pa =
        i.pension_adjustment_cents > 0
          ? ` <span class="muted">(PA ${fmt(i.pension_adjustment_cents)})</span>`
          : "";
      return `<li><strong>${i.year}</strong> — ${fmt(i.earned_income_cents)}${pa}</li>`;
    })
    .join("");
}

async function refreshContributions() {
  const list = await invoke("list_contributions", { account: ACCOUNT });
  const body = document.querySelector("#contrib-body");
  if (list.length === 0) {
    body.innerHTML = '<tr><td colspan="5" class="empty">No contributions logged yet.</td></tr>';
    return;
  }
  body.innerHTML = list
    .map(
      (c) => `<tr>
        <td>${c.tax_year}</td>
        <td>${c.date}</td>
        <td class="num">${fmt(c.amount_cents)}</td>
        <td>${escapeHtml(c.note)}</td>
        <td class="num"><button class="link-btn" data-id="${c.id}">delete</button></td>
      </tr>`
    )
    .join("");
}

function escapeHtml(s) {
  return s.replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])
  );
}

async function refreshBackupSettings() {
  const s = await invoke("get_backup_settings");
  document.querySelector("#backup-dir").textContent = `Data repo: ${s.dir}`;
  document.querySelector("#backup-remote").value = s.remote;
  document.querySelector("#backup-folder").value = s.folder;
  const status = document.querySelector("#backup-status");
  status.textContent = s.enabled
    ? "Google Drive backup is configured."
    : "Google Drive not configured — local git history only.";
}

async function refreshAll() {
  await Promise.all([refreshSummary(), refreshIncome(), refreshContributions()]);
}

// ---- wiring ----------------------------------------------------------------

window.addEventListener("DOMContentLoaded", async () => {
  // Save annual income
  document.querySelector("#income-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("upsert_annual_income", {
      year: Number(document.querySelector("#income-year").value),
      earnedIncomeCents: toCents(document.querySelector("#income-amount").value),
      pensionAdjustmentCents: toCents(document.querySelector("#income-pa").value || 0),
    });
    e.target.reset();
    await refreshAll();
  });

  // Add contribution
  document.querySelector("#contrib-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("add_contribution", {
      account: ACCOUNT,
      taxYear: Number(document.querySelector("#contrib-year").value),
      date: document.querySelector("#contrib-date").value,
      amountCents: toCents(document.querySelector("#contrib-amount").value),
      note: document.querySelector("#contrib-note").value || "",
    });
    e.target.reset();
    await refreshAll();
  });

  // Delete contribution (event delegation)
  document.querySelector("#contrib-body").addEventListener("click", async (e) => {
    const btn = e.target.closest("button[data-id]");
    if (!btn) return;
    await invoke("delete_contribution", { id: Number(btn.dataset.id) });
    await refreshAll();
  });

  // Save opening room
  document.querySelector("#opening-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_rrsp_opening_room", {
      cents: toCents(document.querySelector("#opening-room").value || 0),
    });
    await refreshAll();
  });

  // Save backup settings
  document.querySelector("#backup-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    await invoke("set_backup_settings", {
      remote: document.querySelector("#backup-remote").value || "",
      folder: document.querySelector("#backup-folder").value || "CRAcked",
    });
    await refreshBackupSettings();
  });

  // Manual "Back up now"
  document.querySelector("#backup-now-btn").addEventListener("click", async () => {
    const status = document.querySelector("#backup-status");
    status.textContent = "Backing up…";
    try {
      const r = await invoke("backup_now");
      const parts = [];
      parts.push(r.committed ? "Committed new snapshot." : "No changes to commit.");
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

  // Load current opening room into the field
  const opening = await invoke("get_rrsp_opening_room");
  document.querySelector("#opening-room").value = (opening / 100).toString();

  await refreshBackupSettings();
  await refreshAll();
});
